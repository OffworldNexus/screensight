#!/usr/bin/env bash
# Acceptance check: while the pairing window is open, the device's mDNS TXT
# payload must contain only id/model/api/version/key/pairing — never the pairing
# code and never anything else.
#
#   ./scripts/check-mdns.sh [service-type] [device-id]
# With a device ID, ignore other devices on the LAN.
#
# Requires `avahi-browse` and a running screensightd on the same host.
set -euo pipefail

SERVICE="${1:-_screensight._tcp}"
DEVICE_ID="${2:-}"
if [ -n "${DEVICE_ID}" ] && [[ ! "${DEVICE_ID}" =~ ^[0-9a-f]+$ ]]; then
    echo "invalid device id: ${DEVICE_ID}" >&2
    exit 1
fi

if ! command -v avahi-browse >/dev/null 2>&1; then
    echo "avahi-browse not found (install avahi-utils)" >&2
    exit 1
fi

echo "== discovered instances =="
# Take one snapshot: discovery and validation must inspect the same records.
records="$(avahi-browse -rtp "${SERVICE}" | grep -E '^=' || true)"
if [ -n "${DEVICE_ID}" ]; then
    records="$(printf '%s\n' "${records}" | grep -F "\"id=${DEVICE_ID}\"" || true)"
fi
if [ -z "${records}" ]; then
    echo "no ${SERVICE} services found${DEVICE_ID:+ for device ${DEVICE_ID}}" >&2
    exit 1
fi
printf '%s\n' "${records}"

echo
echo "== TXT payload validation =="
# avahi-browse -p quotes each TXT entry individually, e.g.
#   "pairing=1" "version=0.1.0" "model=Screensight Studio"
# so read them one quoted token per line (values may contain spaces).
allowed='^(id|model|api|version|key|pairing)='
failed=0
while IFS= read -r entry; do
    [ -z "${entry}" ] && continue
    if ! printf '%s' "${entry}" | grep -qE "${allowed}"; then
        echo "UNEXPECTED TXT ENTRY: ${entry}" >&2
        failed=1
    fi
    if [[ "${entry}" == api=* ]] && [ "${entry}" != api=1 ]; then
        echo "UNSUPPORTED API: ${entry} (expected api=1)" >&2
        failed=1
    fi
done < <(printf '%s\n' "${records}" | grep -oE '"[^"]*"' | tr -d '"' || true)

while IFS= read -r record; do
    if ! printf '%s\n' "${record}" | grep -qF '"api=1"'; then
        echo "FAIL: missing api=1 in record: ${record}" >&2
        failed=1
    fi
done <<<"${records}"

if [ "${failed}" -ne 0 ]; then
    echo "FAIL: unexpected TXT entries (possible code leak)" >&2
    exit 1
fi
echo "OK: only allow-listed TXT keys are advertised"
