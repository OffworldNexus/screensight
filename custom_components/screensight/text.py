"""The ``Display text`` entity exposed for each paired Screensight device."""

from __future__ import annotations

from typing import TYPE_CHECKING

from homeassistant.components.text import TextEntity

from .const import CONF_DEVICE_ID, DOMAIN
from .entity import ScreensightEntity

if TYPE_CHECKING:
    from homeassistant.config_entries import ConfigEntry
    from homeassistant.core import HomeAssistant
    from homeassistant.helpers.entity_platform import AddEntitiesCallback

    from .connection import ScreensightConnection

MAX_LENGTH = 255


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry,
    async_add_entities: AddEntitiesCallback,
) -> None:
    """Set up the Screensight text platform."""
    connection: ScreensightConnection = hass.data[DOMAIN][entry.entry_id]
    async_add_entities([ScreensightText(connection, entry)])


class ScreensightText(ScreensightEntity, TextEntity):
    """Mirror the device's display text and let the user replace it."""

    _attr_name = "Display text"
    _attr_translation_key = "display_text"
    _attr_native_max = MAX_LENGTH

    def __init__(self, connection: ScreensightConnection, entry: ConfigEntry) -> None:
        """Wire the entity to its connection and unique id."""
        super().__init__(connection, entry)
        self._attr_unique_id = f"{entry.data[CONF_DEVICE_ID]}_display_text"

    @property
    def native_value(self) -> str | None:
        """Return the text the device last reported or accepted."""
        return self._connection.text

    async def async_set_value(self, value: str) -> None:
        """Push new text to the device."""
        await self._connection.async_set_text(value)

    async def async_added_to_hass(self) -> None:
        """Subscribe to connection updates once the entity is live."""
        self.async_on_remove(
            self._connection.async_add_listener(self.async_write_ha_state)
        )
