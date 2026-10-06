"""Step definitions for the Screensight BDD scenarios.

The Home Assistant ``hass`` fixture is async, and pytest-bdd executes step
functions synchronously, so each step drives the Home Assistant event loop
explicitly via ``hass.loop.run_until_complete``. That keeps the Gherkin prose
async-free while still exercising the real config flow and entity code.
"""

from __future__ import annotations

from unittest.mock import AsyncMock

from homeassistant.config_entries import SOURCE_ZEROCONF
from homeassistant.data_entry_flow import FlowResultType
from pytest_bdd import given, parsers, then, when
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.screensight.connection import ScreensightConnection
from custom_components.screensight.const import (
    CONF_DEVICE_ID,
    CONF_DEVICE_STATIC_KEY,
    CONF_HA_PRIVATE_KEY,
    CONF_HOST,
    CONF_NAME,
    DOMAIN,
)
from custom_components.screensight.text import ScreensightText
from tests.helpers import (
    DEVICE_ID,
    DEVICE_STATIC_KEY,
    HA_PRIVATE_KEY,
    FakeClientSession,
    FakeNoiseTransport,
    FakeWebSocket,
    dec,
    make_discovery,
    pair_frames,
)

_HANDSHAKE_REPLY = b"handshake-response"


@given(
    "a Screensight device is advertising itself over mDNS in pairing mode",
    target_fixture="pairing",
)
def _advertising(hass, monkeypatch):
    """Discover a pairing device and park the flow on its code form."""
    FakeNoiseTransport.instances = []
    monkeypatch.setattr(
        "custom_components.screensight.config_flow.NoiseTransport",
        FakeNoiseTransport,
    )
    session = FakeClientSession(FakeWebSocket([_HANDSHAKE_REPLY, *pair_frames()]))
    monkeypatch.setattr(
        "homeassistant.helpers.aiohttp_client.async_get_clientsession",
        lambda *args, **kwargs: session,
    )
    monkeypatch.setattr(
        "homeassistant.helpers.instance_id.async_get",
        AsyncMock(return_value="ha-uuid"),
    )
    flow = hass.loop.run_until_complete(
        hass.config_entries.flow.async_init(
            DOMAIN, context={"source": SOURCE_ZEROCONF}, data=make_discovery()
        )
    )
    hass.loop.run_until_complete(hass.async_block_till_done())
    flow = hass.loop.run_until_complete(
        hass.config_entries.flow.async_configure(flow["flow_id"])
    )
    assert flow["type"] is FlowResultType.FORM
    return {"hass": hass, "flow_id": flow["flow_id"]}


@when(parsers.parse('I submit the pairing code "{code}"'))
def _submit_code(pairing, code):
    """Submit the panel code and start the pairing exchange."""
    result = pairing["hass"].loop.run_until_complete(
        pairing["hass"].config_entries.flow.async_configure(
            pairing["flow_id"], {"code": code, "name": "Home Assistant"}
        )
    )
    assert result["type"] is FlowResultType.SHOW_PROGRESS
    pairing["result"] = result


@when("the device confirms the pairing on its panel")
def _confirm_on_panel(pairing):
    """Wait for the device to answer ``pair_success`` and finish the flow."""
    hass = pairing["hass"]
    hass.loop.run_until_complete(hass.async_block_till_done())
    pairing["result"] = hass.loop.run_until_complete(
        hass.config_entries.flow.async_configure(pairing["flow_id"])
    )


@then("a Screensight config entry exists with mutual static keys")
def _entry_created(pairing):
    """The completed flow must have created a key-bearing entry."""
    result = pairing["result"]
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["data"][CONF_DEVICE_STATIC_KEY] == DEVICE_STATIC_KEY
    assert result["data"][CONF_HA_PRIVATE_KEY]
    assert result["data"][CONF_DEVICE_ID] == DEVICE_ID
    entries = pairing["hass"].config_entries.async_entries(DOMAIN)
    assert len(entries) == 1


@given("a paired Screensight device is connected", target_fixture="display")
def _connected_device(hass):
    """Wire a text entity to a fake in-memory connection."""
    entry = MockConfigEntry(
        domain=DOMAIN,
        data={
            CONF_DEVICE_ID: DEVICE_ID,
            CONF_NAME: "screensight-ab12cd34",
            CONF_HOST: "192.168.1.50",
        },
    )
    connection = ScreensightConnection(
        hass,
        entry,
        host="192.168.1.50",
        port=8765,
        ha_private=bytes.fromhex(HA_PRIVATE_KEY),
        device_static=bytes.fromhex(DEVICE_STATIC_KEY),
    )
    ws = FakeWebSocket()
    connection._ws = ws
    connection._transport = FakeNoiseTransport()
    connection._set_connected(True)
    return {
        "hass": hass,
        "connection": connection,
        "entity": ScreensightText(connection, entry),
        "ws": ws,
    }


@when(parsers.parse('I set the Display text entity to "{value}"'))
def _set_value(display, value):
    """Ask the entity to update the panel text."""
    display["hass"].loop.run_until_complete(display["entity"].async_set_value(value))


@then(parsers.parse('the device receives a "{key}" value of "{value}"'))
def _frame_received(display, key, value):
    """The exact ``set_value`` frame must have reached the device."""
    assert {"type": "set_value", "key": key, "value": value} in [
        dec(blob) for blob in display["ws"].sent
    ]
    assert display["connection"].value(key) == value
