#!/usr/bin/env bash
# Run a throwaway Home Assistant Core locally with the Screensight integration.
#
# Managed lifecycle so it always responds to Ctrl+C and never traps your shell:
#
#   ./scripts/ha-dev.sh up        start (detached) + auto-onboard + print info
#   ./scripts/ha-dev.sh logs      follow the logs (Ctrl+C just stops following)
#   ./scripts/ha-dev.sh restart   down + up
#   ./scripts/ha-dev.sh down      stop and remove the container
#
# Onboarding is automatic: there is no setup wizard. Log in with dev / dev.
# This never touches production.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG_DIR="${SCREENSIGHT_HA_CONFIG:-${ROOT}/.dev/homeassistant}"
export SCREENSIGHT_HA_CONFIG="${CONFIG_DIR}"
export SCREENSIGHT_HA_IMAGE="${SCREENSIGHT_HA_IMAGE:-ghcr.io/home-assistant/home-assistant:stable}"
export SCREENSIGHT_HA_UID="${SCREENSIGHT_HA_UID:-$(id -u)}"
export SCREENSIGHT_HA_GID="${SCREENSIGHT_HA_GID:-$(id -g)}"
URL="${SCREENSIGHT_HA_URL:-http://localhost:8123}"
COMPOSE=(docker compose -f "${ROOT}/scripts/ha-dev.compose.yml")

require_docker() {
    command -v docker >/dev/null 2>&1 || {
        echo "error: docker is not installed" >&2
        exit 1
    }
    docker info >/dev/null 2>&1 || {
        echo "error: cannot talk to the Docker daemon" >&2
        exit 1
    }
}

seed_config() {
    mkdir -p "${CONFIG_DIR}/custom_components"
    # Refresh the mounted copy of the integration on every start.
    rm -rf "${CONFIG_DIR}/custom_components/screensight"
    cp -r "${ROOT}/custom_components/screensight" "${CONFIG_DIR}/custom_components/screensight"
    find "${CONFIG_DIR}/custom_components/screensight" -name __pycache__ -type d \
        -exec rm -rf {} + 2>/dev/null || true

    if [ ! -f "${CONFIG_DIR}/configuration.yaml" ]; then
        cat > "${CONFIG_DIR}/configuration.yaml" <<'YAML'
# Throwaway Screensight development instance. No secrets, no production data.
default_config:

logger:
  default: info
  logs:
    custom_components.screensight: debug
YAML
    fi
}

cmd_up() {
    require_docker
    seed_config
    echo "==> starting Home Assistant (${SCREENSIGHT_HA_IMAGE})"
    "${COMPOSE[@]}" up -d
    echo "==> waiting for it to become ready and onboarding automatically"
    PYTHONUNBUFFERED=1 uv run --no-project "${ROOT}/scripts/ha-provision.py" "${URL}" || true
    echo
    echo "Home Assistant: ${URL}   (login: dev / dev)"
    echo "Follow logs:    ./scripts/ha-dev.sh logs"
    echo "Stop:           ./scripts/ha-dev.sh down"
    echo
    echo "Now start the emulated panel in another terminal:"
    echo "    ./scripts/emulate-device.sh"
}

cmd_logs() {
    require_docker
    "${COMPOSE[@]}" logs -f --tail=100
}

cmd_down() {
    require_docker
    "${COMPOSE[@]}" down --remove-orphans
    # Belt and braces in case a previous `docker run` left the name behind.
    docker rm -f screensight-ha >/dev/null 2>&1 || true
    echo "stopped and removed screensight-ha"
}

cmd_reset() {
    cmd_down
    rm -rf "${CONFIG_DIR}"
    echo "wiped ${CONFIG_DIR} (next 'up' will onboard fresh with dev / dev)"
}

case "${1:-up}" in
    up) cmd_up ;;
    up-fg)
        require_docker
        seed_config
        # Foreground mode; the compose file sets `init: true` so Ctrl+C works.
        "${COMPOSE[@]}" up
        ;;
    logs) cmd_logs ;;
    restart)
        cmd_down
        cmd_up
        ;;
    down) cmd_down ;;
    reset) cmd_reset ;;
    *)
        echo "usage: $0 {up|up-fg|logs|restart|down|reset}" >&2
        exit 2
        ;;
esac
