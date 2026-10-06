"""Tests for the Noise transport helper and the pairing SAS derivation."""

from __future__ import annotations

from noise.connection import Keypair, NoiseConnection

from custom_components.screensight.noise import (
    IK_PATTERN,
    SAS_DIGITS,
    SAS_MODULUS,
    XX_PATTERN,
    NoiseTransport,
    generate_keypair,
    public_from_private,
    sas_from_handshake_hash,
)


def test_sas_known_answer_matches_the_rust_vector() -> None:
    """The SAS derivation is byte-identical to the Rust device's."""
    handshake_hash = bytes(range(32))
    assert sas_from_handshake_hash(handshake_hash) == "93704101"


def test_sas_is_eight_digits_within_modulus() -> None:
    """Every derivation yields a well-formed, in-range code."""
    for i in range(2000):
        sas = sas_from_handshake_hash(i.to_bytes(4, "big"))
        assert len(sas) == SAS_DIGITS
        assert int(sas) < SAS_MODULUS


def test_keypair_round_trips() -> None:
    """A generated private key derives back to its public key."""
    private_bytes, public_bytes = generate_keypair()
    assert len(private_bytes) == 32
    assert len(public_bytes) == 32
    assert public_from_private(private_bytes) == public_bytes


def _responder(
    pattern: str, private_bytes: bytes, remote_public: bytes | None = None
) -> NoiseConnection:
    """Build a Noise responder for the given pattern."""
    connection = NoiseConnection.from_name(pattern.encode("ascii"))
    connection.set_as_responder()
    connection.set_keypair_from_private_bytes(Keypair.STATIC, private_bytes)
    if remote_public is not None:
        connection.set_keypair_from_public_bytes(Keypair.REMOTE_STATIC, remote_public)
    connection.start_handshake()
    return connection


def test_xx_initiator_and_responder_agree_on_the_sas() -> None:
    """XX completes, reveals the device key, and both sides derive one SAS."""
    device_private, device_public = generate_keypair()
    ha_private, _ = generate_keypair()

    initiator = NoiseTransport.xx_initiator(ha_private)
    responder = _responder(XX_PATTERN, device_private)

    responder.read_message(initiator.write_handshake())
    initiator.read_handshake(bytes(responder.write_message()))
    responder.read_message(initiator.write_handshake())

    assert initiator.finished
    assert initiator.peer_static == device_public
    expected = sas_from_handshake_hash(responder.noise_protocol.handshake_hash)
    assert initiator.sas == expected

    ciphertext = initiator.encrypt(b"hello")
    assert bytes(responder.decrypt(ciphertext)) == b"hello"


def test_ik_initiator_reaches_transport_with_the_device_key() -> None:
    """IK authenticates the device and enters transport in one round trip."""
    device_private, device_public = generate_keypair()
    ha_private, _ = generate_keypair()

    initiator = NoiseTransport.ik_initiator(ha_private, device_public)
    responder = _responder(IK_PATTERN, device_private)

    responder.read_message(initiator.write_handshake())
    initiator.read_handshake(bytes(responder.write_message()))

    assert initiator.finished
    assert initiator.peer_static == device_public
    ciphertext = initiator.encrypt(b"ping")
    assert bytes(responder.decrypt(ciphertext)) == b"ping"
