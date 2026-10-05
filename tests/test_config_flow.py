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
    CONF_HOST,
    CONF_PORT,
    CONF_TOKEN,
    DOMAIN,
)
from tests.helpers import (
    DEVICE_ID,
    SERVICE_NAME,
    TOKEN,
    FakeClientSession,
    FakeWebSocket,
    make_discovery,
    pair_frames,
)

_ENTRY_DATA = {
    CONF_DEVICE_ID: DEVICE_ID,
    "name": "screensight-ab12cd34",
    CONF_HOST: "192.168.1.50",
    CONF_PORT: 8765,
    CONF_TOKEN: "old-token",
    "service_name": SERVICE_NAME,
}


def _patch_session(monkeypatch: pytest.MonkeyPatch, ws: object) -> None:
    """Point the config flow at a fake client session and instance id."""
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


async def _finish_pairing(hass, flow_id: str) -> ConfigFlowResult:
    """Drive the progress step to completion and return the final result."""
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


async def test_zeroconf_already_configured_aborts(hass) -> None:
    """A device already paired with this instance is not paired twice."""
    MockConfigEntry(domain=DOMAIN, unique_id=DEVICE_ID, data=_ENTRY_DATA).add_to_hass(
        hass
    )
    result = await _start(hass)
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "already_configured"


async def test_pair_success_creates_entry(hass, monkeypatch) -> None:
    """A correct code plus an on-panel confirmation creates the entry."""
    _patch_session(monkeypatch, FakeWebSocket(pair_frames()))

    result = await _start(hass)
    assert result["type"] is FlowResultType.FORM
    assert result["step_id"] == "pair"

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "123456", "name": "Home Assistant"}
    )
    assert result["type"] is FlowResultType.SHOW_PROGRESS
    assert result["progress_action"] == "pair_confirm"

    result = await _finish_pairing(hass, result["flow_id"])
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["data"][CONF_TOKEN] == TOKEN
    assert result["data"][CONF_DEVICE_ID] == DEVICE_ID
    assert result["data"][CONF_HOST] == "192.168.1.50"
    assert result["title"] == "screensight-ab12cd34"


async def test_invalid_code_format_stays_on_form(hass, monkeypatch) -> None:
    """A code that is not 6 digits is rejected before any socket is opened."""
    _patch_session(monkeypatch, FakeWebSocket(pair_frames()))
    result = await _start(hass)
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "12ab", "name": "Home Assistant"}
    )
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"code": "invalid_code_format"}


@pytest.mark.parametrize(
    ("reason", "expected"),
    [
        ("invalid_code", "invalid_code"),
        ("rate_limited", "rate_limited"),
        ("window_closed", "window_closed"),
        ("wrong_device", "wrong_device"),
    ],
)
async def test_pair_error_maps_to_form_error(
    hass, monkeypatch, reason: str, expected: str
) -> None:
    """Device ``pair_error`` reasons surface as translated form errors."""
    _patch_session(
        monkeypatch, FakeWebSocket([{"type": "pair_error", "reason": reason}])
    )
    result = await _start(hass)
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "123456", "name": "Home Assistant"}
    )
    result = await _finish_pairing(hass, result["flow_id"])
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"code": expected}


async def test_pair_rejected_aborts(hass, monkeypatch) -> None:
    """An on-panel rejection aborts the flow with ``pairing_declined``."""
    _patch_session(
        monkeypatch,
        FakeWebSocket([{"type": "pair_rejected", "reason": "declined"}]),
    )
    result = await _start(hass)
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "123456", "name": "Home Assistant"}
    )
    result = await _finish_pairing(hass, result["flow_id"])
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "pairing_declined"


async def test_pair_cannot_connect(hass, monkeypatch) -> None:
    """A device that is unreachable yields a ``cannot_connect`` form error."""
    _patch_session(monkeypatch, aiohttp.ClientConnectionError("no route to host"))
    result = await _start(hass)
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "123456", "name": "Home Assistant"}
    )
    result = await _finish_pairing(hass, result["flow_id"])
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"code": "cannot_connect"}


async def test_reconfigure_updates_entry(hass, monkeypatch) -> None:
    """Reconfiguring an existing device replaces its stored token."""
    entry = MockConfigEntry(domain=DOMAIN, unique_id=DEVICE_ID, data=_ENTRY_DATA)
    entry.add_to_hass(hass)
    _patch_session(monkeypatch, FakeWebSocket(pair_frames()))

    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": "reconfigure", "entry_id": entry.entry_id}
    )
    assert result["type"] is FlowResultType.FORM

    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"code": "123456", "name": "Home Assistant"}
    )
    assert result["type"] is FlowResultType.SHOW_PROGRESS
    result = await _finish_pairing(hass, result["flow_id"])
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "reconfigure_successful"
    assert entry.data[CONF_TOKEN] == TOKEN
    assert entry.data["service_name"] == SERVICE_NAME
