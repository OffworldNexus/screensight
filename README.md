# Screensight

A Home Assistant companion display for the Raspberry Pi. A small Rust daemon
drives the Pi's 800×480 touch panel, advertises itself over mDNS, pairs with
one or more Home Assistant instances using a code shown on the screen, and keeps
a heartbeat WebSocket link so Home Assistant can set the on-screen text.

This repository contains two components:

| Component | Path | Language |
| --- | --- | --- |
| Device daemon + control CLI | `device/` | Rust (GPUI/V3D) |
| Home Assistant custom integration | `custom_components/screensight/` | Python (HACS) |

Design source of truth: the **Screensight · Brand & Interface** Figma design
system, mirrored locally under [`docs/brand/`](docs/brand/) (palette, type ramp,
all 800×480 device screens, bundled fonts).

---

## How it works

```
┌───────────────────────────┐        mDNS  _screensight._tcp        ┌──────────────────────┐
│  Raspberry Pi 4           │  ◄───────────────────────────────►   │  Home Assistant       │
│  cage (Wayland kiosk)     │                                      │  custom_components/   │
│   └─ screensightd         │        WebSocket  /ws  (Bearer)      │   screensight         │
│        • identity/store   │  ◄───────────────────────────────►   │   • zeroconf config   │
│        • WS server        │        heartbeat + set_value         │     flow              │
│        • mDNS (Avahi)     │                                      │   • connection mgr    │
│        • CPU rasteriser   │                                      │   • text entity       │
│  screensight (CLI)        │                                      └──────────────────────┘
└───────────────────────────┘
```

* **Discovery** — the device publishes `_screensight._tcp` via Avahi with TXT
  keys `id`, `model`, `api` and `version` (plus `pairing=1` only while the
  pairing window is open). Home Assistant discovers it with no manual IP entry.
* **Pairing** — the panel draws a 6-digit code. The user types it into the Home
  Assistant config flow; the code is verified *on the device*. The panel then
  names the Home Assistant asking to pair and the user approves on the touch
  panel. Only then is a long-lived token issued over the same WebSocket. The
  code is never advertised over mDNS.
* **Multi-pair** — the device can be paired with several Home Assistant
  instances at once (for example a dev and a prod instance). Each keeps its own
  display state; only the selected one is shown. `screensight select <id>`
  switches which instance drives the panel.
* **Link** — a persistent WebSocket with an application-level heartbeat. Home
  Assistant pushes per-key dashboard values (`set_value`, or a full `set_state`
  on reconnect); the device stores them per paired instance and renders the
  selected one. Components subscribe to the keys they need (see `state.rs`).

The renderer CPU-rasterises the whole 800×480 frame (fonts, Unicode shaping,
emoji fallback) and blits it through GPUI as a single image. This deliberately
avoids GPUI's glyph-atlas text layer, which hangs the Pi's V3D GPU.

---

## Repository layout

```
device/                     Rust device daemon + `screensight` CLI
  src/                      db (SeaORM), state, pairing, runtime, WS server,
                            control socket, mDNS, rasteriser, GPUI panel
  assets/fonts/             bundled Silkscreen / VT323 / JetBrains Mono + fallbacks
custom_components/screensight/   Home Assistant integration (HACS-native)
tests/                      Home Assistant unit + BDD tests
deploy/
  systemd/                  cage.service, screensight.socket, screensightd.service
  deb/                      .deb packaging for Raspberry Pi OS (aarch64)
  cross-image/              custom `cross` Docker image (Pi toolchain)
  cursor-theme/             transparent cursor theme
scripts/                    ha-dev.sh, emulate-device.sh, fake-ha.py
docs/brand/                 design system reference (fonts, screen renders)
```

Git flow: `main` is production, `develop` is the integration branch; features
live on `feature/*`.

---

## Building the device

The daemon and CLI build and test on any machine with no GPU and no system
graphics libraries, because GPUI is an optional `gui` feature:

```sh
make device            # headless daemon + CLI (host)
make test-device       # Rust unit + integration tests
make device-gui        # full renderer (needs libxkbcommon, freetype, wayland/xcb)
make check-gui         # type-check the renderer without linking
```

Cross-compile for the Pi (aarch64):

```sh
docker build -t screensight-cross-aarch64:latest deploy/cross-image
make cross             # cross build --release --features gui
```

### Package for Raspberry Pi OS

```sh
make deb                       # builds screensight_<version>_arm64.deb
PI=remy@172.27.1.58 make deploy   # build + install over SSH
```

The `.deb` installs `screensightd`, the `screensight` CLI, the systemd units
and the transparent cursor theme, and depends on `cage` and `avahi-daemon`.
On install it starts:

* `cage.service` — the idle Wayland kiosk compositor;
* `screensight.socket` — the control socket, passed to the daemon as fd 3;
* `screensightd.service` — the daemon, which opens its window on cage.

