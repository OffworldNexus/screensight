# Bundled fonts

These fonts are embedded into the `screensight` binary with `include_bytes!`
(see `device/src/raster.rs`). The rasteriser never reads system fonts or uses
fontconfig, so the device renders identically regardless of the host.

The database is loaded in this order and the text fallback chain is:

1. `Silkscreen` — design face, SIGNAL/kicker labels (12 px)
2. `VT323` — design face, GLANCE headings and numbers
3. `JetBrains Mono` — design face, READ body and metadata
4. `Noto Sans` — Unicode body coverage
5. `Noto Sans Symbols2` — symbol coverage
6. `Noto Emoji` — monochrome emoji coverage

Any codepoint none of these covers is rendered with the requested font's
U+FFFD replacement glyph, or a visible tofu box, rather than being dropped.

## Files and provenance

| File | Source | License |
| --- | --- | --- |
| `Silkscreen-Regular.ttf` | `docs/brand/fonts/Silkscreen-Regular.ttf` | `silkscreen-OFL.txt` (SIL OFL 1.1) |
| `VT323-Regular.ttf` | `docs/brand/fonts/VT323-Regular.ttf` | `vt323-OFL.txt` (SIL OFL 1.1) |
| `JetBrainsMono-Regular.ttf` | `docs/brand/fonts/JetBrainsMono[wght].ttf`, renamed | `jetbrainsmono-OFL.txt` (SIL OFL 1.1) |
| `NotoSans-Regular.ttf` | `/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf` (Debian `fonts-noto-core`) | `noto-OFL.txt` (SIL OFL 1.1) |
| `NotoSansSymbols2-Regular.ttf` | `/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf` (Debian `fonts-noto-core`) | `noto-OFL.txt` (SIL OFL 1.1) |
| `NotoEmoji-Regular.ttf` | Google Fonts static build (`https://fonts.gstatic.com/s/notoemoji/v47/...ttf`) | `noto-OFL.txt` (SIL OFL 1.1) |

`JetBrainsMono-Regular.ttf` is the upstream variable font (`wght` axis, default
instance 400). cosmic-text's fallback matches faces by exact weight, so all
roles are shaped at the regular instance and "bold" is faux-emboldened at
raster time by overdrawing a one-pixel-shifted copy.

The Noto fonts are redistributed under the SIL Open Font License 1.1; the full
license text is in `noto-OFL.txt` (from the Noto project) and is the same
license shipped with `fonts-noto-core`.
