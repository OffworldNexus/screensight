"""Diagnostics support for the Screensight integration."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from homeassistant.components.diagnostics import async_redact_data

from .const import CONF_HA_PRIVATE_KEY, DOMAIN

if TYPE_CHECKING:
    from homeassistant.config_entries import ConfigEntry
    from homeassistant.core import HomeAssistant

TO_REDACT = {CONF_HA_PRIVATE_KEY}


async def async_get_config_entry_diagnostics(
    hass: HomeAssistant, entry: ConfigEntry
) -> dict[str, Any]:
    """Return redacted diagnostics for a Screensight config entry."""
    connection = hass.data.get(DOMAIN, {}).get(entry.entry_id)
    return {
        "entry": async_redact_data(dict(entry.data), TO_REDACT),
        "options": async_redact_data(dict(entry.options), TO_REDACT),
        "connection": {
            "available": bool(connection.available) if connection else False,
            "values": connection.values if connection else {},
            "selected": connection.selected if connection else False,
            "host": connection.host if connection else None,
        },
    }
