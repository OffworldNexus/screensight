#!/usr/bin/env bash
# Deploy Screensight to the Raspberry Pi.
#
# The supported path is the .deb package produced by deploy/deb/build.sh, which
# installs `screensightd`, the `screensight` CLI, the systemd units
# (cage.service / screensight.socket / screensightd.service) and the transparent
# cursor theme.
#
#   PI=remy@172.27.1.58 ./deploy/deploy.sh
#
# `deploy/restore-kiosk.sh` still hands the panel back to the Home Assistant
# kiosk by disabling the Screensight units.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PI="${PI:-remy@172.27.1.58}"

PI="${PI}" "${ROOT}/deploy/deb/build.sh" --install

echo
echo "Follow the logs:   ssh ${PI} journalctl -fu screensightd.service"
echo "Check the status:  ssh ${PI} screensight status"
echo "Restore the kiosk: ./deploy/restore-kiosk.sh"
