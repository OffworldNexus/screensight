# How to make a GPU-accelerated UI on Raspberry Pi with Rust

Field notes from getting **GPUI** (Zed's GPU UI framework) to render smoothly on
a Raspberry Pi 4's DSI panel: what actually works, what silently breaks, and how
to diagnose it. Written to be reusable for any Rust GPU UI stack (GPUI, wgpu,
iced, bevy, or raw Vulkan/GLES).

---

## 0. TL;DR

1. **Update the OS first.** A Raspberry Pi 4 on an old kernel/Mesa presents
   *all* GPU content as green garbage — even `vkcube`. On this machine, kernel
   `6.12.62` + Mesa `25.0.7` were broken; `6.18.50` + Mesa `26.2.2` work. This
   is the single most important lesson.
2. **Verify the baseline with `vkcube`.** If `vkcube` is corrupt, it's the
   driver/kernel, not your code. Stop debugging your app.
3. **Present through Wayland with a single-app compositor (`cage`)**, not X11.
   X11 direct-present tore with a fixed horizontal line; Wayland/cage was
   tear-free and can *direct scan-out* the client (Kodi-style, no extra copy).
4. **Cross-compile with `cross` + a custom Docker image based on Debian trixie**
   (matching the Pi). `cross`'s stock image is Ubuntu 16.04 and too old.
5. **Don't trust `scrot`/`XGetImage`** on this platform — it captures garbage
   for GPU-presented content. Use your eyes, or DRM capture.
6. Expect two *library-level* landmines: an `xattr`/`ENOATTR` aarch64 build
   break, and GPUI's glyph-atlas text path hanging V3D.

---

## 1. Know the hardware and where the GPU lives

On a Pi 4 (BCM2711, VideoCore VI), DRM is split across two devices:

```
/dev/dri/by-path/platform-fec00000.v3d-card    -> card0   # V3D render node
/dev/dri/by-path/platform-gpu-card             -> card1   # vc4 display controller (KMS/DSI)
/dev/dri/renderD128                                       # render node (V3D)
```

* `card1` owns the connectors/modes (`/sys/class/drm/card1-DSI-1/modes`).
* The Vulkan device is **V3D**, driver **V3DV** (`libvulkan_broadcom.so`).
* GLES is also V3D (`libgl1-mesa-dri`).

Handy checks:

```sh
cat /proc/device-tree/model
cat /sys/class/drm/card*-*/status        # which connector is connected
head -1 /sys/class/drm/card*-DSI-1/modes # 800x480
vulkaninfo --summary                     # device name, driver, apiVersion
```

> Gotcha: `kmscube` / many DRM tools default to `card0` (the render-only node)
> and show nothing. Pass `kmscube -D /dev/dri/card1`.

---

## 2. The upgrade that fixes everything

### Symptom

Every GPU app presents **green/black blocky garbage**. GPUI showed it, and so
did the known-good `vkcube`. `dmesg` repeated:

```
v3d fec00000.v3d: [drm:v3d_reset [v3d]] *ERROR* Resetting GPU for hang.
v3d fec00000.v3d: [drm:v3d_reset [v3d]] *ERROR* V3D_ERR_STAT: 0x00001000
```

`V3D_ERR_STAT: 0x1000` is bit 12 ("IDENT1 register error"). This is a V3D
kernel/driver bug family (there are several Raspberry Pi issues about it), not
application code.

### Fix (stable channel)

```sh
sudo apt-get update
sudo apt-get -y full-upgrade      # kernel 6.18.50 + Mesa 26.2.2 in this case
sudo reboot
```

Confirm afterwards:

```sh
uname -r                          # 6.18.50+rpt-rpi-v8
dpkg -l mesa-vulkan-drivers | awk '{print $3}'   # 26.2.2
vulkaninfo --summary | grep apiVersion
```

After this, `vkcube` renders a clean cube. **Build the UI after this step.**

---

## 3. Diagnosing "is it the GPU or my code?"

Ordered from cheapest to most specific:

1. **`vkcube`** (`vulkan-tools`). Known-good Vulkan triangle-cube with input.
   `vkcube --wsi xcb` (X11) or `vkcube --wsi wayland`. If this is corrupt, stop.
2. **`vulkaninfo --summary`** — confirms the ICD actually loaded V3DV.
3. **`dmesg | grep -i v3d`** — any `Resetting GPU for hang` means driver-level
   faults. Correlate timestamps with when your app is running.
4. **Switch GPU vs CPU**: force the software Vulkan driver to prove your app's
   logic is fine:
   ```sh
   VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json <app>
   ```
   (ICDs live in `/usr/share/vulkan/icd.d/`.) If it renders correctly under
   `lvp` (lavapipe) but not on V3D, it's a V3D interaction.
5. **`libinput list-devices`** for input problems (touch vs pointer).
6. **`drm_info`** (if installed) to see scanout formats/modifiers.

### Screenshots lie

`scrot` uses `XGetImage`, which reads the X *front buffer*. For GPU-composited
content on vc4 this returned pure noise **even for `vkcube`**. Do not conclude
your renderer is broken from a black/noisy screenshot. Use the physical panel,
or capture the DRM framebuffer, or render to a texture and read it back.

---

## 4. X11 vs Wayland vs straight DRM/KMS

### X11

* Direct rendering works via DRI3. **Tearing appeared as a fixed horizontal
  line (~20% from the top) with the full-screen animated app.** Adding
  `xcompmgr` did not fix it.
* GPUI's X11 backend only starts its display-refresh loop once the window is
  *mapped and not fully obscured*. With no window manager the window may not be
  considered visible, so nothing animates (0% CPU). Run a WM (e.g.
  `matchbox-window-manager -use_titlebar no`).
* Don't pass `-logfile` to `xinit` on this setup — it made `xinit` exit
  immediately with status 1, before Xorg even started.

### Wayland

* Tear-free out of the box.
* A single-application kiosk compositor is ideal:
  ```
  cage -- /path/to/app
  ```
  Fullscreen, no desktop, and wlroots will **direct scan-out** the client buffer
  when it's directly displayable (no intermediate copy). This is the practical
  "Kodi-style" target for a Wayland client.
* Compositors need DRM master via `libseat`/logind. In a headless/SSH context
  while another session owns `tty1`, logind refuses the seat. Running the
  compositor **as root** (built-in seatd backend) works for a PoC:
  `libseat … Could not open target tty: Permission denied` is the tell.

### Straight DRM/KMS

That's what Kodi does (EGL + GBM + `drmModePageFlip`, no compositor at all) and
it is the lowest-latency path. There is no mainstream Rust UI framework that
renders to KMS directly; on Wayland, compositor direct-scanout is the next best
thing.

### Present mode

Prefer **`FIFO`** (vsync). `MAILBOX`/`IMMEDIATE` invite tearing; on the broken
driver stack they also correlated with garbage. Force it in your renderer if it
defaults to mailbox.

---

## 5. Cross-compiling Rust to the Pi (from x86_64)

Goal: write on the dev machine, build for `aarch64-unknown-linux-gnu`, copy the
binary, run on the Pi.

### Why `cross` + a custom image

[`cross`](https://github.com/cross-rs/cross) runs the build in Docker with the
target's C toolchain and multiarch dev libraries. But:

* `cross`'s stock `aarch64-unknown-linux-gnu` image is **Ubuntu 16.04**. Too old.
* Native C libraries are the hard part: a GPU UI stack links `libxkbcommon`,
  `libfontconfig`, `libfreetype`, `libxcb`, sometimes `libwayland`, and may
  `dlopen` `libvulkan`.
* `x11rb`'s `xcb_ffi` backend (used to hand an XCB connection to Vulkan WSI)
  needs **`xcb_send_request_with_fds64`**, which only exists in **libxcb ≥ 1.16**.
  Ubuntu 16.04 ships 1.11 → undefined reference at link time.

Solution: a custom image based on **Debian trixie** (the Pi's own distro), which
gives a glibc/ABI and lib versions that match or predate the target:

```dockerfile
# deploy/cross-image/Dockerfile
FROM rust:1.98-trixie
RUN dpkg --add-architecture arm64 && apt-get update && apt-get install -y --no-install-recommends \
      gcc-aarch64-linux-gnu g++-aarch64-linux-gnu pkg-config \
      libxkbcommon-dev:arm64 libxkbcommon-x11-dev:arm64 \
      libfontconfig1-dev:arm64 libfreetype-dev:arm64 libexpat1-dev:arm64 \
      libwayland-dev:arm64 libvulkan-dev:arm64 libssl-dev:arm64 \
      libxcb1-dev:arm64 libxcb-xkb-dev:arm64 \
 && rm -rf /var/lib/apt/lists/*
# cross's stock images preinstall the target std; a custom image must too.
RUN rustup target add --toolchain 1.98.1 aarch64-unknown-linux-gnu
# `cross` reads these from the image environment.
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_PATH="/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
```

```toml
# Cross.toml
[target.aarch64-unknown-linux-gnu]
image = "screensight-cross-aarch64:latest"
```

```sh
docker build -t screensight-cross-aarch64:latest deploy/cross-image
CARGO_TARGET_DIR=/tmp/target cross build --release --target aarch64-unknown-linux-gnu
```

`cross` relies on the *image's* `ENV` for the linker, compiler, and pkg-config
paths — a from-scratch image must set them (as above).

### Notes

* **glibc is forward-compatible.** Building against trixie on the Pi or
  bookworm in a container is fine; just don't build against a *newer* glibc than
  the target.
* Pin the toolchain with `rust-toolchain.toml` so the container and host agree.
* Keep `CARGO_TARGET_DIR` off the (often full) root filesystem.

### Landmine: `xattr 0.2.3` on aarch64

Transitively pulled in by `gpui_http_client -> zed-async-tar`. It references
`libc::ENOATTR`, which does not exist on Linux/aarch64:

```
error[E0425]: cannot find value `ENOATTR` in crate `::libc`
```

Fix: patch it in a fork and map `ENOATTR` to the Linux equivalent `ENODATA`,
then pin that fork:

```toml
[patch.crates-io]
xattr = { git = "https://github.com/OffworldNexus/xattr", rev = "<rev>" }
```

(The tar/xattr code path is never exercised; it only has to compile.)

---

## 6. GPUI-specific notes

The published crate is **`gpui = 0.2.2`** (2025-10); it uses the `blade-graphics`
Vulkan renderer on Linux. (Zed's `main` moved to a wgpu renderer, but that is not
the crates.io release at time of writing.)

### Minimal setup

```toml
gpui = { version = "0.2.2", default-features = false, features = ["wayland", "x11"] }
```

Both windowing backends can be enabled; the platform picks based on
`WAYLAND_DISPLAY` / `DISPLAY`.

### Animation

* `impl Render for MyView { fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement }`
* `with_animation(id, Animation::new(dur).repeat().with_easing(e), |el, delta| …)`
  re-requests a frame every layout while repeating → an infinite frame pump.
* Wrap a `canvas(..)` in one `with_animation` to drive a fully procedural,
  60 fps scene:

  ```rust
  canvas(|_, _, _| {}, move |bounds, _, window, _| { /* paint_quad(...) */ })
      .size_full()
      .with_animation("pump", Animation::new(Duration::from_secs(3600)).repeat(), |el, _| el)
  ```
* `window.paint_quad(quad(bounds, corner_radii, bg, border_widths, border_color, style))`
  is the low-level primitive; there is no general per-element CSS transform for
  `div`s (SVG has `Transformation`).

### Gotchas

* **Text hangs V3D.** GPUI's glyph-atlas / sprite text path drove `V3D_ERR_STAT
  0x1000` hangs and green garbage on Mesa 26.2.2 (independent of font size,
  content, and MSAA via `ZED_PATH_SAMPLE_COUNT`). The fix is to stay off the
  glyph-atlas path entirely: rasterise the whole frame on the CPU with
  cosmic-text/swash and blit it as a GPUI `RenderImage`.
* **No touch.** GPUI 0.2.2 has no Wayland `wl_touch` and no X11 touch events.
  Read the touchscreen device yourself (next section).
* `ZED_PATH_SAMPLE_COUNT=1` disables the path MSAA intermediate if you suspect
  MSAA resolve issues.

---

## 7. Reading the touchscreen yourself (Kodi-style)

For a touch panel with no compositor-level touch support, read the evdev device
directly. On this panel the controller is an `ft5x06`, reporting absolute
coordinates matching the display (e.g. x∈[0..799], y∈[0..479]).

Recipe (see `src/touch.rs` for a full implementation):

* Scan `/dev/input/event*`; identify the touchscreen by name (`EVIOCGNAME`) or
  by the presence of `ABS_MT_POSITION_X/Y` via `EVIOCGABS`.
* Read the axis range with `EVIOCGABS(ABS_MT_POSITION_X/Y)` (fall back to
  `ABS_X/Y`); normalize to `[0, 1]`.
* Parse 24-byte `struct input_event` records:
  `{ i64 tv_sec, i64 tv_usec; u16 type; u16 code; i32 value }`.
* Track `ABS_MT_TRACKING_ID` (`>= 0` = touching) / `BTN_TOUCH`, and act on
  `SYN_REPORT`. Emit immediately on touch-down, then throttle drag samples
  (~40 ms) for a trail.
* Feed samples to the UI through an `Arc<Mutex<Vec<…>>>` drained each frame.

ioctl numbers are `_IOC`-encoded; e.g. `EVIOCGABS(abs)` is
`_IOC(READ, 'E', 0x40 + abs, sizeof(input_absinfo))` where `input_absinfo` is six
`i32`s. Constants: `EV_SYN=0`, `EV_KEY=1`, `EV_ABS=3`, `ABS_X=0`, `ABS_Y=1`,
`ABS_MT_POSITION_X=0x35`, `ABS_MT_POSITION_Y=0x36`, `ABS_MT_TRACKING_ID=0x39`,
`BTN_TOUCH=0x14a`, `SYN_REPORT=0`.

Permission: read `/dev/input/event*` as root or as a member of the `input`
group.

Bonus: with cage, run the compositor with a **transparent xcursor theme**
(`XCURSOR_THEME=blank`, `XCURSOR_PATH=…`) so a touch panel doesn't show a mouse
cursor — the HDMI-CEC virtual devices advertise pointer capability and would
otherwise cause one to be drawn.

---

## 8. Performance notes

* V3D is a **tile-based (TBDR)** GPU; avoid unnecessary full-screen clears and
  redundant render passes. GPUI batches primitives, so thousands of small quads
  are fine; the cost is mostly CPU-side scene building.
* The fastest path is **direct scan-out** (no compositor copy). On Wayland,
  `cage`/wlroots will do this for a fullscreen client when the buffer is
  directly displayable. Keeping a compositor out of the copy path matters a lot
  on a Pi.
* `FIFO` present avoids tearing; expect ~60 fps at 800×480 with this scene.
* Measure CPU to distinguish real rendering from a stalled loop: a working
  60 fps scene shows meaningful CPU jiffies; a window that never gets its
  refresh loop running sits at ~0%.

---

## 9. Checklist (copy/paste)

```sh
# --- Pi prerequisites ---
sudo apt-get update && sudo apt-get -y full-upgrade && sudo reboot
uname -r                                   # expect >= 6.18
dpkg -l mesa-vulkan-drivers | awk '{print $3}'   # expect >= 26.2
vkcube --wsi wayland                       # known-good baseline; must be clean
sudo apt-get install -y cage                # single-app Wayland kiosk compositor

# --- dev machine (x86_64) ---
docker build -t my-cross-aarch64:latest deploy/cross-image
cargo install cross --locked
CARGO_TARGET_DIR=/tmp/target cross build --release --target aarch64-unknown-linux-gnu

# --- run on the Pi ---
scp target/aarch64-unknown-linux-gnu/release/app pi:~/app
systemctl stop kiosk.service               # free the display
cage -- ~/app                              # fullscreen, tear-free, direct scan-out
```

---

## 10. Error-message index

| Message | Meaning / fix |
|---|---|
| `V3D_ERR_STAT: 0x00001000` + `Resetting GPU for hang` | V3D driver bug; update kernel/Mesa |
| green/black garbage on panel, even `vkcube` | same; update before blaming your code |
| `undefined reference to xcb_send_request_with_fds64` | link against libxcb ≥ 1.16 (use Debian trixie, not Ubuntu 16.04) |
| `cannot find value ENOATTR in crate ::libc` | aarch64 has no `ENOATTR`; map to `ENODATA` |
| `xinit: ... status=1` with no Xorg log | drop `-logfile`; ensure the client script is executable |
| GPUI window animates at 0% CPU / black | no WM → refresh loop never starts; run `matchbox` |
| `libseat … Could not open target tty: Permission denied` | run the compositor as root, or in an active seat |
| `scrot` shows noise on a working GPU app | `XGetImage` can't read GPU front buffers; ignore it |
| GPUI text → green garbage / hangs | glyph-atlas path; render text as quads yourself |

---

*Verified on: Raspberry Pi 4 Model B, 800×480 DSI touch panel, Debian 13
"trixie" aarch64, kernel 6.18.50, Mesa 26.2.2, GPUI 0.2.2, cage 0.3.1.*
