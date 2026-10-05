"""Config flow for the Screensight integration.

Pairing is a two-step dance:

1. The device advertises ``_screensight._tcp.local.`` with a ``pairing=1`` TXT
   key and shows a 6-digit code on its panel.
2. Home Assistant discovers it, asks the user for that code, opens a WebSocket
   to the device and sends ``pair``. The device answers ``pair_pending`` and
   waits for an on-panel confirmation; once the user taps "Yes" it answers
   ``pair_success`` with a long-lived bearer token.

Because the on-panel confirmation is asynchronous, the flow parks on a
progress step (with a bounded 120s wait) and completes when the device answers.
"""

from __future__ import annotations

import asyncio
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
    CONF_HA_ID,
    CONF_HA_NAME,
    CONF_HOST,
    CONF_MODEL,
    CONF_NAME,
    CONF_PORT,
    CONF_SERVICE_NAME,
    CONF_TOKEN,
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
    "invalid_code": "invalid_code",
    "rate_limited": "rate_limited",
    "window_closed": "window_closed",
    "wrong_device": "wrong_device",
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
        self._port: int = DEFAULT_PORT
        self._service_name: str | None = None
        self._model: str | None = None
        self._version: str | None = None
        self._code: str | None = None
        self._ha_name: str = DEFAULT_HA_NAME
        self._pair_task: asyncio.Task[dict[str, Any]] | None = None
        self._pair_outcome: dict[str, Any] | None = None

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
        # Show the device's own name in Home Assistant's discovery card.
        self.context["title_placeholders"] = {"name": self._display_name}
        return await self.async_step_pair()

    # -- repair flows -------------------------------------------------------

    async def async_step_reauth(self, entry_data: dict[str, Any]) -> ConfigFlowResult:
        """Re-pair an existing entry after the token was rejected."""
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
        return await self.async_step_pair()

    # -- pairing ------------------------------------------------------------

    async def async_step_pair(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Ask for the code shown on the device, then start pairing."""
        errors: dict[str, str] = {}
        if user_input is not None:
            code = user_input[CONF_CODE].strip()
            self._ha_name = (
                user_input.get(CONF_NAME) or DEFAULT_HA_NAME
            ).strip() or DEFAULT_HA_NAME
            if len(code) != 6 or not code.isdigit():
                errors[CONF_CODE] = "invalid_code_format"
            if not errors:
                self._code = code
                self._pair_task = self.hass.async_create_task(
                    self._async_pair(), "screensight-pair"
                )
                return self.async_show_progress(
                    step_id="pair_confirm",
                    progress_action="pair_confirm",
                    description_placeholders={"device_name": self._display_name},
                    progress_task=self._pair_task,
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
        task = self._pair_task
        if task is None or not task.done():
            return self.async_show_progress(
                step_id="pair_confirm",
                progress_action="pair_confirm",
                description_placeholders={"device_name": self._display_name},
                progress_task=task,
            )

        self._pair_outcome = task.result()
        outcome = self._pair_outcome["result"]
        if outcome == "success":
            return self.async_show_progress_done(next_step_id="pair_done")
        if outcome == "abort":
            return self.async_show_progress_done(next_step_id="pair_abort")
        return self.async_show_progress_done(next_step_id="pair_retry")

    async def async_step_pair_done(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Create or update the entry after a successful pairing."""
        outcome = self._pair_outcome or {}
        data: dict[str, Any] = outcome.get("data", {})
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
        self._pair_task = None
        error = (self._pair_outcome or {}).get("error", "unknown")
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
        reason = (self._pair_outcome or {}).get("reason", "pairing_declined")
        return self.async_abort(reason=reason)

    # -- websocket exchange -------------------------------------------------

    async def _async_pair(self) -> dict[str, Any]:
        """Run the pairing WebSocket exchange and normalise the outcome."""
        try:
            return await self._async_pair_once()
        except asyncio.CancelledError:
            raise
        except Exception:
            _LOGGER.debug("Unexpected pairing failure", exc_info=True)
            return {"result": "error", "error": "unknown"}

    async def _async_pair_once(self) -> dict[str, Any]:
        """Try each candidate address until the device answers."""
        session = aiohttp_client.async_get_clientsession(self.hass)
        ha_id = await instance_id.async_get(self.hass)
        frame = {
            "type": TYPE_PAIR,
            "device_id": self._device_id,
            "code": self._code,
            "ha_id": ha_id,
            "ha_name": self._ha_name,
        }
        last_error = "cannot_connect"
        for host in self._hosts:
            url = f"ws://{host}:{self._port}{WS_PATH}"
            try:
                async with async_timeout.timeout(PAIR_TIMEOUT):
                    async with session.ws_connect(url) as ws:
                        await ws.send_json(frame)
                        outcome = await self._async_read_pair_reply(ws, host)
                        if outcome is not None:
                            return outcome
            except TimeoutError:
                return {"result": "error", "error": "pairing_timeout"}
            except (aiohttp.ClientError, OSError, ValueError) as err:
                _LOGGER.debug("Pairing connection to %s failed: %s", host, err)
                last_error = "cannot_connect"
        return {"result": "error", "error": last_error}

    async def _async_read_pair_reply(self, ws: Any, host: str) -> dict[str, Any] | None:
        """Read frames until the device resolves the pairing (or drop)."""
        while True:
            message = await ws.receive_json()
            message_type = message.get("type")
            if message_type == TYPE_PAIR_PENDING:
                continue
            if message_type == TYPE_PAIR_SUCCESS:
                data = self._entry_data(message, host)
                if data is None:
                    return {"result": "error", "error": "cannot_connect"}
                return {"result": "success", "data": data}
            if message_type == TYPE_PAIR_REJECTED:
                reason = message.get("reason", "declined")
                abort_reason = (
                    "pairing_declined" if reason == "declined" else "pairing_timed_out"
                )
                return {"result": "abort", "reason": abort_reason}
            if message_type == TYPE_PAIR_ERROR:
                reason = message.get("reason", "unknown")
                return {"result": "error", "error": _PAIR_ERRORS.get(reason, "unknown")}
            if message_type == TYPE_ERROR:
                return {"result": "error", "error": "cannot_connect"}
            _LOGGER.debug("Ignoring unexpected pairing frame: %s", message_type)

    def _entry_data(self, message: dict[str, Any], host: str) -> dict[str, Any] | None:
        """Build the config entry data from a ``pair_success`` frame."""
        token = message.get("token")
        if not token:
            return None
        return {
            CONF_DEVICE_ID: message.get("device_id", self._device_id),
            CONF_NAME: message.get("name") or _clean_service_name(self._device_name),
            CONF_HOST: host,
            CONF_PORT: self._port,
            CONF_TOKEN: token,
            CONF_HA_ID: message.get(CONF_HA_ID),
            CONF_HA_NAME: self._ha_name,
            CONF_MODEL: self._model,
            CONF_VERSION: self._version,
            CONF_SERVICE_NAME: self._service_name,
        }
