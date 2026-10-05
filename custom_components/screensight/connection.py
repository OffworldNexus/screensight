"""Reconnecting WebSocket connection to a paired Screensight device.

The connection is deliberately a small state machine rather than a
:class:`~homeassistant.helpers.update_coordinator.DataUpdateCoordinator`: the
device pushes state on its own schedule, so we keep a persistent socket, mirror
the latest ``state`` frame, and notify entities whenever anything changes.
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging
from typing import TYPE_CHECKING, Any

from aiohttp import WSMsgType
from homeassistant.components import zeroconf as zeroconf_component
from homeassistant.core import callback
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import aiohttp_client
from homeassistant.helpers.dispatcher import async_dispatcher_send
from zeroconf import ServiceStateChange
from zeroconf.asyncio import AsyncServiceBrowser

from .const import (
    CONF_HOST,
    HEARTBEAT_INTERVAL,
    MAX_MISSED_HEARTBEATS,
    RECONNECT_MAX,
    RECONNECT_MIN,
    SERVICE_TYPE,
    SIGNAL_STATE_UPDATED,
    TYPE_ERROR,
    TYPE_PING,
    TYPE_PONG,
    TYPE_SET_STATE,
    TYPE_SET_VALUE,
    TYPE_STATE,
    WS_PATH,
)

if TYPE_CHECKING:
    from collections.abc import Callable

    from homeassistant.config_entries import ConfigEntry
    from homeassistant.core import CALLBACK_TYPE, HomeAssistant

_LOGGER = logging.getLogger(__name__)

# Presence of this constant documents the "drop after N missed heartbeats"
# behaviour for readers of the module.
_LOST_CONNECTION = "Screensight connection lost"
_Message = dict[str, Any]


class ScreensightConnectionError(HomeAssistantError):
    """Raised internally when the device connection can no longer be trusted."""


class ScreensightConnection:
    """A persistent, self-healing WebSocket client for one device."""

    def __init__(
        self,
        hass: HomeAssistant,
        entry: ConfigEntry,
        *,
        host: str,
        port: int,
        token: str,
        service_name: str | None = None,
    ) -> None:
        """Prepare the connection manager without opening a socket yet."""
        self.hass = hass
        self._entry = entry
        self._host = host
        self._port = port
        self._token = token
        self._service_name = service_name

        self._ws: Any | None = None
        self._task: asyncio.Task[None] | None = None
        self._stopping = False
        self._connected = False
        self._failures = 0
        self._values: dict[str, str] = {}
        self._desired: dict[str, str] = {}
        self._selected = False
        self._listeners: set[Callable[[], None]] = set()
        self._write_lock = asyncio.Lock()
        # Signalled when mDNS says our device is back; wakes the retry sleep.
        self._wake = asyncio.Event()
        self._browser: AsyncServiceBrowser | None = None
        # Injectable so tests can drive the reconnect loop without sleeping.
        self._sleep = self._async_sleep_with_wake

    # -- public API ---------------------------------------------------------

    @property
    def values(self) -> dict[str, str]:
        """Return a copy of the device's dashboard values."""
        return dict(self._values)

    def value(self, key: str) -> str | None:
        """Return one dashboard value, if the device has set it."""
        return self._values.get(key)

    @property
    def selected(self) -> bool:
        """Return whether this instance currently drives the panel."""
        return self._selected

    @property
    def available(self) -> bool:
        """Return whether the socket is currently connected."""
        return self._connected

    @property
    def host(self) -> str:
        """Return the host we are currently targeting."""
        return self._host

    @callback
    def async_add_listener(self, update_callback: Callable[[], None]) -> CALLBACK_TYPE:
        """Register a listener called on every state change; returns its remover."""
        self._listeners.add(update_callback)

        @callback
        def remove_listener() -> None:
            self._listeners.discard(update_callback)

        return remove_listener

    async def async_start(self) -> None:
        """Start the background connection loop (idempotent)."""
        if self._task is not None:
            return
        self._stopping = False
        self._start_browser()
        self._task = self.hass.async_create_background_task(
            self._run(), "screensight-websocket"
        )

    async def async_stop(self) -> None:
        """Stop the loop and close the socket."""
        self._stopping = True
        task, self._task = self._task, None
        if task is not None:
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task
        browser, self._browser = self._browser, None
        if browser is not None:
            with contextlib.suppress(Exception):
                await browser.async_cancel()
        await self._close_ws()
        self._set_connected(False)

    async def async_set_value(self, key: str, value: str) -> None:
        """Send ``set_value`` and optimistically mirror it locally."""
        async with self._write_lock:
            ws = self._ws
            if not self._connected or ws is None:
                msg = "Screensight device is not connected; cannot set value"
                raise HomeAssistantError(msg)
            await ws.send_json({"type": TYPE_SET_VALUE, "key": key, "value": value})
            self._desired[key] = value
            self._values[key] = value
            self._notify()

    # -- connection loop ----------------------------------------------------

    async def _run(self) -> None:
        """Connect, listen, and reconnect with exponential backoff."""
        while not self._stopping:
            try:
                await self._connect_and_listen()
            except asyncio.CancelledError:
                raise
            except Exception:
                self._failures += 1
                _LOGGER.debug(
                    "Screensight connection to %s failed (attempt %s)",
                    self._host,
                    self._failures,
                    exc_info=True,
                )
            self._set_connected(False)
            if self._stopping:
                break
            await self._sleep(self._backoff_delay())

    def _backoff_delay(self) -> float:
        """Return the next backoff delay, doubling from 1s up to 60s."""
        exponent = max(self._failures - 1, 0)
        return float(min(RECONNECT_MIN * (2**exponent), RECONNECT_MAX))

    async def _connect_and_listen(self) -> None:
        """Open the socket and pump frames until it drops."""
        host = await self._async_resolve_host()
        session = aiohttp_client.async_get_clientsession(self.hass)
        url = f"ws://{host}:{self._port}{WS_PATH}"
        headers = {"Authorization": f"Bearer {self._token}"}
        try:
            async with session.ws_connect(url, headers=headers, heartbeat=None) as ws:
                self._ws = ws
                self._failures = 0
                self._set_connected(True)
                # Re-send the full state we hold so a device that rebooted or
                # missed frames converges on Home Assistant's values.
                if self._desired:
                    await ws.send_json(
                        {"type": TYPE_SET_STATE, "values": dict(self._desired)}
                    )
                await self._async_read_loop(ws)
        finally:
            self._ws = None

    async def _async_read_loop(self, ws: Any) -> None:
        """Read frames, sending an application-level ping on idle."""
        missed = 0
        while True:
            try:
                message = await asyncio.wait_for(
                    ws.receive(), timeout=HEARTBEAT_INTERVAL
                )
            except TimeoutError:
                missed += 1
                if missed >= MAX_MISSED_HEARTBEATS:
                    raise ScreensightConnectionError(_LOST_CONNECTION) from None
                await ws.send_json({"type": TYPE_PING})
                continue

            if message.type is WSMsgType.TEXT:
                missed = 0
                await self._async_handle_message(json.loads(message.data))
            elif message.type in (
                WSMsgType.CLOSE,
                WSMsgType.CLOSING,
                WSMsgType.CLOSED,
                WSMsgType.ERROR,
            ):
                raise ScreensightConnectionError(_LOST_CONNECTION)

    async def _async_handle_message(self, message: _Message) -> None:
        """Apply one decoded device frame to the cached state."""
        message_type = message.get("type")
        if message_type == TYPE_STATE:
            self._values = dict(message.get("values", {}))
            self._selected = bool(message.get("selected", False))
            self._notify()
        elif message_type == TYPE_PONG:
            return
        elif message_type == TYPE_ERROR:
            _LOGGER.warning(
                "Screensight device reported an error: %s", message.get("message")
            )
        else:
            _LOGGER.debug("Ignoring unknown Screensight frame: %s", message_type)

    async def _async_resolve_host(self) -> str:
        """Re-resolve the stored mDNS service name, falling back to the host."""
        if not self._service_name:
            return self._host
        try:
            zeroconf = await zeroconf_component.async_get_async_instance(self.hass)
            info = await zeroconf.async_get_service_info(
                SERVICE_TYPE, self._service_name
            )
        except Exception:
            _LOGGER.debug("Could not re-resolve %s", self._service_name, exc_info=True)
            return self._host

        if info is None:
            return self._host
        addresses = info.parsed_addresses()
        host = next((addr for addr in addresses if ":" not in addr), None)
        if host is None and addresses:
            host = addresses[0]
        if host is None or host == self._host:
            return self._host
        _LOGGER.info("Screensight device moved from %s to %s", self._host, host)
        self._host = host
        self._update_entry_host(host)
        return host

    # -- mDNS-driven reconnect ----------------------------------------------

    @callback
    def _start_browser(self) -> None:
        """Browse our service so the device's return wakes the retry sleep."""
        if "zeroconf" not in self.hass.config.components:
            return
        try:
            azc = zeroconf_component.async_get_async_zeroconf(self.hass)
        except Exception:
            _LOGGER.debug("Screensight: zeroconf unavailable", exc_info=True)
            return
        _LOGGER.debug("Screensight: watching %s for reconnects", SERVICE_TYPE)
        self._browser = AsyncServiceBrowser(
            azc.zeroconf, SERVICE_TYPE, handlers=[self._async_on_service]
        )

    @callback
    def _async_on_service(
        self,
        zeroconf: Any,
        service_type: str,
        name: str,
        state_change: ServiceStateChange,
    ) -> None:
        """Wake the reconnect loop when our device announces itself again."""
        if state_change not in (ServiceStateChange.Added, ServiceStateChange.Updated):
            return
        if self._service_name is not None and name != self._service_name:
            return
        _LOGGER.debug("Screensight: %s announced; waking reconnect", name)
        self._wake.set()

    async def _async_sleep_with_wake(self, delay: float) -> None:
        """Sleep for ``delay`` seconds, waking early on an mDNS announcement."""
        self._wake.clear()
        try:
            async with asyncio.timeout(delay):
                await self._wake.wait()
        except TimeoutError:
            pass

    # -- helpers ------------------------------------------------------------

    @callback
    def _set_connected(self, connected: bool) -> None:
        """Update the connected flag and notify on transitions."""
        if self._connected == connected:
            return
        self._connected = connected
        self._notify()

    @callback
    def _notify(self) -> None:
        """Fan out a state change to listeners and the dispatcher."""
        async_dispatcher_send(
            self.hass, SIGNAL_STATE_UPDATED.format(self._entry.entry_id)
        )
        for update_callback in list(self._listeners):
            update_callback()

    @callback
    def _update_entry_host(self, host: str) -> None:
        """Persist a newly discovered host back into the config entry."""
        if self._entry.data.get(CONF_HOST) == host:
            return
        self.hass.config_entries.async_update_entry(
            self._entry, data={**self._entry.data, CONF_HOST: host}
        )

    async def _close_ws(self) -> None:
        """Close the current socket, ignoring errors during teardown."""
        ws, self._ws = self._ws, None
        if ws is None:
            return
        with contextlib.suppress(Exception):
            await ws.close()
