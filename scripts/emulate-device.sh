#!/usr/bin/env bash
# Run the Screensight device on your desktop, as an "emulated Pi".
#
# Uses the same `screensightd` daemon and GPUI panel that ship to the device,
# with state under `.dev/device` and the panel in an 800x480 window. Combined
# with `scripts/ha-dev.sh` on the same host this exercises the whole loop
# (mDNS discovery -> pairing -> display text) without any Raspberry Pi.
#
# Set SCREENSIGHT_DISPLAY=:0 (X11) or WAYLAND_DISPLAY=wayland-0 first if your
# session needs it. The panel is 800x480 regardless of your screen size.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STATE_DIR="${SCREENSIGHT_STATE_DIR:-${ROOT}/.dev/device}"
mkdir -p "${STATE_DIR}"

if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
    echo "No DISPLAY / WAYLAND_DISPLAY set. Falling back to Xvfb on :99."
    if command -v Xvfb >/dev/null 2>&1; then
        Xvfb :99 -screen 0 800x480x24 >/dev/null 2>&1 &
        export DISPLAY=:99
        sleep 1
    else
        echo "Install Xvfb or run inside a graphical session." >&2
        exit 1
    fi
fi

export SCREENSIGHT_STATE_DIR="${STATE_DIR}"
export SCREENSIGHT_CONTROL_SOCKET="${SCREENSIGHT_CONTROL_SOCKET:-${STATE_DIR}/control.sock}"
export RUST_LOG="${RUST_LOG:-warn,screensight=info,screensightd=info}"

echo "State:      ${STATE_DIR}"
echo "Control:    ${SCREENSIGHT_CONTROL_SOCKET}"
echo "Status:     SCREENSIGHT_CONTROL_SOCKET=${SCREENSIGHT_CONTROL_SOCKET} ./target/debug/screensight status"
echo

exec cargo run -p screensight --features gui --bin screensightd -- "$@"
