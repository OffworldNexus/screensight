"""Noise transport for the Screensight device link.

The device is the Noise **responder**; Home Assistant is always the initiator.
Pairing uses ``Noise_XX`` (no prior keys) and steady state uses ``Noise_IK``
(Home Assistant remembers the device's static key). Every Noise message travels
as exactly one WebSocket **binary** frame.

The 8-digit pairing SAS is derived from the final Noise handshake hash, which
binds both legs' ephemeral keys: an active MITM that terminates and re-originates
the handshake produces a different SAS on each leg, so the value the user reads
off the panel will not match what Home Assistant derived.
"""

from __future__ import annotations

import hashlib
from typing import Any, Final

from aiohttp import WSMsgType
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import x25519
from noise.connection import Keypair, NoiseConnection

#: Noise protocol names, kept byte-identical to the Rust device.
XX_PATTERN: Final = "Noise_XX_25519_ChaChaPoly_SHA256"
IK_PATTERN: Final = "Noise_IK_25519_ChaChaPoly_SHA256"

#: WebSocket subprotocol tokens that select the Noise pattern.
SUBPROTOCOL_XX: Final = "screensight.noise.xx"
SUBPROTOCOL_IK: Final = "screensight.noise.ik"

#: Domain separation mixed into the handshake hash before digit extraction.
SAS_DOMAIN: Final = b"screensight/pairing-sas/v1"
#: Number of decimal digits in a SAS.
SAS_DIGITS: Final = 8
#: SAS values are drawn uniformly from ``0..SAS_MODULUS``.
SAS_MODULUS: Final = 100_000_000


def generate_keypair() -> tuple[bytes, bytes]:
    """Generate an X25519 static keypair as ``(private_bytes, public_bytes)``."""
    private = x25519.X25519PrivateKey.generate()
    private_bytes = private.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    public_bytes = private.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    return private_bytes, public_bytes


def public_from_private(private_bytes: bytes) -> bytes:
    """Derive the raw X25519 public key for a private key."""
    public = x25519.X25519PrivateKey.from_private_bytes(private_bytes).public_key()
    return public.public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )


async def receive_binary(ws: Any) -> bytes:
    """Await the next binary WebSocket frame, skipping control frames."""
    while True:
        message = await ws.receive()
        if message.type is WSMsgType.BINARY:
            return bytes(message.data)
        if message.type in (
            WSMsgType.CLOSE,
            WSMsgType.CLOSING,
            WSMsgType.CLOSED,
            WSMsgType.ERROR,
        ):
            msg = "websocket closed during the handshake"
            raise ConnectionError(msg)


def sas_from_handshake_hash(handshake_hash: bytes) -> str:
    """Derive the 8-digit pairing SAS from the final Noise handshake hash.

    ``SHA256(SAS_DOMAIN || handshake_hash)`` is mapped uniformly into
    ``0..SAS_MODULUS`` by rejection sampling over big-endian 4-byte windows,
    matching the Rust implementation exactly (see ``device/src/noise.rs``).
    """
    digest = hashlib.sha256(SAS_DOMAIN + handshake_hash).digest()
    limit = (0xFFFFFFFF // SAS_MODULUS) * SAS_MODULUS
    for offset in range(0, len(digest), 4):
        value = int.from_bytes(digest[offset : offset + 4], "big")
        if value < limit:
            return f"{value % SAS_MODULUS:0{SAS_DIGITS}d}"
    value = int.from_bytes(digest[:4], "big")
    return f"{value % SAS_MODULUS:0{SAS_DIGITS}d}"


class NoiseTransport:
    """One Noise session: the handshake followed by the transport ciphers."""

    def __init__(self, connection: NoiseConnection) -> None:
        """Wrap an initialised :class:`noise.connection.NoiseConnection`."""
        self._connection = connection
        self._peer_static: bytes | None = None

    @classmethod
    def xx_initiator(cls, private_bytes: bytes) -> NoiseTransport:
        """Start a pairing handshake as the initiator (no remote key needed)."""
        connection = NoiseConnection.from_name(XX_PATTERN.encode("ascii"))
        connection.set_as_initiator()
        connection.set_keypair_from_private_bytes(Keypair.STATIC, private_bytes)
        connection.start_handshake()
        return cls(connection)

    @classmethod
    def ik_initiator(
        cls, private_bytes: bytes, device_public_bytes: bytes
    ) -> NoiseTransport:
        """Start a remembered-key handshake against the device's static key."""
        connection = NoiseConnection.from_name(IK_PATTERN.encode("ascii"))
        connection.set_as_initiator()
        connection.set_keypair_from_private_bytes(Keypair.STATIC, private_bytes)
        connection.set_keypair_from_public_bytes(
            Keypair.REMOTE_STATIC, device_public_bytes
        )
        connection.start_handshake()
        return cls(connection)

    def write_handshake(self, payload: bytes = b"") -> bytes:
        """Produce the next handshake message."""
        message = bytes(self._connection.write_message(payload))
        self._capture_peer_static()
        return message

    def read_handshake(self, message: bytes) -> bytes:
        """Consume the peer's handshake message."""
        payload = bytes(self._connection.read_message(message))
        self._capture_peer_static()
        return payload

    def _capture_peer_static(self) -> None:
        """Remember the peer's static key while the handshake still exposes it.

        ``noiseprotocol`` deletes the handshake state when the pattern
        completes, so the key is copied out as soon as it becomes available.
        """
        if self._peer_static is not None:
            return
        protocol = self._connection.noise_protocol
        state = getattr(protocol, "handshake_state", None)
        remote = getattr(state, "rs", None) if state is not None else None
        if not hasattr(remote, "public_bytes"):
            remote = protocol.keypairs.get("rs")
        if remote is not None and hasattr(remote, "public_bytes"):
            self._peer_static = bytes(remote.public_bytes)

    @property
    def finished(self) -> bool:
        """Return whether the handshake has completed."""
        return bool(self._connection.handshake_finished)

    @property
    def sas(self) -> str:
        """Return the 8-digit SAS derived from the completed handshake."""
        return sas_from_handshake_hash(self._connection.get_handshake_hash())

    @property
    def peer_static(self) -> bytes | None:
        """Return the device's static public key, once the handshake reveals it."""
        return self._peer_static

    def encrypt(self, data: bytes) -> bytes:
        """Encrypt one application frame."""
        return bytes(self._connection.encrypt(data))

    def decrypt(self, data: bytes) -> bytes:
        """Decrypt one application frame."""
        return bytes(self._connection.decrypt(data))
