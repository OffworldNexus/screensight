"""Base entity shared by the Screensight platforms."""

from __future__ import annotations

from typing import TYPE_CHECKING

from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity import Entity

from .const import (
    CONF_DEVICE_ID,
    CONF_MODEL,
    CONF_NAME,
    CONF_VERSION,
    DEFAULT_MODEL,
    DOMAIN,
    MANUFACTURER,
)

if TYPE_CHECKING:
    from homeassistant.config_entries import ConfigEntry

    from .connection import ScreensightConnection


class ScreensightEntity(Entity):
    """Common behaviour for entities backed by a Screensight connection."""

    _attr_has_entity_name = True

    def __init__(self, connection: ScreensightConnection, entry: ConfigEntry) -> None:
        """Attach the entity to its connection and device registry entry."""
        self._connection = connection
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, entry.data[CONF_DEVICE_ID])},
            name=entry.data.get(CONF_NAME),
            manufacturer=MANUFACTURER,
            model=entry.data.get(CONF_MODEL, DEFAULT_MODEL),
            sw_version=entry.data.get(CONF_VERSION),
        )

    @property
    def available(self) -> bool:
        """Return whether the underlying device connection is up."""
        return self._connection.available
