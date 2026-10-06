#!/usr/bin/env bash
# End-to-end smoke test of the device pipeline on any machine, with no GPU and
# no physical panel:
#
#   boot -> mDNS -> pairing window -> WebSocket pairing -> on-device confirm
#   (via the control CLI) -> saved static keys -> Noise IK set_value round trip.
#
# Requires the Rust toolchain; uses `uv` for the WebSocket client when present
# (otherwise it tells you to run the Rust integration test instead).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp/opencode}/screensight-smoke.XXXXXX")"
KEYS="${WORK}/keys.json"
PORT="${SCREENSIGHT_HTTP_PORT:-18765}"
export SCREENSIGHT_STATE_DIR="${WORK}/state"
export SCREENSIGHT_CONTROL_SOCKET="${WORK}/control.sock"
export SCREENSIGHT_HTTP_PORT="${PORT}"
BIN="${ROOT}/target/debug"

DAEMON_PID=""
FAKE_PID=""
cleanup() {
    result=$?
    if [ "${result}" -ne 0 ]; then
        echo "==> failure diagnostics" >&2
        status_json >"${WORK}/status.log" 2>&1 || true
        for log in status daemon fake fake-text confirm mdns; do
            if [ -f "${WORK}/${log}.log" ]; then
                echo "--- ${log}.log (last 100 lines) ---" >&2
                tail -n 100 "${WORK}/${log}.log" >&2
            fi
        done
    fi
    [ -n "${FAKE_PID}" ] && kill "${FAKE_PID}" 2>/dev/null || true
    [ -n "${DAEMON_PID}" ] && kill "${DAEMON_PID}" 2>/dev/null || true
    [ -n "${FAKE_PID}" ] && wait "${FAKE_PID}" 2>/dev/null || true
    [ -n "${DAEMON_PID}" ] && wait "${DAEMON_PID}" 2>/dev/null || true
    if [ "${result}" -eq 0 ]; then
        rm -rf "${WORK}"
    else
        # Retain diagnostics, not private pairing keys or device state.
        rm -rf "${KEYS}" "${WORK}/state" "${WORK}/code" "${WORK}/control.sock"
        echo "Failure logs retained in ${WORK}" >&2
    fi
}
trap cleanup EXIT

status_json() { "${BIN}/screensight" --json status; }
pairing_sas() {
    status_json | grep -oE '"sas":"[0-9]{8}"' | grep -oE '[0-9]{8}' | head -1
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

ID="$(device_id)"
echo "    device id: ${ID}"

echo "==> mDNS TXT check (if avahi-browse is available)"
if command -v avahi-browse >/dev/null 2>&1; then
    "${ROOT}/scripts/check-mdns.sh" _screensight._tcp "${ID}" >"${WORK}/mdns.log" 2>&1 || {
        echo "FAIL: mDNS TXT check for ${ID}" >&2
        exit 1
    }
    cat "${WORK}/mdns.log"
else
    echo "    avahi-browse not installed; skipping"
fi

echo "==> re-arming pairing window"
"${BIN}/screensight" pair >/dev/null

echo "==> full WebSocket pairing + set_value"
if command -v uv >/dev/null 2>&1; then
    mkfifo "${WORK}/code"
    # Open both ends so startup failures cannot block the shell on FIFO open.
    exec 3<>"${WORK}/code"
    # The parent's read/write descriptor makes the child's read-only open
    # nonblocking. Close the inherited descriptor in the client so stdin sees
    # EOF after the parent delivers the code and closes its writer.
    PYTHONUNBUFFERED=1 timeout 60s uv run "${ROOT}/scripts/fake-ha.py" --port "${PORT}" --keys "${KEYS}" pair \
        <"${WORK}/code" 3>&- >"${WORK}/fake.log" 2>&1 &
    FAKE_PID=$!
    # XX must finish before a SAS exists.
    CODE=""
    for _ in $(seq 1 200); do
        CODE="$(pairing_sas || true)"
        [ -n "${CODE}" ] && break
        kill -0 "${FAKE_PID}" 2>/dev/null || {
            echo "FAIL: pairing client exited before SAS was available" >&2
            exit 1
        }
        sleep 0.1
    done
    [ -n "${CODE}" ] || { echo "FAIL: no SAS after handshake" >&2; exit 1; }
    printf '%s\n' "${CODE}" >&3
    exec 3>&-
    CONFIRMED=false
    # Simulate the user tapping "Pair" on the panel.
    for _ in $(seq 1 100); do
        if "${BIN}/screensight" confirm >"${WORK}/confirm.log" 2>&1; then
            CONFIRMED=true
            break
        fi
        sleep 0.1
    done
    if [ "${CONFIRMED}" != true ] || ! wait "${FAKE_PID}"; then
        echo "FAIL: pairing did not complete" >&2
        cat "${WORK}/fake.log" >&2
        exit 1
    fi
    FAKE_PID=""

    if [ ! -s "${KEYS}" ]; then
        echo "FAIL: pairing did not complete" >&2
        cat "${WORK}/fake.log" >&2
        exit 1
    fi
    echo "    paired; static keys saved"

    PYTHONUNBUFFERED=1 timeout 30s uv run "${ROOT}/scripts/fake-ha.py" --port "${PORT}" --keys "${KEYS}" text \
        "smoke test ✓" >"${WORK}/fake-text.log" 2>&1 || {
        cat "${WORK}/fake-text.log" >&2
        exit 1
    }
    cat "${WORK}/fake-text.log"
    echo "==> final status"
    status_json
else
    echo "    uv not found; skipping the WebSocket round trip."
    echo "    Run the Rust equivalent: cargo test -p screensight --test ws_pairing"
fi

echo
echo "SMOKE TEST OK"
