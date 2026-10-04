# Custom `cross` image for aarch64-unknown-linux-gnu.
#
# We can't use cross's stock image: it is built on Ubuntu 16.04, whose libxcb
# (1.11) predates `xcb_send_request_with_fds64` (libxcb >= 1.16), which
# x11rb's `xcb_ffi` backend — used by GPUI to hand an XCB connection to the
# Vulkan XCB WSI — requires. Debian trixie (the Pi's own distro) ships
# libxcb 1.17, so we build on `rust:1.98-trixie` (host/amd64) and add arm64
# dev libraries via multiarch.
FROM rust:1.98-trixie

RUN dpkg --add-architecture arm64 \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu \
        g++-aarch64-linux-gnu \
        pkg-config \
        libxkbcommon-dev:arm64 \
        libxkbcommon-x11-dev:arm64 \
        libfontconfig1-dev:arm64 \
        libfreetype-dev:arm64 \
        libexpat1-dev:arm64 \
        libwayland-dev:arm64 \
        libvulkan-dev:arm64 \
        libssl-dev:arm64 \
        libxcb1-dev:arm64 \
        libxcb-xkb-dev:arm64 \
 && rm -rf /var/lib/apt/lists/*

# cross's stock images preinstall the target's std; ours must too.
RUN rustup target add --toolchain 1.98.1 aarch64-unknown-linux-gnu

# `cross` picks these up from the image environment (see its
# docker/Dockerfile.aarch64-unknown-linux-gnu).
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_gnu="--sysroot=/usr/aarch64-linux-gnu" \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_PATH="/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
