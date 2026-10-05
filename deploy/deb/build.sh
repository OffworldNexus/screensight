#!/usr/bin/env bash
# Build the Screensight .deb for the Raspberry Pi (aarch64) and, optionally,
# copy it to the device.
#
#   ./deploy/deb/build.sh              # build only
#   PI=remy@172.27.1.58 ./deploy/deb/build.sh --install
#
# The package installs `screensightd`, the `screensight` CLI, the systemd units
# and the transparent cursor theme. It depends on `cage` and `avahi-daemon`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="aarch64-unknown-linux-gnu"
ARCH="arm64"
OUT="${ROOT}/deploy/deb/out"
VERSION="$(grep -m1 '^version' "${ROOT}/device/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
PKG="screensight_${VERSION}_${ARCH}"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/opencode/screensight-target}"
export CROSS_CONTAINER_ENGINE="${CROSS_CONTAINER_ENGINE:-docker}"

echo "==> [1/5] cross-building ${TARGET} (screensightd + screensight CLI)"
cd "${ROOT}"
cross build --release --target "${TARGET}" -p screensight --features gui

echo "==> [2/5] staging package tree ${PKG}"
rm -rf "${OUT}/${PKG}"
mkdir -p \
    "${OUT}/${PKG}/DEBIAN" \
    "${OUT}/${PKG}/usr/bin" \
    "${OUT}/${PKG}/lib/systemd/system" \
    "${OUT}/${PKG}/usr/share/icons" \
    "${OUT}/${PKG}/usr/share/doc/screensight"

install -m 0755 "${CARGO_TARGET_DIR}/${TARGET}/release/screensightd" "${OUT}/${PKG}/usr/bin/screensightd"
install -m 0755 "${CARGO_TARGET_DIR}/${TARGET}/release/screensight" "${OUT}/${PKG}/usr/bin/screensight"
install -m 0644 "${ROOT}/deploy/systemd/cage.service" "${OUT}/${PKG}/lib/systemd/system/cage.service"
install -m 0644 "${ROOT}/deploy/systemd/screensight.socket" "${OUT}/${PKG}/lib/systemd/system/screensight.socket"
install -m 0644 "${ROOT}/deploy/systemd/screensightd.service" "${OUT}/${PKG}/lib/systemd/system/screensightd.service"
cp -r "${ROOT}/deploy/cursor-theme/blank" "${OUT}/${PKG}/usr/share/icons/screensight-blank"
install -m 0644 "${ROOT}/README.md" "${OUT}/${PKG}/usr/share/doc/screensight/README.md"

echo "==> [3/5] writing control scripts"
sed "s/@VERSION@/${VERSION}/" "${ROOT}/deploy/deb/control.in" > "${OUT}/${PKG}/DEBIAN/control"
install -m 0755 "${ROOT}/deploy/deb/postinst" "${OUT}/${PKG}/DEBIAN/postinst"
install -m 0755 "${ROOT}/deploy/deb/prerm" "${OUT}/${PKG}/DEBIAN/prerm"

echo "==> [4/5] building .deb"
dpkg-deb --build --root-owner-group "${OUT}/${PKG}"
echo "    ${OUT}/${PKG}.deb"

if [ "${1:-}" = "--install" ]; then
    PI="${PI:-remy@172.27.1.58}"
    echo "==> [5/5] installing on ${PI}"
    scp "${OUT}/${PKG}.deb" "${PI}:/tmp/${PKG}.deb"
    ssh "${PI}" "sudo apt-get install -y --reinstall /tmp/${PKG}.deb"
else
    echo "==> [5/5] skipped install (pass --install to deploy)"
fi
