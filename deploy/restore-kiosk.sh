#!/usr/bin/env bash
# Stop the GPUI PoC and hand the panel back to the Home Assistant kiosk.
set -euo pipefail
PI="${PI:-remy@172.27.1.58}"

ssh "${PI}" "sudo systemctl stop gpui-poc.service 2>/dev/null || true; \
             sudo systemctl disable gpui-poc.service 2>/dev/null || true; \
             sudo systemctl enable --now kiosk.service"
echo "Home Assistant kiosk restored."
