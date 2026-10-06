"""In-memory fakes for the Screensight Noise WebSocket protocol.

The Home Assistant test harness blocks real sockets, so every test drives the
integration through these fakes instead of a live device. The Noise transport
itself is faked with a tag-prefixed identity cipher; the real crypto lives in
``custom_components/screensight/noise.py`` and is exercised by ``test_noise.py``.
"""

from __future__ import annotations

import asyncio
import ipaddress
import json
from typing import TYPE_CHECKING, Any

from aiohttp import WSMsgType
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo

if TYPE_CHECKING:
    from collections.abc import Callable

DEVICE_ID = "ab12cd34deadbeef"
SERVICE_NAME = "screensight-ab12cd34._screensight._tcp.local."
#: The device's static Noise public key, as advertised in the mDNS ``key=`` TXT.
DEVICE_STATIC_KEY = "ab" * 32
#: Home Assistant's own static keypair, for pre-populated entries.
HA_PRIVATE_KEY = "cd" * 32
HA_PUBLIC_KEY = "ef" * 32
#: The SAS the fake device derives; the test user must type this value.
SAS = "93704101"

#: Prefix the fake transport puts in front of every plaintext frame.
TAG = b"screensight-fake:"


def enc(payload: dict[str, Any]) -> bytes:
    """Wrap a device->HA frame the way the fake transport's ``decrypt`` expects."""
    return TAG + json.dumps(payload).encode()


def dec(blob: bytes) -> dict[str, Any]:
    """Unwrap a HA->device frame the fake transport encrypted."""
    assert blob.startswith(TAG), blob
    return json.loads(blob[len(TAG) :])


def make_discovery(**overrides: Any) -> ZeroconfServiceInfo:
    """Build a zeroconf discovery payload shaped like the real device's."""
    properties: dict[str, Any] = {
        "id": DEVICE_ID,
        "pairing": "1",
        "noise": "1",
        "model": "Screensight Studio",
        "version": "0.1.0",
        "key": DEVICE_STATIC_KEY,
    }
    data: dict[str, Any] = {
        "ip_address": ipaddress.ip_address("192.168.1.50"),
        "ip_addresses": [ipaddress.ip_address("192.168.1.50")],
        "port": 8765,
        "hostname": "screensight.local.",
        "type": "_screensight._tcp.local.",
        "name": SERVICE_NAME,
        "properties": properties,
    }
    data.update(overrides)
    return ZeroconfServiceInfo(**data)


def pair_frames() -> list[bytes]:
    """Return the binary frames the fake device sends for a successful pairing."""
    return [
        enc(
            {
                "type": "pair_pending",
                "device_id": DEVICE_ID,
                "name": "screensight-ab12cd34",
                "ha_name": "Home Assistant",
            }
        ),
        enc(
            {
                "type": "pair_success",
                "device_id": DEVICE_ID,
                "name": "screensight-ab12cd34",
                "ha_id": "ha-uuid",
            }
        ),
    ]


class FakeMessage:
    """A minimal stand-in for :class:`aiohttp.WSMessage`."""

    def __init__(self, type_: WSMsgType, data: bytes | str | None = None) -> None:
        """Store the message type and payload."""
        self.type = type_
        self.data = data


class FakeNoiseTransport:
    """A tag-prefixed identity cipher standing in for a Noise session."""

    #: The SAS every fake handshake derives.
    sas_value = SAS
    #: The peer (device) static key every fake handshake reveals.
    peer_value: bytes = bytes.fromhex(DEVICE_STATIC_KEY)
    #: All instances created, so tests can reach the active transport.
    instances: list[FakeNoiseTransport] = []

    def __init__(self) -> None:
        """Start an unfinished fake handshake."""
        self.finished = False

    @classmethod
    def xx_initiator(cls, private_bytes: bytes) -> FakeNoiseTransport:
        """Stand in for a pairing initiator."""
        transport = cls()
        cls.instances.append(transport)
        return transport

    @classmethod
    def ik_initiator(
        cls, private_bytes: bytes, device_public_bytes: bytes
    ) -> FakeNoiseTransport:
        """Stand in for a steady-state initiator."""
        transport = cls()
        transport.finished = True
        cls.instances.append(transport)
        return transport

    def write_handshake(self, payload: bytes = b"") -> bytes:
        """Produce a handshake frame."""
        return b"handshake"

    def read_handshake(self, message: bytes) -> bytes:
        """Consume the peer's handshake frame and complete the handshake."""
        self.finished = True
        return b""

    @property
    def sas(self) -> str:
        """Return the pairing SAS."""
        return self.sas_value

    @property
    def peer_static(self) -> bytes | None:
        """Return the device static key revealed by the handshake."""
        return self.peer_value

    def encrypt(self, data: bytes) -> bytes:
        """Tag a frame as if it were encrypted."""
        return TAG + data

    def decrypt(self, data: bytes) -> bytes:
        """Untag an encrypted frame."""
        assert data.startswith(TAG), data
        return data[len(TAG) :]


class FakeWebSocket:
    """A scripted binary WebSocket: frames are popped from ``incoming``."""

    def __init__(self, incoming: list[Any] | None = None) -> None:
        """Seed the socket with binary frames or exceptions to raise."""
        self.incoming: list[Any] = list(incoming or [])
        self.sent: list[bytes] = []
        self.closed = False

    async def send_bytes(self, data: bytes) -> None:
        """Record an encrypted frame sent by the client."""
        self.sent.append(bytes(data))

    async def receive(self) -> FakeMessage:
        """Return the next scripted frame, or block like an idle socket."""
        while self.incoming:
            item = self.incoming.pop(0)
            if isinstance(item, BaseException):
                raise item
            if isinstance(item, FakeMessage):
                return item
            if isinstance(item, (bytes, bytearray)):
                return FakeMessage(WSMsgType.BINARY, bytes(item))
            return FakeMessage(WSMsgType.TEXT, json.dumps(item))
        await asyncio.sleep(3600)  # idle: only wait_for/timeouts wake us
        msg = "idle socket should have been cancelled"
        raise AssertionError(msg)

    async def close(self) -> None:
        """Mark the socket as closed."""
        self.closed = True


class FakeClientSession:
    """Stand-in for ``aiohttp.ClientSession`` with a scripted ``ws_connect``."""

    def __init__(
        self, ws: FakeWebSocket | Callable[[], FakeWebSocket] | BaseException
    ) -> None:
        """Accept a fixed socket, a factory, or an exception to raise."""
        self._ws = ws
        self.connect_calls: list[dict[str, Any]] = []

    async def ws_connect(self, url: str, **kwargs: Any) -> FakeWebSocket:
        """Record the call and produce the next socket (or raise)."""
        self.connect_calls.append({"url": str(url), "kwargs": kwargs})
        if isinstance(self._ws, BaseException):
            raise self._ws
        return self._ws() if callable(self._ws) else self._ws
