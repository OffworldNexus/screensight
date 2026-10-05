"""Tests for the Screensight ``Display text`` entity."""

from __future__ import annotations

from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.screensight.connection import ScreensightConnection
from custom_components.screensight.const import CONF_DEVICE_ID, CONF_HOST, DOMAIN
from custom_components.screensight.text import ScreensightText
from tests.helpers import FakeWebSocket

_TOKEN = "tok"


def _entity(hass):
    """Build a text entity backed by a fake in-memory connection."""
    entry = MockConfigEntry(
        domain=DOMAIN,
        data={
            CONF_DEVICE_ID: "dev",
            "name": "screensight-dev",
            CONF_HOST: "192.168.1.50",
        },
    )
    connection = ScreensightConnection(
        hass, entry, host="192.168.1.50", port=8765, token=_TOKEN
    )
    ws = FakeWebSocket()
    connection._ws = ws
    connection._set_connected(True)
    return ScreensightText(connection, entry), connection, ws


async def test_set_value_sends_frame(hass) -> None:
    """``async_set_value`` forwards the text unchanged, emoji and all."""
    entity, connection, ws = _entity(hass)

    await entity.async_set_value("Hello 🌧")

    assert ws.sent == [{"type": "set_text", "text": "Hello 🌧"}]
    assert entity.native_value == "Hello 🌧"
    assert connection.text == "Hello 🌧"


async def test_entity_metadata(hass) -> None:
    """The entity advertises the expected name, limit and unique id."""
    entity, _, _ = _entity(hass)
    assert entity.name == "Display text"
    assert entity.translation_key == "display_text"
    assert entity.native_max == 255
    assert entity.unique_id == "dev_display_text"


async def test_entity_becomes_unavailable(hass) -> None:
    """Availability follows the connection flag."""
    entity, connection, _ = _entity(hass)
    assert entity.available is True
    connection._set_connected(False)
    assert entity.available is False
