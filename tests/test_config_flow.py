"""Tests for the Screensight config flow."""

from __future__ import annotations

from unittest.mock import AsyncMock

import aiohttp
import pytest
from homeassistant.config_entries import SOURCE_ZEROCONF, ConfigFlowResult
from homeassistant.data_entry_flow import FlowResultType
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.screensight.const import (
    CONF_DEVICE_ID,
    CONF_DEVICE_STATIC_KEY,
    CONF_HA_PRIVATE_KEY,
    CONF_HA_PUBLIC_KEY,
    CONF_HOST,
    CONF_PORT,
    DOMAIN,
)
from tests.helpers import (
    DEVICE_ID,
    DEVICE_STATIC_KEY,
    HA_PRIVATE_KEY,
    HA_PUBLIC_KEY,
    SAS,
    SERVICE_NAME,
    FakeClientSession,
    FakeNoiseTransport,
    FakeWebSocket,
    enc,
    make_discovery,
    pair_frames,
)

_ENTRY_DATA = {
    CONF_DEVICE_ID: DEVICE_ID,
    "name": "screensight-ab12cd34",
    CONF_HOST: "192.168.1.50",
    CONF_PORT: 8765,
    CONF_DEVICE_STATIC_KEY: DEVICE_STATIC_KEY,
    CONF_HA_PRIVATE_KEY: HA_PRIVATE_KEY,
    CONF_HA_PUBLIC_KEY: HA_PUBLIC_KEY,
    "service_name": SERVICE_NAME,
}

#: The bytes the fake device replies with to the XX handshake.
_HANDSHAKE_REPLY = b"handshake-response"


def _patch_session(monkeypatch: pytest.MonkeyPatch, ws: object) -> None:
    """Point the config flow at fakes for the session, Noise and instance id."""
    FakeNoiseTransport.instances = []
    monkeypatch.setattr(
        "custom_components.screensight.config_flow.NoiseTransport",
        FakeNoiseTransport,
    )
    monkeypatch.setattr(
        "homeassistant.helpers.aiohttp_client.async_get_clientsession",
        lambda *args, **kwargs: FakeClientSession(ws),
    )
    monkeypatch.setattr(
        "homeassistant.helpers.instance_id.async_get",
        AsyncMock(return_value="ha-uuid"),
    )


async def _start(hass) -> ConfigFlowResult:
    """Start a zeroconf flow for a device that is pairing."""
    return await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_ZEROCONF}, data=make_discovery()
    )


async def _advance(hass, flow_id: str) -> ConfigFlowResult:
    """Let the current background task finish and step the flow on."""
    await hass.async_block_till_done()
    return await hass.config_entries.flow.async_configure(flow_id)


async def test_zeroconf_without_id_aborts(hass) -> None:
    """A device that advertises no identifier is ignored."""
    result = await hass.config_entries.flow.async_init(
        DOMAIN,
        context={"source": SOURCE_ZEROCONF},
        data=make_discovery(properties={"pairing": "1"}),
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "no_device_id"


async def test_zeroconf_not_pairing_aborts(hass) -> None:
    """A device that is not in pairing mode aborts with ``not_pairing``."""
    result = await hass.config_entries.flow.async_init(
        DOMAIN,
        context={"source": SOURCE_ZEROCONF},
        data=make_discovery(properties={"id": DEVICE_ID}),
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "not_pairing"


async def test_zeroconf_old_firmware_aborts(hass) -> None:
    """A device without the Noise marker cannot be paired."""
    result = await hass.config_entries.flow.async_init(
        DOMAIN,
        context={"source": SOURCE_ZEROCONF},
        data=make_discovery(properties={"id": DEVICE_ID, "pairing": "1"}),
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "unsupported_firmware"


async def test_zeroconf_already_configured_aborts(hass) -> None:
    """A device already paired with this instance is not paired twice."""
    MockConfigEntry(domain=DOMAIN, unique_id=DEVICE_ID, data=_ENTRY_DATA).add_to_hass(
        hass
    )
    result = await _start(hass)
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "already_configured"


async def test_handshake_then_code_creates_entry(hass, monkeypatch) -> None:
    """XX runs first, then the matching code creates the entry."""
    _patch_session(monkeypatch, FakeWebSocket([_HANDSHAKE_REPLY, *pair_frames()]))

    result = await _start(hass)
    assert result["type"] is FlowResultType.SHOW_PROGRESS
    assert result["step_id"] == "pair_connect"

    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.FORM
    assert result["step_id"] == "pair"

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": SAS, "name": "Home Assistant"}
    )
    assert result["type"] is FlowResultType.SHOW_PROGRESS
    assert result["progress_action"] == "pair_confirm"

    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["data"][CONF_DEVICE_ID] == DEVICE_ID
    assert result["data"][CONF_HOST] == "192.168.1.50"
    assert result["data"][CONF_DEVICE_STATIC_KEY] == DEVICE_STATIC_KEY
    assert result["data"][CONF_HA_PRIVATE_KEY]
    assert result["data"][CONF_HA_PUBLIC_KEY]
    # No long-lived token is ever stored.
    assert "token" not in result["data"]
    assert result["title"] == "screensight-ab12cd34"


async def test_invalid_code_format_stays_on_form(hass, monkeypatch) -> None:
    """A code that is not 8 digits is rejected before any frame is sent."""
    _patch_session(monkeypatch, FakeWebSocket([_HANDSHAKE_REPLY, *pair_frames()]))
    result = await _start(hass)
    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.FORM

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "12ab", "name": "Home Assistant"}
    )
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"code": "invalid_code_format"}


