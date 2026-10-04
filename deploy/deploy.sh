#!/usr/bin/env bash
# Build the GPUI PoC on this machine (cross-compiled to aarch64), ship it to
# the Raspberry Pi, and run it fullscreen under the cage Wayland compositor in
# place of the Home Assistant kiosk.
set -euo pipefail

PI="${PI:-remy@172.27.1.58}"
TARGET="aarch64-unknown-linux-gnu"
BIN="screensight-gpui-poc"
REMOTE_DIR="/home/remy/gpui-poc"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/opencode/screensight-target}"
export CROSS_CONTAINER_ENGINE="${CROSS_CONTAINER_ENGINE:-docker}"

echo "==> [1/5] cross-building ${TARGET} (output in ${CARGO_TARGET_DIR})"
cross build --release --target "${TARGET}"

echo "==> [2/5] copying binary to ${PI}"
ssh "${PI}" "mkdir -p ${REMOTE_DIR}"
scp "${CARGO_TARGET_DIR}/${TARGET}/release/${BIN}" "${PI}:${REMOTE_DIR}/gpui-poc"

echo "==> [3/5] installing transparent cursor theme"
ssh "${PI}" "mkdir -p /home/remy/.local/share/icons && rm -rf /home/remy/.local/share/icons/blank"
scp -r "${ROOT}/deploy/cursor-theme/blank" "${PI}:/home/remy/.local/share/icons/blank"

echo "==> [4/5] installing systemd unit"
scp "${ROOT}/deploy/gpui-poc.service" "${PI}:/tmp/gpui-poc.service"
ssh "${PI}" "sudo cp /tmp/gpui-poc.service /etc/systemd/system/gpui-poc.service && sudo systemctl daemon-reload"

echo "==> [5/5] stopping kiosk, starting GPUI PoC"
ssh "${PI}" "sudo systemctl stop kiosk.service 2>/dev/null || true; \
             sudo systemctl disable kiosk.service 2>/dev/null || true; \
             sudo systemctl restart gpui-poc.service"
sleep 8
ssh "${PI}" "systemctl --no-pager --full status gpui-poc.service | head -14; \
             echo '----- GPU hangs since boot -----'; \
             dmesg | grep -c 'Resetting GPU' || true"

echo
echo "Restore the Home Assistant kiosk:  ./deploy/restore-kiosk.sh"
