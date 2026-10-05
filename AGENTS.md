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
  (~1s when cached; first build can take minutes — timeout 900000ms)
- **Python tests** (unit + BDD, parallel): `uv run pytest -n auto`
  (~5s; timeout 300000ms)
- **Single Rust test**: `cargo test -p screensight <name>`
- **Single Python test**: `uv run pytest -q custom_components/... path::test_name`
- **Everything**: `make lint` and `make test`

Last measured: 2026-10-05 — Rust 52 passed (~0.7s; 47 unit + 1 WGSL validation + 4 WS integration), Python 31 passed (~2s).
