# Bee-eater mascot artwork

Hand-authored in the reference bitmap's 1024 × 1024 coordinate space. No
auto-traced paths, embedded bitmaps, gradients, filters, or strokes are used.
The full mascot has **13 paths and 3 circle primitives**. Disconnected areas
with the same colour share named compound paths (perch/feet, neck/tail light,
forehead/cheek).

## Deliverables and review

- [Full mascot](mascot.svg): transparent, `viewBox="0 0 1024 1024"`.
- [Head mark](mascot-head.svg): ten original head-related shapes, clipped at a
  clean neck/gorget boundary; `viewBox="495 180 332 190"`. The geometry remains
  in the full mascot's coordinates.
- [Ink cutout](mascot-head-ink.svg) / [paper cutout](mascot-head-paper.svg): one
  compound path each, with an even-odd transparent eye hole. Every fill is
  `#141110` / `#faf7f2`, respectively.
- [Overlay comparison](mascot-compare.png): reference | true 50% overlay |
  flat artwork on white.
- [Side-by-side](mascot-side-by-side.png): reference and clean flat artwork.
- [Local brand sheet](screensight-brand-proposal.html) and its
  [render](screensight-brand-proposal.png) use the same geometry inline.

The reference's soft breast shading is deliberately collapsed to one gold
plane. The neck/tail light uses sampled `#26a7a3` to retain the bitmap's muted
teal rather than forcing the more saturated brand accent. The eye is centred
at `(638, 242)`, as measured from the bitmap; the source brief's position was
approximate. No rust tail-base patch is present in the actual bitmap.

The comparison harness originally composited an opaque white image **over**
the reference before blending. This was corrected for the saved comparison;
the middle panel actually contains both images.

## Path inventory

“Anchors” counts explicit move/line/Bézier endpoints, including the initial
point of each subpath, but not `Z` closures or Bézier control handles. Circles
remain native primitives and have no authored path nodes.

| SVG shape ID | Anchors | Bézier segments |
| --- | ---: | ---: |
| `perch-and-feet` | 22 | 13 |
| `tail` | 5 | 2 |
| `body-neck-and-tail-light` | 14 | 5 |
| `underparts` | 11 | 7 |
| `folded-wing` | 8 | 4 |
| `wing-light` | 5 | 2 |
| `wing-underedge` | 5 | 2 |
| `crown-nape` | 7 | 3 |
| `forehead-and-cheek` | 12 | 7 |
| `bill` | 5 | 2 |
| `bill-ridge` | 4 | 2 |
| `eye-mask` | 11 | 7 |
| `gorget` | 4 | 2 |
| `iris` | circle | — |
| `pupil` | circle | — |
| `eye-glint` | circle | — |

## Figma sync

Updated in [Screensight · Brand & Interface](https://www.figma.com/design/2zZzaw783NEwWqaOwSpLyU):

- Identity hero `3:2`: `mascot / full` (`12:117`), `mark / head` (`12:134`).
- Marks frame `6:2`: dark/light mascot specimens; head at 16, 24, 32, 48,
  and 64 px; paper, ink, and ink-on-gorget cutouts.
- SVGs imported as editable vectors, preserving aspect ratio and existing
  specimen placement. No reference bitmap is embedded in Figma.

## Typography follow-up

Typography was intentionally unchanged during the mascot-only task. The
subsequent [device UI and typography concept](screensight-ui-concept.md) now
replaces page **04 · Typography & feel** with embedded open-font specimens.
VT323 / JetBrains Mono / Silkscreen are proposed, with Doto, Space Mono and
Press Start 2P alternatives. Final hardware rasterisation and font selection
remain to be confirmed.
