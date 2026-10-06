# Screensight — development & deployment

Technical reference for building, packaging and running Screensight. For what the
project is, see the [README](../README.md).

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

## Package for Raspberry Pi OS

```sh
make deb                       # builds screensight_<version>_arm64.deb
PI=remy@172.27.1.58 make deploy   # build + install over SSH
```

The `.deb` installs `screensightd`, the `screensight` CLI, the systemd units and
the transparent cursor theme, and depends on `cage` and `avahi-daemon`. On install
it enables and starts:

* `cage.service` — the idle Wayland kiosk compositor;
* `screensight.socket` — the control socket, passed to the daemon as fd 3;
* `screensightd.service` — the daemon, which opens its window on cage.

The units are `WantedBy=multi-user.target`, so the panel comes back by itself on
reboot (the Pi boots headless, with no desktop session). `screensightd` waits for
cage's Wayland socket before starting, so the first boot attempt does not race the
compositor. To hand the panel back to the old Home Assistant kiosk:

```sh
./deploy/restore-kiosk.sh
```

## Control CLI

The panel has no physical buttons, so everything is managed over SSH through the
control socket:

```sh
screensight status              # identity, pairing window, paired instances
screensight pair                # open/re-arm the pairing window
screensight cancel-pair         # close the window without pairing
screensight unpair <id>         # forget one instance
screensight unpair --all        # forget every instance
screensight select <id>         # choose which instance drives the display
```

## Home Assistant integration

Install it as a HACS custom repository (`hacs.json` at the repo root), or copy
`custom_components/screensight/` into your Home Assistant `config` directory.

* Discovery and setup are automatic: the code form appears when the device is in
  pairing mode.
* The device exposes a **Display text** `text` entity; setting it updates the panel.
* The connection manager re-resolves the device by its mDNS name across IP
  changes, so DHCP changes do not require re-pairing.
* Supports `en`, `fr`, `de`, `es`, `it`, `pt`, `nl` and every other locale Home
  Assistant ships (see `custom_components/screensight/translations/`).

### Testing the Home Assistant side locally (recommended)

Never test against production. Two throwaway pieces on the same machine give you
the whole loop without a Raspberry Pi:

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

To drive the device without Home Assistant at all:

```sh
uv run --no-project scripts/fake-ha.py pair                     # reads the SAS from the panel
uv run --no-project scripts/fake-ha.py text "hello from the CLI"
```

For a fully offline device-only run:

```sh
make run-headless
SCREENSIGHT_CONTROL_SOCKET=/tmp/opencode/screensight-state/control.sock ./target/debug/screensight status
```

## Under the hood

```
┌───────────────────────────┐        mDNS  _screensight._tcp        ┌──────────────────────┐
│  Raspberry Pi 4           │  ◄───────────────────────────────►   │  Home Assistant       │
│  cage (Wayland kiosk)     │                                      │  custom_components/   │
│   └─ screensightd         │        WebSocket  /ws  (Noise)       │   screensight         │
│        • identity/store   │  ◄───────────────────────────────►   │   • zeroconf config   │
│        • WS server        │        heartbeat + set_value         │     flow              │
│        • mDNS (Avahi)     │                                      │   • connection mgr    │
│        • CPU rasteriser   │                                      │   • text entity       │
│  screensight (CLI)        │                                      └──────────────────────┘
└───────────────────────────┘
```

* **Discovery** — the device publishes `_screensight._tcp` via Avahi with TXT
  keys `id`, `model`, `api=1`, `version` and its static public key (`key=<hex>`,
  public by definition), plus `pairing=1` only while the pairing window is open.
  Home Assistant discovers it with no manual IP entry.
* **Pairing** — the device and Home Assistant run **Noise XX** over the existing
  `/ws` WebSocket (binary frames), then each derives an 8-digit SAS from the
  handshake. The panel shows it; the user types it into the config flow and Home
  Assistant compares it *locally* — the code is never transmitted. On a match the
  user approves on the panel and the two sides store each other's static public key.
* **Link** — after pairing, Home Assistant reconnects with **Noise IK**, presenting
  its static key. The device authenticates the peer by that key and encrypts every
  application frame; a passive capture yields no plaintext and no reusable
  credential. An unknown key cannot send `set_value`/`set_state`.
* **Multi-pair** — the device can be paired with several Home Assistant instances
  at once (for example a dev and a prod instance). Each keeps its own display
  state; only the selected one is shown. `screensight select <id>` switches which
  instance drives the panel.
* **Heartbeat** — a persistent WebSocket with an application-level heartbeat. Home
  Assistant pushes per-key dashboard values (`set_value`, or a full `set_state` on
  reconnect); the device stores them per paired instance and renders the selected
  one.
* **Persistence** — SQLite schema changes use ordered `sea-orm-migration`
  migrations in `device/src/migration/`, starting with `m000001_create_tables.rs`.
  Startup applies pending migrations and records them in `seaql_migrations`. Add
  future migrations to the `Migrator` list; keep applied migration definitions
  unchanged rather than generating them from evolving runtime entities.

The renderer CPU-rasterises the whole 800×480 frame (fonts, Unicode shaping, emoji
fallback) and blits it through GPUI as a single image. This deliberately avoids
GPUI's glyph-atlas text layer, which hangs the Pi's V3D GPU.

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

Git flow: `main` is production, `develop` is the integration branch; features live
on `feature/*`.

## Tests & CI

```sh
make test        # Rust + Home Assistant
make lint        # cargo fmt/clippy + ruff/mypy

make test-device # cargo test -p screensight
make ha-test     # uv run pytest  (unit + BDD, Allure results in allure-results/)
make ha-bdd      # only the @bdd scenarios
```

* Rust: unit tests across identity, the SQLite store, the state manager,
  pairing/rate limiting, the Noise transport and the runtime, plus WebSocket
  end-to-end tests (XX pairing → SAS → approval → key-authenticated `set_value`)
  in `device/tests/`.
* Python: config-flow, Noise transport, connection and text-entity tests, plus
  pytest-bdd scenarios with Allure reporting.
* GitHub Actions: `.github/workflows/rust.yml` (fmt, clippy, tests, release build
  with the `gui` feature), `.github/workflows/python.yml` (ruff, mypy, pytest,
  Allure artifact) and `.github/workflows/release.yml` (builds the aarch64 `.deb`
  and attaches it to the GitHub Release on `v*` tags).

## Hardware notes

Built for a Raspberry Pi 4 Model B with the 800×480 DSI touch panel, on Raspberry
Pi OS / Debian 13 "trixie" (aarch64).

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
under the `OffworldNexus` org (each README documents its delta) and are wired in at
the Cargo workspace root via `[patch.crates-io]`.

## Brand

The design system source of truth is the **Screensight · Brand & Interface** Figma
system, mirrored under [`docs/brand/`](brand/) (palette, type ramp, all 800×480
device screens, bundled fonts). The mark is the European bee-eater
(`docs/brand/mascot.svg`); the README banner is `.github/brand/header-{light,dark}.svg`.