To hand the panel back to the old Home Assistant kiosk:

```sh
./deploy/restore-kiosk.sh
```

### Control CLI

The panel has no physical buttons, so everything is managed over SSH through the
control socket:

```sh
screensight status              # identity, pairing window, paired instances
screensight pair                # open/re-arm the pairing window (prints the code)
screensight cancel-pair         # close the window without pairing
screensight unpair <id>         # forget one instance
screensight unpair --all        # forget every instance
screensight select <id>         # choose which instance drives the display
```

---

## Home Assistant integration

Install it as a HACS custom repository (`hacs.json` at the repo root), or copy
`custom_components/screensight/` into your Home Assistant `config` directory.

* Discovery and setup are automatic: the code form appears when the device is in
  pairing mode.
* The device exposes a **Display text** `text` entity; setting it updates the
  panel.
* The connection manager re-resolves the device by its mDNS name across IP
  changes, so DHCP changes do not require re-pairing.
* Supports `en`, `fr`, `de`, `es`, `it`, `pt`, `nl` and every other locale Home
  Assistant ships (see `custom_components/screensight/translations/`).

### Testing the Home Assistant side locally (recommended)

**Never test against production.** Two throwaway pieces on the same machine give
you the whole loop without a Raspberry Pi:

```sh
# Terminal 1 — a disposable Home Assistant Core (started in the background and
# onboarded automatically; there is no setup wizard)
make ha-dev            # start + auto-onboard; UI at http://localhost:8123
make ha-dev-logs       # follow logs (Ctrl+C just stops following)
make ha-dev-stop       # stop and remove the container
make ha-dev-reset      # wipe config and start fresh (use if login ever fails)

# Terminal 2 — the device daemon + panel on your desktop ("emulated Pi")
make emulate           # or ./scripts/emulate-device.sh
```

`make ha-dev` runs `ghcr.io/home-assistant/home-assistant:stable` with
`network_mode: host` — required so zeroconf can see the device — binds the
integration and a scratch `configuration.yaml` under `.dev/`, and completes
onboarding over HA's own API. Log in with **`dev` / `dev`**. Then open
**Settings → Devices & Services → Add Integration → Screensight**, read the code
off the emulated panel and confirm on it.

The container is managed through `docker compose` and runs detached, so it never
traps your shell; `Ctrl+C` during `make ha-dev-logs` only stops the log stream.

To drive the device without Home Assistant at all:

```sh
uv run --no-project scripts/fake-ha.py pair --code <code>     # prints the token after you confirm
uv run --no-project scripts/fake-ha.py text --token <token> "hello from the CLI"
```

For a fully offline device-only run:

```sh
make run-headless
SCREENSIGHT_CONTROL_SOCKET=/tmp/opencode/screensight-state/control.sock ./target/debug/screensight status
```

### Running against the real Pi

Once the PoC is validated locally, install the `.deb` on the Pi
(`make deploy`) and pair your production Home Assistant from the panel. The dev
instance can stay paired at the same time; use `screensight select` to choose
which one is displayed.

---

## Tests and CI

```sh
make test        # Rust + Home Assistant
make lint        # cargo fmt/clippy + ruff/mypy

make test-device # cargo test -p screensight
make ha-test     # uv run pytest  (unit + BDD, Allure results in allure-results/)
make ha-bdd      # only the @bdd scenarios
```

* Rust: unit tests across identity, the SQLite store, the state manager,
  pairing/rate limiting and the runtime, plus WebSocket end-to-end tests
  (pairing → approval → token → `set_value`) in `device/tests/`.
* Python: config-flow, connection and text-entity tests, plus pytest-bdd
  scenarios with Allure reporting.
* GitHub Actions: `.github/workflows/rust.yml` (fmt, clippy, tests, release
  build with the `gui` feature) and `.github/workflows/python.yml` (ruff, mypy,
  pytest, Allure artifact).

---

## Hardware notes

Built for a Raspberry Pi 4 Model B with the 800×480 DSI touch panel, on
Raspberry Pi OS / Debian 13 "trixie" (aarch64).

**The kernel and Mesa must be current.** On kernel `6.12.62` + Mesa `25.0.7`
every GPU app presented green/black garbage; a full system upgrade to kernel
≥ `6.18.50` + Mesa ≥ `26.2.2` fixes it:

```sh
sudo apt-get update && sudo apt-get -y full-upgrade && sudo reboot
```

The device runs `cage` as root because `libseat`/logind will not hand DRM master
to a transient session while another session owns `tty1`. Touch is read directly
from the `ft5x06` evdev device (`device/src/touch.rs`) because GPUI 0.2.2 has no
Wayland touch support.

The patched `xattr`, `blade-graphics` and `gpui` crates live in rev-pinned forks
under the `OffworldNexus` org (each README documents its delta) and are wired in
at the Cargo workspace root via `[patch.crates-io]`.