async def test_mismatched_sas_stays_on_form(hass, monkeypatch) -> None:
    """A typed code that differs from our SAS is rejected locally."""
    _patch_session(monkeypatch, FakeWebSocket([_HANDSHAKE_REPLY, *pair_frames()]))
    result = await _start(hass)
    result = await _advance(hass, result["flow_id"])

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "00000000", "name": "Home Assistant"}
    )
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"code": "invalid_code"}


@pytest.mark.parametrize("reason", ["window_closed", "rate_limited"])
async def test_pair_error_maps_to_form_error(hass, monkeypatch, reason: str) -> None:
    """Device ``pair_error`` reasons surface as translated form errors."""
    _patch_session(
        monkeypatch,
        FakeWebSocket(
            [_HANDSHAKE_REPLY, enc({"type": "pair_error", "reason": reason})]
        ),
    )
    result = await _start(hass)
    result = await _advance(hass, result["flow_id"])
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": SAS, "name": "Home Assistant"}
    )
    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"code": reason}


async def test_pair_rejected_aborts(hass, monkeypatch) -> None:
    """An on-panel rejection aborts the flow with ``pairing_declined``."""
    _patch_session(
        monkeypatch,
        FakeWebSocket(
            [_HANDSHAKE_REPLY, enc({"type": "pair_rejected", "reason": "declined"})]
        ),
    )
    result = await _start(hass)
    result = await _advance(hass, result["flow_id"])
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": SAS, "name": "Home Assistant"}
    )
    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "pairing_declined"


async def test_handshake_cannot_connect(hass, monkeypatch) -> None:
    """A device that is unreachable aborts with ``cannot_connect``."""
    _patch_session(monkeypatch, aiohttp.ClientConnectionError("no route to host"))
    result = await _start(hass)
    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "cannot_connect"


async def test_reconfigure_reuses_keypair_and_updates_entry(hass, monkeypatch) -> None:
    """Reconfiguring keeps Home Assistant's keypair and stores the device key."""
    entry = MockConfigEntry(domain=DOMAIN, unique_id=DEVICE_ID, data=_ENTRY_DATA)
    entry.add_to_hass(hass)
    _patch_session(monkeypatch, FakeWebSocket([_HANDSHAKE_REPLY, *pair_frames()]))

    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": "reconfigure", "entry_id": entry.entry_id}
    )
    assert result["type"] is FlowResultType.SHOW_PROGRESS

    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.FORM

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": SAS, "name": "Home Assistant"}
    )
    result = await _advance(hass, result["flow_id"])
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "reconfigure_successful"
    assert entry.data[CONF_HA_PRIVATE_KEY] == HA_PRIVATE_KEY
    assert entry.data["service_name"] == SERVICE_NAME
