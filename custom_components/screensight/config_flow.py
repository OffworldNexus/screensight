"""Config flow for the Screensight integration.

Pairing follows the Noise XX ordering required by OFF-220:

1. The device advertises ``_screensight._tcp.local.`` with ``pairing=1`` while
   its pairing window is open.
2. Home Assistant opens ``/ws`` negotiating the ``screensight.noise.xx``
   subprotocol and completes the Noise XX handshake. The handshake yields a
   8-digit SAS on both sides; the device shows it on the panel.
3. Only *after* the handshake does the flow ask the user for the code shown on
   the panel. The typed value is compared locally with Home Assistant's own SAS
   and is never transmitted.
4. On a match, Home Assistant names itself inside the now-authenticated channel,
   the user approves on the panel, and the flow stores the mutual static keys.

The live WebSocket and Noise session are held on the flow object across steps,
because a second XX handshake would derive a different SAS.
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging
from typing import TYPE_CHECKING, Any

import aiohttp
import async_timeout
import voluptuous as vol
from homeassistant.config_entries import (
    SOURCE_REAUTH,
    SOURCE_RECONFIGURE,
    ConfigEntry,
    ConfigFlow,
    ConfigFlowResult,
)
from homeassistant.helpers import aiohttp_client, instance_id, selector

from .const import (
    CONF_CODE,
    CONF_DEVICE_ID,
    CONF_DEVICE_STATIC_KEY,
    CONF_HA_ID,
    CONF_HA_NAME,
    CONF_HA_PRIVATE_KEY,
    CONF_HA_PUBLIC_KEY,
    CONF_HOST,
    CONF_MODEL,
    CONF_NAME,
    CONF_PORT,
    CONF_SERVICE_NAME,
    CONF_VERSION,
    DEFAULT_HA_NAME,
    DEFAULT_PORT,
    DOMAIN,
    PAIR_TIMEOUT,
    TYPE_ERROR,
    TYPE_PAIR,
    TYPE_PAIR_ERROR,
    TYPE_PAIR_PENDING,
    TYPE_PAIR_REJECTED,
    TYPE_PAIR_SUCCESS,
    WS_PATH,
)
from .noise_transport import (
    SAS_DIGITS,
    SUBPROTOCOL_XX,
    NoiseTransport,
    generate_keypair,
    public_from_private,
    receive_binary,
)

if TYPE_CHECKING:
    from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo

_LOGGER = logging.getLogger(__name__)

PAIR_SCHEMA = vol.Schema(
    {
        vol.Required(CONF_CODE): selector.TextSelector(
            selector.TextSelectorConfig(type=selector.TextSelectorType.TEXT)
        ),
        vol.Optional(CONF_NAME, default=DEFAULT_HA_NAME): selector.TextSelector(),
    }
)

# Device ``pair_error`` reasons map onto translated form errors.
_PAIR_ERRORS = {
    "rate_limited": "rate_limited",
    "window_closed": "window_closed",
}


def _clean_service_name(name: str | None) -> str:
    """Turn an mDNS instance name into something worth showing a human."""
    if not name:
        return "Screensight"
    suffix = "._screensight._tcp.local."
    if name.endswith(suffix):
        return name[: -len(suffix)]
    return name


class ScreensightConfigFlow(ConfigFlow, domain=DOMAIN):
    """Handle zeroconf discovery and the interactive pairing of a device."""

    VERSION = 1

    def __init__(self) -> None:
        """Initialise the per-flow pairing context."""
        self._device_id: str | None = None
        self._device_name: str | None = None
        self._hosts: list[str] = []
        self._host: str | None = None
        self._port: int = DEFAULT_PORT
        self._service_name: str | None = None
        self._model: str | None = None
        self._version: str | None = None
        self._device_static_key: str | None = None
        self._ha_private: bytes | None = None
        self._ha_public: bytes | None = None
        self._ha_id: str | None = None
        self._ha_name: str = DEFAULT_HA_NAME
        self._transport: NoiseTransport | None = None
        self._ws: Any | None = None
        self._sas: str | None = None
        self._connect_task: asyncio.Task[dict[str, Any]] | None = None
        self._confirm_task: asyncio.Task[dict[str, Any]] | None = None
        self._outcome: dict[str, Any] | None = None

    @property
    def _display_name(self) -> str:
        """Human-facing device name used in the flow strings."""
        return _clean_service_name(self._device_name or self._device_id)

    # -- discovery ----------------------------------------------------------

    async def async_step_zeroconf(
        self, discovery_info: ZeroconfServiceInfo
    ) -> ConfigFlowResult:
        """Handle a Screensight device advertising itself over mDNS."""
        properties = discovery_info.properties
        device_id = properties.get("id")
        if not device_id:
            return self.async_abort(reason="no_device_id")
        if properties.get("pairing") != "1":
            return self.async_abort(reason="not_pairing")
        self._device_id = str(device_id)
        await self.async_set_unique_id(self._device_id)
        self._abort_if_unique_id_configured()

        self._device_name = discovery_info.name
        self._service_name = discovery_info.name
        self._port = discovery_info.port or DEFAULT_PORT
        self._hosts = [str(ip) for ip in discovery_info.ip_addresses]
        if not self._hosts:
            self._hosts = [str(discovery_info.ip_address)]
        self._model = properties.get("model")
        self._version = properties.get("version")
        advertised = properties.get("key")
        if isinstance(advertised, str) and advertised:
            self._device_static_key = advertised
        # Show the device's own name in Home Assistant's discovery card.
        self.context["title_placeholders"] = {"name": self._display_name}
        return await self.async_step_connect()

    # -- repair flows -------------------------------------------------------

    async def async_step_reauth(self, entry_data: dict[str, Any]) -> ConfigFlowResult:
        """Re-pair an existing entry after its keys were rejected."""
        return await self._async_step_repair(self._get_reauth_entry())

    async def async_step_reconfigure(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Re-pair an existing entry from scratch."""
        return await self._async_step_repair(self._get_reconfigure_entry())

    async def _async_step_repair(self, entry: ConfigEntry) -> ConfigFlowResult:
        """Prime the pairing context from an existing entry."""
        self._device_id = entry.data[CONF_DEVICE_ID]
        await self.async_set_unique_id(self._device_id)
        self._device_name = entry.data.get(CONF_NAME)
        self._service_name = entry.data.get(CONF_SERVICE_NAME)
        self._hosts = [entry.data[CONF_HOST]]
        self._port = entry.data.get(CONF_PORT, DEFAULT_PORT)
        self._model = entry.data.get(CONF_MODEL)
        self._version = entry.data.get(CONF_VERSION)
        self._device_static_key = entry.data.get(CONF_DEVICE_STATIC_KEY)
        # Reuse Home Assistant's existing keypair so its identity is stable.
        self._ha_private = bytes.fromhex(entry.data[CONF_HA_PRIVATE_KEY])
        self._ha_public = public_from_private(self._ha_private)
        return await self.async_step_connect()

    # -- handshake (XX) -----------------------------------------------------

    async def async_step_connect(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Open the socket and run Noise XX before asking for anything."""
        if self._connect_task is None:
            self._connect_task = self.hass.async_create_task(
                self._async_connect(), "screensight-connect"
            )
        return self.async_show_progress(
            step_id="pair_connect",
            progress_action="pair_connect",
            description_placeholders={"device_name": self._display_name},
            progress_task=self._connect_task,
        )

    async def async_step_pair_connect(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Finish the handshake, then ask for the displayed code."""
        task = self._connect_task
        if task is None or not task.done():
            return self.async_show_progress(
                step_id="pair_connect",
                progress_action="pair_connect",
                description_placeholders={"device_name": self._display_name},
                progress_task=task,
            )
        self._outcome = task.result()
        if self._outcome.get("result") == "connected":
            return self.async_show_progress_done(next_step_id="pair")
        return self.async_show_progress_done(next_step_id="pair_failed")

    async def async_step_pair_failed(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Abort the flow when the handshake could not be completed."""
        await self._async_close_session()
        reason = (self._outcome or {}).get("error", "cannot_connect")
        return self.async_abort(reason=reason)

    # -- code (SAS) comparison ---------------------------------------------

    async def async_step_pair(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Ask for the code shown on the device and compare it locally."""
        errors: dict[str, str] = {}
        if user_input is not None:
            code = user_input[CONF_CODE].strip()
            self._ha_name = (
                user_input.get(CONF_NAME) or DEFAULT_HA_NAME
            ).strip() or DEFAULT_HA_NAME
            if len(code) != SAS_DIGITS or not code.isdigit():
                errors[CONF_CODE] = "invalid_code_format"
            elif self._sas is not None and code != self._sas:
                # Never transmitted: the mismatch is detected here.
                errors[CONF_CODE] = "invalid_code"
            if not errors:
                self._confirm_task = self.hass.async_create_task(
                    self._async_confirm(), "screensight-confirm"
                )
                return self.async_show_progress(
                    step_id="pair_confirm",
                    progress_action="pair_confirm",
                    description_placeholders={"device_name": self._display_name},
                    progress_task=self._confirm_task,
                )
        return self.async_show_form(
            step_id="pair",
            data_schema=self.add_suggested_values_to_schema(
                PAIR_SCHEMA, {CONF_NAME: DEFAULT_HA_NAME}
            ),
            errors=errors,
            description_placeholders={"device_name": self._display_name},
        )

    async def async_step_pair_confirm(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Park while the user confirms the pairing on the panel."""
        task = self._confirm_task
        if task is None or not task.done():
            return self.async_show_progress(
                step_id="pair_confirm",
                progress_action="pair_confirm",
                description_placeholders={"device_name": self._display_name},
                progress_task=task,
            )
        self._outcome = task.result()
        outcome = self._outcome.get("result")
        if outcome == "success":
            return self.async_show_progress_done(next_step_id="pair_done")
        if outcome == "abort":
            return self.async_show_progress_done(next_step_id="pair_abort")
        return self.async_show_progress_done(next_step_id="pair_retry")

    async def async_step_pair_done(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Create or update the entry after a successful pairing."""
        outcome = self._outcome or {}
        data: dict[str, Any] = outcome.get("data", {})
        await self._async_close_session()
        if not data:
            return self.async_abort(reason="unknown")
        if self.source == SOURCE_REAUTH:
            return self.async_update_reload_and_abort(
                self._get_reauth_entry(), data_updates=data
            )
        if self.source == SOURCE_RECONFIGURE:
            return self.async_update_reload_and_abort(
                self._get_reconfigure_entry(), data_updates=data
            )
        return self.async_create_entry(title=data[CONF_NAME], data=data)

    async def async_step_pair_retry(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Show the code form again after a recoverable pairing error."""
        self._confirm_task = None
        error = (self._outcome or {}).get("error", "unknown")
        return self.async_show_form(
            step_id="pair",
            data_schema=self.add_suggested_values_to_schema(
                PAIR_SCHEMA, {CONF_NAME: self._ha_name}
            ),
            errors={CONF_CODE: error},
            description_placeholders={"device_name": self._display_name},
        )

    async def async_step_pair_abort(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Abort the flow after the device rejected the pairing."""
        await self._async_close_session()
        reason = (self._outcome or {}).get("reason", "pairing_declined")
        return self.async_abort(reason=reason)

    # -- websocket exchange -------------------------------------------------

    async def _async_connect(self) -> dict[str, Any]:
        """Open the socket, run Noise XX, and keep the session for the flow."""
        try:
            return await self._async_connect_once()
        except asyncio.CancelledError:
            await self._async_close_session()
            raise
        except Exception:
            _LOGGER.debug("Unexpected pairing failure", exc_info=True)
            await self._async_close_session()
            return {"result": "error", "error": "unknown"}

    async def _async_connect_once(self) -> dict[str, Any]:
        """Try each candidate address until one completes the XX handshake."""
        if self._ha_private is None:
            self._ha_private, self._ha_public = generate_keypair()
        elif self._ha_public is None:
            self._ha_public = public_from_private(self._ha_private)
        self._ha_id = await instance_id.async_get(self.hass)

        session = aiohttp_client.async_get_clientsession(self.hass)
        last_error = "cannot_connect"
        for host in self._hosts:
            url = f"ws://{host}:{self._port}{WS_PATH}"
            transport = NoiseTransport.xx_initiator(self._ha_private)
            ws: Any = None
            try:
                async with async_timeout.timeout(PAIR_TIMEOUT):
                    ws = await session.ws_connect(
                        url, protocols=(SUBPROTOCOL_XX,), heartbeat=None
                    )
                    first = transport.write_handshake()
                    await ws.send_bytes(first)
                    response = await receive_binary(ws)
                    transport.read_handshake(response)
                    third = transport.write_handshake()
                    await ws.send_bytes(third)
                    if not transport.finished:
                        msg = "Noise XX handshake did not finish"
                        raise ValueError(msg)
                    self._ws = ws
                    self._transport = transport
                    self._host = host
                    self._sas = transport.sas
                    device_key = transport.peer_static
                    if device_key is None and self._device_static_key:
                        device_key = bytes.fromhex(self._device_static_key)
                    if device_key is not None:
                        self._device_static_key = device_key.hex()
                    return {"result": "connected"}
            except TimeoutError:
                await _close(ws)
                return {"result": "error", "error": "pairing_timeout"}
            except (aiohttp.ClientError, OSError, ValueError) as err:
                _LOGGER.debug("Pairing connection to %s failed: %s", host, err)
                await _close(ws)
                last_error = "cannot_connect"
        return {"result": "error", "error": last_error}

    async def _async_confirm(self) -> dict[str, Any]:
        """Send the pair request over Noise and read the device's answer."""
        transport = self._transport
        ws = self._ws
        if transport is None or ws is None:
            return {"result": "error", "error": "unknown"}
        frame = {
            "type": TYPE_PAIR,
            "ha_id": self._ha_id,
            "ha_name": self._ha_name,
        }
        try:
            await ws.send_bytes(transport.encrypt(json.dumps(frame).encode()))
            while True:
                plaintext = transport.decrypt(await receive_binary(ws))
                message = json.loads(plaintext)
                message_type = message.get("type")
                if message_type == TYPE_PAIR_PENDING:
                    continue
                if message_type == TYPE_PAIR_SUCCESS:
                    return {"result": "success", "data": self._entry_data(message)}
                if message_type == TYPE_PAIR_REJECTED:
                    reason = message.get("reason", "declined")
                    abort_reason = (
                        "pairing_declined"
                        if reason == "declined"
                        else "pairing_timed_out"
                    )
                    return {"result": "abort", "reason": abort_reason}
                if message_type == TYPE_PAIR_ERROR:
                    reason = message.get("reason", "unknown")
                    return {
                        "result": "error",
                        "error": _PAIR_ERRORS.get(reason, "unknown"),
                    }
                if message_type == TYPE_ERROR:
                    return {"result": "error", "error": "cannot_connect"}
                _LOGGER.debug("Ignoring unexpected pairing frame: %s", message_type)
        except asyncio.CancelledError:
            raise
        except Exception:
            _LOGGER.debug("Unexpected pairing failure", exc_info=True)
            return {"result": "error", "error": "unknown"}
        finally:
            await self._async_close_session()

    def _entry_data(self, message: dict[str, Any]) -> dict[str, Any]:
        """Build the config entry data from a ``pair_success`` frame."""
        return {
            CONF_DEVICE_ID: message.get("device_id", self._device_id),
            CONF_NAME: message.get("name") or _clean_service_name(self._device_name),
            CONF_HOST: self._host,
            CONF_PORT: self._port,
            CONF_DEVICE_STATIC_KEY: self._device_static_key,
            CONF_HA_PRIVATE_KEY: self._ha_private.hex() if self._ha_private else None,
            CONF_HA_PUBLIC_KEY: self._ha_public.hex() if self._ha_public else None,
            CONF_HA_ID: message.get(CONF_HA_ID, self._ha_id),
            CONF_HA_NAME: self._ha_name,
            CONF_MODEL: self._model,
            CONF_VERSION: self._version,
            CONF_SERVICE_NAME: self._service_name,
        }

    async def _async_close_session(self) -> None:
        """Close the live socket, if any, ignoring teardown errors."""
        ws, self._ws = self._ws, None
        self._transport = None
        await _close(ws)


async def _close(ws: Any) -> None:
    """Close a websocket if one is open."""
    if ws is None:
        return
    with contextlib.suppress(Exception):
        await ws.close()
