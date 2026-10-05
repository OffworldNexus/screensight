#!/usr/bin/env bash
# Acceptance check: while the pairing window is open, the device's mDNS TXT
# payload must contain only id/model/api/version/pairing — never the pairing
# code and never anything else.
#
#   ./scripts/check-mdns.sh [service-type]
#
# Requires `avahi-browse` and a running screensightd on the same host.
set -euo pipefail

SERVICE="${1:-_screensight._tcp}"

if ! command -v avahi-browse >/dev/null 2>&1; then
    echo "avahi-browse not found (install avahi-utils)" >&2
    exit 1
fi

echo "== discovered instances =="
avahi-browse -rtp "${SERVICE}" 2>/dev/null | grep -E '^=' || {
    echo "no ${SERVICE} services found" >&2
    exit 1
}

echo
echo "== TXT payload validation =="
# avahi-browse -p quotes each TXT entry individually, e.g.
#   "pairing=1" "version=0.1.0" "model=Screensight Studio"
# so read them one quoted token per line (values may contain spaces).
allowed='^(id|model|api|version|pairing)='
failed=0
while IFS= read -r entry; do
    [ -z "${entry}" ] && continue
    if ! printf '%s' "${entry}" | grep -qE "${allowed}"; then
        echo "UNEXPECTED TXT ENTRY: ${entry}" >&2
        failed=1
    fi
done < <(avahi-browse -rtp "${SERVICE}" 2>/dev/null | grep -oE '"[^"]*"' | tr -d '"' || true)

if [ "${failed}" -ne 0 ]; then
    echo "FAIL: unexpected TXT entries (possible code leak)" >&2
    exit 1
fi
echo "OK: only allow-listed TXT keys are advertised"
