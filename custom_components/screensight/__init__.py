"""The Screensight integration: pair a Pi display and drive its text."""

from __future__ import annotations

from typing import TYPE_CHECKING

from .connection import ScreensightConnection
from .const import (
    CONF_HOST,
    CONF_PORT,
    CONF_SERVICE_NAME,
    CONF_TOKEN,
    DOMAIN,
    PLATFORMS,
)

if TYPE_CHECKING:
    from homeassistant.config_entries import ConfigEntry
    from homeassistant.core import HomeAssistant


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:
    """Set up a Screensight device from a config entry."""
    connection = ScreensightConnection(
        hass,
        entry,
        host=entry.data[CONF_HOST],
        port=entry.data[CONF_PORT],
        token=entry.data[CONF_TOKEN],
        service_name=entry.data.get(CONF_SERVICE_NAME),
    )
    await connection.async_start()
    hass.data.setdefault(DOMAIN, {})[entry.entry_id] = connection
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    entry.async_on_unload(connection.async_stop)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:
    """Unload a Screensight config entry."""
    unloaded = await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
    if unloaded:
        connection: ScreensightConnection | None = hass.data.get(DOMAIN, {}).pop(
            entry.entry_id, None
        )
        if connection is not None:
            await connection.async_stop()
    return unloaded
