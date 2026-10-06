# Screensight — agent notes

Monorepo for the Screensight Raspberry Pi display: the Rust device lives in
`device/` (Cargo package `screensight`), the Home Assistant integration in
`custom_components/screensight/` (HACS-native), and Python is managed with `uv`.

## Testing

Fast, quiet commands. Always redirect output to a temp file and dump only
failures; pass an explicit timeout (≈2× the measured wall time).

- **Rust static** (fmt + clippy, all features): `make lint-device`
  (~cached; timeout 900000ms)
- **Python static** (ruff format check, ruff check, mypy): `make ha-lint`
  (~5s; timeout 120000ms)
- **Rust tests** (unit + WS integration): `cargo test -p screensight`
  (~11s with incremental build; first build can take minutes — timeout 900000ms)
- **Python tests** (unit + BDD, parallel): `uv run pytest -q -n auto`
  (~5s; timeout 300000ms)
- **Single Rust test**: `cargo test -p screensight <name>`
- **Single Python test**: `uv run pytest -q custom_components/... path::test_name`
- **Everything**: `make lint` and `make test`
- **Headless smoke** (XX pairing + IK reconnect): `timeout 120s ./scripts/smoke-test.sh`
  (~11s; timeout 180000ms)

Last measured: 2026-10-06 — Rust 65 passed (~11s; 59 unit + 1 WGSL validation + 5 WS integration), Python 36 passed (~4s), headless smoke passed (~11s).
