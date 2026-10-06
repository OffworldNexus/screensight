"""Constants and configuration keys for the Screensight integration."""

from __future__ import annotations

from typing import Final

from homeassistant.const import Platform

DOMAIN: Final = "screensight"
MANUFACTURER: Final = "Offworld Nexus"
DEFAULT_MODEL: Final = "Screensight"

PLATFORMS: Final = [Platform.TEXT]

# Config entry / flow keys.
CONF_DEVICE_ID: Final = "device_id"
CONF_NAME: Final = "name"
CONF_HOST: Final = "host"
CONF_PORT: Final = "port"
#: The device's static Noise public key (hex), learned at pairing.
CONF_DEVICE_STATIC_KEY: Final = "device_static_key"
#: Home Assistant's own static Noise keypair (hex), generated once per entry.
CONF_HA_PRIVATE_KEY: Final = "ha_private_key"
CONF_HA_PUBLIC_KEY: Final = "ha_public_key"
CONF_HA_ID: Final = "ha_id"
CONF_HA_NAME: Final = "ha_name"
#: Form field for the 8-digit SAS the user reads off the panel.
CONF_CODE: Final = "code"
CONF_MODEL: Final = "model"
CONF_VERSION: Final = "version"
CONF_SERVICE_NAME: Final = "service_name"

# The device rejects an IK reconnect from an unknown key with this phrase inside
# the encrypted channel; the connection manager uses it to raise a repair flow.
UNKNOWN_KEY_MARKER: Final = "unknown static key"

# mDNS / transport defaults.
SERVICE_TYPE: Final = "_screensight._tcp.local."
DEFAULT_PORT: Final = 8765
DEFAULT_HA_NAME: Final = "Home Assistant"
WS_PATH: Final = "/ws"

# Client -> device message types.
TYPE_PAIR: Final = "pair"
TYPE_SET_VALUE: Final = "set_value"
TYPE_SET_STATE: Final = "set_state"
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
