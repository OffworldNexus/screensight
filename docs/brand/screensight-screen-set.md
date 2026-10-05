# Screensight device screen set

## Latest revision: Act now replaces corner alerts

Snooze closes an Immediate/Critical pop-in and retains its card in **Act now**;
tap that card to reopen it. There are no duplicate corner indicators. Discard
removes the occurrence from both until clear → new occurrence.

**Act now** and **Maintenance** disappear individually when empty. The remaining
section expands to the full right-column height. With neither populated, the
current placeholder proposal is the bee-eater and **3615 SCREENSIGHT / Rien à
signaler**. Unknown/unavailable data is not empty. Revised states are
`immediate-snoozed`, `critical-snoozed`, `alerts-multiple`, `actions-only` and
`both-empty`; the gallery now contains **69** compositions. The historical
corner wording below is superseded by this revision.

The current review entry point is [the complete gallery](screensight-screen-set.html).
Each device scene is **800 × 480**, with no device scrolling. The gallery itself
is a review document and may scroll. Use `?screen=overview` (or another scene ID)
for a single native-size view. Text, keys and graphs are editable in Figma on
**06 · Device UI**. These are design proposals with illustrative data, not live
Home Assistant integrations.

## Latest decisions

- **Maintenance:** routine, deferrable tasks, with evening/next-occurrence
  reminder choices. Actionable tasks also have a deliberate start action.
- **Immediate:** act now, but no physical-harm implication. High CO₂ and this
  printer's user-confirmed **42°C PLA clogging point** belong here.
- **Critical:** physical danger, such as a fire or dedicated toxic-gas alarm.
  Full-screen crown treatment; generic VOC indices are not toxic-gas alarms.
- Both alert classes have **Snooze** (persistent corner/reopen), **Discard**
  (hide this occurrence until clear → new occurrence), and quieter **See more**
  (sensor graph or source event history). A confirmation is proposed for discard.
  Display controls never silence the physical source or resolve the condition.
- **Next rain** sits beside min/max and looks beyond the one-hour rain strip.
  The calm-day example shows rain at 16:10 with a dry next hour. No predicted
  rain and unavailable forecast are separate states.
- No “Linked to Home Assistant” footer on the device dashboard.
- Buttons inherit Tunnel Weaver's **Minitel keycaps**: wide/short, approximately
  3 px corners, cut-away top-right, lower travel lip, short mechanical movement.
- The light slider is an **instrument rail**: recessed segmented illumination,
  notched cursor and calibrated ticks. Its input remains continuous 0–100%;
  24 visual segments do not imply 24 allowed values.

## State inventory

| Family | Designed states |
| --- | --- |
| Overview | Day, night, calm, offline/stale |
| Routine maintenance | Queue, next page, humidor/battery snooze, deferred confirmation |
| Immediate | Printer clogging, CO₂, VOC source check |
| Critical | Fire, dedicated toxic-air alarm |
| Alert lifecycle | Immediate/critical corner, multiple alerts, discard confirmation, discarded, resolved |
| Details | Printer temperature, CO₂, VOC, PM2.5 with outdoor context, fire/toxic source event history |
| Pairing | Unpaired instructions/example code, confirm home, success, timeout, help, no home discovered |
| Light | Off, full, dragging, pending, failed |
| Touch | Contact, release, reduced motion, motion-language storyboard |
| Executable update | Review/start, starting without percentage, progress in overview, success, failure/retry, detail |
| Weather empty/data states | No rain predicted, no location, loading, unavailable |
| Room empty/data states | No sensors, stale/unavailable, heating idle, heating unknown |
| Agenda empty/data states | No current meeting/next only, no meetings, not configured, unavailable |
| Maintenance empty/data states | No tasks, empty queue, unavailable rules |
| Other empty/data states | No configured light, no history samples, history unavailable, alarm source unavailable |

**67 designed compositions** are rendered under [screens](screens/).
The state variants are examples of a shared UI, not 67 separate production pages.

## Action progress

An executable update replaces its existing routine card with status/progress.
Show actual integration progress when provided; otherwise show an indeterminate
starting/running state. Never generate a fake percentage with a timer. Keep the
sensor powered; distinguish completion from missing confirmation. Expose cancel
only when the underlying integration genuinely supports it. Touch feedback
acknowledges interaction, not successful execution.

## Signal-cast touch expression

[Animated local demo](screensight-touch-demo.html): try the presets and drag the
rail. Proposed timings: contact/key travel in 0–80 ms, angular release wavefronts
and four cut-paper shards settling by 320 ms. No circular spell sigil. Drag uses
cursor feedback rather than an endless particle stream. Reduced motion uses a
brief static contact frame; no expansion or shards. The demo is local-only.

Critical instructions must remain readable; the implementation should constrain
effects to the touched control and budget rendering for the device hardware.

## Empty-state rules

Empty, not configured, loading and unavailable are not interchangeable. Retain
section placement, explain the condition and avoid drawing invented zero values
or graph lines. No meetings is a known empty calendar; a failed calendar cannot
claim that. No rain means none in the forecast horizon, not forever. Missing
alarm-source reports must never clear a last-known-active alarm.

Pairing is a proposed local UX, not a specified implemented protocol. The code
`724 196` is explicitly an example. Network provisioning, discovery, security,
code lifetime and the integration setup route need implementation validation.

## Review status

Local native-size renders were generated for all compositions. Representative
overview, alert, pairing, graph, update and motion compositions were visually
reviewed; Figma screenshots were also reviewed during assembly. Some Figma
empty variants are section-layout studies rather than fully wired prototypes.
The local gallery provides navigation examples; the standalone touch demo
provides actual interaction animation. Real device rasterisation, contrast,
touch usability and live state transitions remain to be validated.

The earlier [UI concept notes](screensight-ui-concept.md) document the first
draft; this page supersedes their alert hierarchy and controls.
