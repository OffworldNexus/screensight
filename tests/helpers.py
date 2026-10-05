"""In-memory fakes for the Screensight WebSocket protocol.

The Home Assistant test harness blocks real sockets, so every test drives the
integration through these fakes instead of a live device.
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
TOKEN = "a" * 64
SERVICE_NAME = "screensight-ab12cd34._screensight._tcp.local."


def make_discovery(**overrides: Any) -> ZeroconfServiceInfo:
    """Build a zeroconf discovery payload shaped like the real device's."""
    properties: dict[str, Any] = {
        "id": DEVICE_ID,
        "pairing": "1",
        "model": "Screensight Studio",
        "version": "0.1.0",
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


def pair_frames() -> list[dict[str, Any]]:
    """Return the two frames the device sends for a successful pairing."""
    return [
        {
            "type": "pair_pending",
            "device_id": DEVICE_ID,
            "name": "screensight-ab12cd34",
            "ha_name": "Home Assistant",
        },
        {
            "type": "pair_success",
            "token": TOKEN,
            "device_id": DEVICE_ID,
            "name": "screensight-ab12cd34",
            "ha_id": "ha-uuid",
        },
    ]


class FakeMessage:
    """A minimal stand-in for :class:`aiohttp.WSMessage`."""

    def __init__(self, type_: WSMsgType, data: str | None = None) -> None:
        """Store the message type and payload."""
        self.type = type_
        self.data = data


class FakeWebSocket:
    """A scripted WebSocket: replies are popped from ``incoming``."""

    def __init__(self, incoming: list[Any] | None = None) -> None:
        """Seed the socket with JSON frames or exceptions to raise."""
        self.incoming: list[Any] = list(incoming or [])
        self.sent: list[dict[str, Any]] = []
        self.closed = False

    async def send_json(self, data: dict[str, Any]) -> None:
        """Record a frame sent by the client."""
        self.sent.append(data)

    async def receive(self) -> FakeMessage:
        """Return the next scripted frame, or block like an idle socket."""
        while self.incoming:
            item = self.incoming.pop(0)
            if isinstance(item, BaseException):
                raise item
            if isinstance(item, FakeMessage):
                return item
            return FakeMessage(WSMsgType.TEXT, json.dumps(item))
        await asyncio.sleep(3600)  # idle: only wait_for/timeouts wake us
        msg = "idle socket should have been cancelled"
        raise AssertionError(msg)

    async def receive_json(self) -> dict[str, Any]:
        """Return the next scripted JSON frame."""
        message = await self.receive()
        return json.loads(message.data or "{}")

    async def close(self) -> None:
        """Mark the socket as closed."""
        self.closed = True


class FakeWSContext:
    """Async context manager returned by :meth:`FakeClientSession.ws_connect`."""

    def __init__(self, ws: FakeWebSocket) -> None:
        """Wrap a fake socket."""
        self._ws = ws

    async def __aenter__(self) -> FakeWebSocket:
        """Return the fake socket."""
        return self._ws

    async def __aexit__(self, *exc_info: object) -> bool:
        """Close the fake socket on exit."""
        await self._ws.close()
        return False


class FakeClientSession:
    """Stand-in for ``aiohttp.ClientSession`` with a scripted ``ws_connect``."""

    def __init__(
        self, ws: FakeWebSocket | Callable[[], FakeWebSocket] | BaseException
    ) -> None:
        """Accept a fixed socket, a factory, or an exception to raise."""
        self._ws = ws
        self.connect_calls: list[dict[str, Any]] = []

    def ws_connect(self, url: str, **kwargs: Any) -> FakeWSContext:
        """Record the call and produce the next socket (or raise)."""
        self.connect_calls.append({"url": str(url), "kwargs": kwargs})
        if isinstance(self._ws, BaseException):
            raise self._ws
        ws = self._ws() if callable(self._ws) else self._ws
        return FakeWSContext(ws)
