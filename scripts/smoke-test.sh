#!/usr/bin/env bash
# End-to-end smoke test of the device pipeline on any machine, with no GPU and
# no physical panel:
#
#   boot -> mDNS -> pairing window -> WebSocket pairing -> on-device confirm
#   (via the control CLI) -> token -> set_text round trip.
#
# Requires the Rust toolchain; uses `uv` for the WebSocket client when present
# (otherwise it tells you to run the Rust integration test instead).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/screensight-smoke.XXXXXX")"
PORT="${SCREENSIGHT_HTTP_PORT:-18765}"
export SCREENSIGHT_STATE_DIR="${WORK}/state"
export SCREENSIGHT_CONTROL_SOCKET="${WORK}/control.sock"
export SCREENSIGHT_HTTP_PORT="${PORT}"
BIN="${ROOT}/target/debug"

DAEMON_PID=""
FAKE_PID=""
cleanup() {
    [ -n "${FAKE_PID}" ] && kill "${FAKE_PID}" 2>/dev/null || true
    [ -n "${DAEMON_PID}" ] && kill "${DAEMON_PID}" 2>/dev/null || true
    rm -rf "${WORK}"
}
trap cleanup EXIT

status_json() { "${BIN}/screensight" --json status; }
pairing_code() {
    status_json | grep -oE '"pairing_code":"[0-9]{6}"' | grep -oE '[0-9]{6}' | head -1
}
device_id() {
    status_json | grep -oE '"id":"[0-9a-f]+"' | sed -E 's/.*:"(.*)"/\1/' | head -1
}

echo "==> building"
cargo build -p screensight --quiet

echo "==> starting screensightd (headless, port ${PORT})"
"${BIN}/screensightd" --headless >"${WORK}/daemon.log" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 1 50); do
    [ -S "${SCREENSIGHT_CONTROL_SOCKET}" ] && break
    sleep 0.1
done

echo "==> status"
status_json

CODE="$(pairing_code)"
ID="$(device_id)"
[ -n "${CODE}" ] || { echo "FAIL: no pairing code in status" >&2; exit 1; }
echo "    device id: ${ID}   pairing code: ${CODE}"

echo "==> mDNS TXT check (if avahi-browse is available)"
if command -v avahi-browse >/dev/null 2>&1; then
    "${ROOT}/scripts/check-mdns.sh" || true
else
    echo "    avahi-browse not installed; skipping"
fi

echo "==> re-arming pairing window"
"${BIN}/screensight" pair >/dev/null
CODE="$(pairing_code)"
echo "    new pairing code: ${CODE}"

echo "==> full WebSocket pairing + set_text"
if command -v uv >/dev/null 2>&1; then
    uv run "${ROOT}/scripts/fake-ha.py" --port "${PORT}" pair \
        --code "${CODE}" --device-id "${ID}" >"${WORK}/fake.log" 2>&1 &
    FAKE_PID=$!
    sleep 3
    # Simulate the user tapping "Yes · pair this display" on the panel.
    "${BIN}/screensight" confirm >/dev/null
    wait "${FAKE_PID}" || true
    FAKE_PID=""

    TOKEN="$(grep -oE 'token = [0-9a-f]{64}' "${WORK}/fake.log" | awk '{print $3}' || true)"
    if [ -z "${TOKEN}" ]; then
        echo "FAIL: pairing did not complete" >&2
        cat "${WORK}/fake.log" >&2
        exit 1
    fi
    echo "    paired; token acquired"

    uv run "${ROOT}/scripts/fake-ha.py" --port "${PORT}" text \
        --token "${TOKEN}" "smoke test ✓"
    echo "==> final status"
    status_json
else
    echo "    uv not found; skipping the WebSocket round trip."
    echo "    Run the Rust equivalent: cargo test -p screensight --test ws_pairing"
fi

echo
echo "SMOKE TEST OK"
