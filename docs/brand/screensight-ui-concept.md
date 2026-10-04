# Screensight / the room terminal

> **Superseded design decisions:** see the [complete screen set](screensight-screen-set.md)
> for the current three-level alert model, corner snooze/discard semantics,
> next-rain field, Minitel instrument controls, update progress and empty states.
> The original draft below is retained as historical context, not the current specification.

A proposed device interface, not a production implementation. All displayed
values and thresholds are illustrative. Location, room, date and meeting names
are sample content, not inferred personal data.

## Review assets

- [Interactive overview](screensight-ui-concept.html): exact 800 × 480 px,
  locally bundled fonts, no scrolling. The brightness presets/slider and
  reminder overlays are demo interactions only; they do not call Home Assistant.
- [Overview PNG](screensight-ui-concept.png).
- [Snooze PNG](screensight-ui-snooze.png).
- [Queue PNG](screensight-ui-queue.png).
- [Typography sheet](screensight-typography.html) / [PNG](screensight-typography.png).
- [Figma overview](https://www.figma.com/design/2zZzaw783NEwWqaOwSpLyU?node-id=15-3)
  and [snooze sheet](https://www.figma.com/design/2zZzaw783NEwWqaOwSpLyU?node-id=15-145)
  on **06 · Device UI**, as editable text/shapes, not screenshots.
- [Figma typography](https://www.figma.com/design/2zZzaw783NEwWqaOwSpLyU?node-id=15-301)
  replaces the former platform-stack proposal on page 04.

## Visual hierarchy

1. **Time first.** A large VT323 clock anchors the top left. Date and the next
   sun transition are quieter. At night, replace sunset with sunrise time and
   countdown. The sun's intensity follows solar elevation, not cloud cover;
   use a moon/night symbol after sunset. The current mock shows the daytime state.
2. **Outside and inside remain distinct.** Weather occupies the middle of the
   top row: current temperature, day's low/high, condition and a one-hour
   minute-by-minute precipitation strip. Room temperature, humidity and heating
   state occupy the top-right card. Heating includes a target temperature.
3. **Meetings are temporal, not a calendar grid.** Show current start/end and
   time remaining with elapsed progress, then next start/end and countdown.
   With no current meeting, promote the next meeting; with none upcoming,
   show a calm “No more meetings today” state. Long titles truncate, with a
   tap-to-view fixed detail overlay. Overlapping meetings need a conflict state.
4. **Lighting is a permanent keyboard deck.** Off, full-on and a continuous
   slider stay visible. Gold indicates the active controllable value, not a
   warning. The value must reflect the Home Assistant virtual light's confirmed
   state; distinguish requested/pending/failed states rather than pretending a
   command was applied. The prototype changes a local demo value only.
5. **Actions have their own column.** Three stable slots are ranked by danger,
   urgency and then age. Each has a plain-language action, measured evidence,
   priority label and an individual later control. Colour supplements the words.
   Printer danger outranks ventilation, which outranks humidor maintenance.

An unlimited maintenance queue cannot literally all be readable at once on
800 × 480. The proposed compromise is **all key categories and top three
actions on the overview**, with a visible count and stacked edge for the rest.
The remaining queue opens in a fixed-size overlay, three per page, using
previous/next controls if needed—never scroll. The mock demonstrates six items.

## Touch and physical panel

Panel: **800 × 480 px / 154.08 × 85.92 mm**. The horizontal and vertical physical
pixel pitches differ slightly; evaluate at native resolution on the real panel.

- Light presets: 60–62 × 58 px, about 11.6–11.9 × 10.4 mm.
- Slider touch area: 230 × 60 px; the visible track is not the hit target.
- Individual reminder controls: 46 × 48 px, about 8.9 × 8.6 mm.
- Queue target: 330 × 44 px, about 63.6 × 7.9 mm.
- No hover-dependent controls, scrolling ticker, or continuously blinking clock.
- Critical action titles are 15 px; small evidence captions are secondary.
  Check readability, contrast, touch accuracy and font rasterisation on-device.

## Reminder semantics and rule design

**This evening:** suppress this reminder until a configurable local evening
time (18:00 in the example). If evening has passed, show an explicit next-day
time rather than a timestamp in the past.

**Next occurrence:** acknowledge the current rule occurrence; wait for the
condition to clear with hysteresis/debounce, then re-arm on a fresh breach.
Firmware or battery tasks need an explicit recurrence policy; they may not have
a meaningful new occurrence until the underlying issue changes.

**Danger is different:** the concept recommends acknowledgement rather than
removing an ongoing unsafe printer condition. Any suppression policy, escalation
and automation affecting equipment must be agreed separately. Sample `42°`
is not an approved PLA safety threshold.

Rules belong to the Home Assistant integration, not the display layout:

| Signal | Possible action | Required context |
| --- | --- | --- |
| CO₂ | Ventilate | Occupancy, sustained breach, outdoor air, hysteresis |
| PM2.5 | Filter or ventilate | Indoor **and outdoor** pollution; avoid opening into worse air |
| Sensor battery | Replace battery | Reliability, threshold persistence, device identity |
| Firmware | Schedule update | Compatibility, availability, impact; never automatic from this card |
| Printer enclosure | Inspect / cool enclosure | Material, printer state, sensor reliability, validated limits |
| Humidor humidity | Re-humidify | Target band, duration, sensor calibration |

Unavailable/stale values must say so. Unknown rain is not “no rain”; unknown
room sensors are not “healthy.” Offline controls must be visibly unavailable.
These are intended production states, not implemented in this illustrative mock.

## Embedded typography proposal

| Role | Recommendation | Alternatives |
| --- | --- | --- |
| Clock, countdowns, large readings | VT323 Regular | Doto Bold for more decorative dot-matrix character |
| Titles, controls, units, metadata | JetBrains Mono Regular / Semibold | Space Mono for a warmer, quirkier tone |
| Short section labels | Silkscreen Regular | Press Start 2P reserved for playful brand expression |

All six downloaded families carry **SIL OFL 1.1** licences. Original font files
and corresponding licence texts are in [fonts](fonts/). Local HTML uses these
bundled files; no CDN or network font lookup is required. Keep the OFL texts with
distributed firmware/fonts. The downloaded JetBrains Mono and Doto are variable
TTFs; an embedded renderer may require pinned/static instances or a glyph atlas.
This is a proposed set, not a final device font lock.

The existing Identity wordmark has not been re-typeset: this task updates page
04 and introduces the device UI, without silently changing the mascot or logo.
