"""Constants and configuration keys for the Screensight integration."""

from __future__ import annotations

from typing import Final

from homeassistant.const import Platform

DOMAIN: Final = "screensight"
MANUFACTURER: Final = "Screensight"
DEFAULT_MODEL: Final = "Screensight"

PLATFORMS: Final = [Platform.TEXT]

# Config entry / flow keys.
CONF_DEVICE_ID: Final = "device_id"
CONF_NAME: Final = "name"
CONF_HOST: Final = "host"
CONF_PORT: Final = "port"
CONF_TOKEN: Final = "token"  # noqa: S105 -- config key, not a secret value
CONF_HA_ID: Final = "ha_id"
CONF_HA_NAME: Final = "ha_name"
CONF_CODE: Final = "code"
CONF_MODEL: Final = "model"
CONF_VERSION: Final = "version"
CONF_SERVICE_NAME: Final = "service_name"

# mDNS / transport defaults.
SERVICE_TYPE: Final = "_screensight._tcp.local."
DEFAULT_PORT: Final = 8765
DEFAULT_HA_NAME: Final = "Home Assistant"
WS_PATH: Final = "/ws"

# Client -> device message types.
TYPE_PAIR: Final = "pair"
TYPE_GET_STATE: Final = "get_state"
TYPE_SET_TEXT: Final = "set_text"
TYPE_PING: Final = "ping"

# Device -> client message types.
TYPE_PAIR_PENDING: Final = "pair_pending"
TYPE_PAIR_SUCCESS: Final = "pair_success"
TYPE_PAIR_REJECTED: Final = "pair_rejected"
TYPE_PAIR_ERROR: Final = "pair_error"
TYPE_STATE: Final = "state"
TYPE_PONG: Final = "pong"
TYPE_ERROR: Final = "error"

# Timing.
HEARTBEAT_INTERVAL: Final = 15
MAX_MISSED_HEARTBEATS: Final = 2
CONNECT_TIMEOUT: Final = 10
PAIR_TIMEOUT: Final = 120
RECONNECT_MIN: Final = 1
RECONNECT_MAX: Final = 60

# Dispatcher signal fired whenever the connection's cached state changes.
SIGNAL_STATE_UPDATED: Final = f"{DOMAIN}_state_updated_{{}}"
