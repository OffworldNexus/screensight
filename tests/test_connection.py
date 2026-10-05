"""Tests for the Screensight WebSocket connection manager."""

from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock

import aiohttp
import pytest
from homeassistant.exceptions import HomeAssistantError
from pytest_homeassistant_custom_component.common import MockConfigEntry
from zeroconf import ServiceStateChange

from custom_components.screensight import connection as connection_module
from custom_components.screensight.connection import (
    ScreensightConnection,
    ScreensightConnectionError,
)
from custom_components.screensight.const import CONF_DEVICE_ID, CONF_HOST, DOMAIN
from tests.helpers import SERVICE_NAME, FakeClientSession, FakeWebSocket

_TOKEN = "tok"


def _connection(hass, *, service_name: str | None = None):
    """Build a connection backed by a mock config entry."""
    entry = MockConfigEntry(
        domain=DOMAIN,
        data={
            CONF_DEVICE_ID: "dev",
            "name": "screensight-dev",
            CONF_HOST: "192.168.1.50",
        },
    )
    connection = ScreensightConnection(
        hass,
        entry,
        host="192.168.1.50",
        port=8765,
        token=_TOKEN,
        service_name=service_name,
    )
    return connection, entry


def _patch_session(monkeypatch: pytest.MonkeyPatch, ws: object) -> None:
    """Route ``async_get_clientsession`` to a fake session."""
    monkeypatch.setattr(
        "homeassistant.helpers.aiohttp_client.async_get_clientsession",
        lambda *args, **kwargs: FakeClientSession(ws),
    )


async def test_backoff_delay_grows_and_caps(hass) -> None:
    """Backoff doubles from one second and saturates at a minute."""
    connection, _ = _connection(hass)
    connection._failures = 0
    assert connection._backoff_delay() == 1.0
    connection._failures = 1
    assert connection._backoff_delay() == 1.0
    connection._failures = 4
    assert connection._backoff_delay() == 8.0
    connection._failures = 100
    assert connection._backoff_delay() == 60.0


async def test_set_value_sends_frame(hass) -> None:
    """``async_set_value`` writes the exact protocol frame and caches it."""
    connection, _ = _connection(hass)
    ws = FakeWebSocket()
    connection._ws = ws
    connection._set_connected(True)

    await connection.async_set_value("text", "Hello 🌧")

    assert ws.sent == [{"type": "set_value", "key": "text", "value": "Hello 🌧"}]
    assert connection.value("text") == "Hello 🌧"
    assert connection.available is True


async def test_set_value_requires_connection(hass) -> None:
    """Setting a value while disconnected raises a HomeAssistantError."""
    connection, _ = _connection(hass)
    with pytest.raises(HomeAssistantError):
        await connection.async_set_value("text", "nope")


async def test_state_frame_updates_values(hass) -> None:
    """A ``state`` frame is mirrored and flags the panel selection."""
    connection, _ = _connection(hass)
    await connection._async_handle_message(
        {"type": "state", "values": {"text": "Hi"}, "selected": True}
    )
    assert connection.value("text") == "Hi"
    assert connection.values == {"text": "Hi"}
    assert connection.selected is True


async def test_state_frame_notifies_listeners(hass) -> None:
    """State changes reach registered listeners."""
    connection, _ = _connection(hass)
    calls: list[int] = []
    connection.async_add_listener(lambda: calls.append(1))
    await connection._async_handle_message({"type": "state", "values": {"text": "x"}})
    assert calls == [1]


async def test_connect_resends_desired_state(hass, monkeypatch) -> None:
    """On (re)connect the values Home Assistant holds are re-sent."""
    connection, _ = _connection(hass)
    connection._desired = {"text": "keep"}
    ws = FakeWebSocket([aiohttp.ClientConnectionError("down")])
    _patch_session(monkeypatch, ws)
    with pytest.raises(aiohttp.ClientConnectionError):
        await connection._connect_and_listen()
    assert {"type": "set_state", "values": {"text": "keep"}} in ws.sent


async def test_run_reconnects_with_growing_backoff(hass, monkeypatch) -> None:
    """A failing socket is retried with 1s, 2s, 4s, ... delays."""
    connection, _ = _connection(hass)
    monkeypatch.setattr(
        "homeassistant.helpers.aiohttp_client.async_get_clientsession",
        lambda *args, **kwargs: FakeClientSession(
            aiohttp.ClientConnectionError("down")
        ),
    )
    delays: list[float] = []

    async def _sleep(delay: float) -> None:
        delays.append(delay)
        if len(delays) == 3:
            connection._stopping = True

    connection._sleep = _sleep
    await connection._run()

    assert delays == [1.0, 2.0, 4.0]


async def test_read_loop_drops_after_missing_heartbeats(hass, monkeypatch) -> None:
    """Two missed heartbeats tear the socket down."""
    connection, _ = _connection(hass)
    monkeypatch.setattr(connection_module, "HEARTBEAT_INTERVAL", 0.01)
    ws = FakeWebSocket([])

    with pytest.raises(ScreensightConnectionError):
        await connection._async_read_loop(ws)

    assert {"type": "ping"} in ws.sent


async def test_resolve_host_updates_entry_from_zeroconf(hass, monkeypatch) -> None:
    """A changed mDNS address is resolved and persisted to the entry."""
    connection, entry = _connection(hass, service_name=SERVICE_NAME)
    entry.add_to_hass(hass)

    class _Info:
        def parsed_addresses(self) -> list[str]:
            return ["10.0.0.9"]

    class _Zeroconf:
        async def async_get_service_info(self, type_: str, name: str) -> _Info:
            return _Info()

    monkeypatch.setattr(
        "homeassistant.components.zeroconf.async_get_async_instance",
        AsyncMock(return_value=_Zeroconf()),
    )

    host = await connection._async_resolve_host()

    assert host == "10.0.0.9"
    assert connection.host == "10.0.0.9"
    assert entry.data[CONF_HOST] == "10.0.0.9"


async def test_start_and_stop_are_idempotent(hass, monkeypatch) -> None:
    """Starting twice keeps one task; stopping cancels it."""
    connection, _ = _connection(hass)
    _patch_session(monkeypatch, FakeWebSocket([]))
    await connection.async_start()
    await connection.async_start()
    assert connection._task is not None
    await connection.async_stop()
    assert connection._task is None
    assert connection.available is False


async def test_mdns_announcement_wakes_the_retry_sleep(hass) -> None:
    """Our device re-announcing ends the backoff sleep early."""
    connection, _ = _connection(hass, service_name=SERVICE_NAME)
    task = asyncio.ensure_future(connection._async_sleep_with_wake(30))
    await asyncio.sleep(0)
    connection._async_on_service(
        None, "_screensight._tcp.local.", SERVICE_NAME, ServiceStateChange.Added
    )
    # Returns long before the 30s timeout would have elapsed.
    await asyncio.wait_for(task, timeout=1)


async def test_other_services_do_not_wake_the_sleep(hass) -> None:
    """A different device's announcement is ignored."""
    connection, _ = _connection(hass, service_name=SERVICE_NAME)
    task = asyncio.ensure_future(connection._async_sleep_with_wake(0.05))
    connection._async_on_service(
        None,
        "_screensight._tcp.local.",
        "Other Device._screensight._tcp.local.",
        ServiceStateChange.Added,
    )
    await task  # completes only once its own short timeout elapses
    assert not connection._wake.is_set()
