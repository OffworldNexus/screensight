/* Design-only scene data. Every state is native 800 × 480; values are examples.
 * Shapes and text are kept separate so the same compositions remain editable
 * in Figma rather than being imported as flattened screenshots. */
const screensightScenes = [];
const colors = { ink: '#141110', panel: '#211c18', edge: '#514943', paper: '#faf7f2', muted: '#c0b4a4', teal: '#0bb2b3', gold: '#f2a900', rust: '#c9541f', danger: '#ff9270' };
function makeScene(id, title, note, background = colors.ink) {
  const s = { id, title, note, background, nodes: [], links: [] };
  screensightScenes.push(s); return s;
}
function shape(s, name, x, y, w, h, fill, r = 0) { s.nodes.push({ type: 'rect', name, x, y, w, h, fill, r }); }
/* The clipped key silhouette and lower lip inherit the sibling Minitel deck. */
function cut(s, name, x, y, w, h, fill, notch = 7) { s.nodes.push({ type: 'cut', name, x, y, w, h, fill, notch }); }
function copy(s, text, x, y, size = 14, fill = colors.paper, font = 'JetBrains Mono', weight = 400) { s.nodes.push({ type: 'text', name: text, text, x, y, size, fill, font, weight }); }
function tag(s, text, x, y, fill = colors.muted) { copy(s, text, x, y, 12, fill, 'Silkscreen'); }
function key(s, text, x, y, w, target, tone = 'quiet', h = 52) {
  const fill = tone === 'gold' ? colors.gold : tone === 'rust' ? colors.rust : colors.panel;
  cut(s, 'Key travel / ' + text, x, y + 3, w, h - 3, tone === 'gold' ? '#a86b00' : tone === 'rust' ? '#743010' : '#0b0908');
  cut(s, 'Key / ' + text, x, y, w, h - 3, fill);
  shape(s, 'Key upper highlight / ' + text, x + 3, y, w - 13, 1, tone === 'gold' ? '#ffcf79' : tone === 'rust' ? '#ff9270' : '#6b6158');
  copy(s, text, x + 12, y + (h - 17) / 2, 13, tone === 'gold' ? colors.ink : colors.paper);
  if (target) s.links.push({ label: text, x, y, w, h, target });
}
/* Angular wavefronts echo cut-paper planes and the Minitel notched keys.
 * These static frames specify motion; they are not success acknowledgements. */
function cast(s, x, y, phase, tone = colors.teal) {
  if (phase === 'reduced') { shape(s, 'Touch / static focus top', x - 30, y - 23, 60, 2, tone); shape(s, 'Touch / static focus bottom', x - 30, y + 23, 60, 2, tone); return; }
  const size = phase === 'press' ? 30 : phase === 'drag' ? 44 : 76;
  s.nodes.push({ type: 'wave', name: 'Touch / clipped wavefront', x: x - size / 2, y: y - size / 2, w: size, h: size, fill: tone, notch: 10 });
  if (phase === 'release') s.nodes.push({ type: 'wave', name: 'Touch / outer wavefront', x: x - 51, y: y - 51, w: 102, h: 102, fill: colors.paper, notch: 15 });
  for (let i = 0; i < 4; i++) { const dx = [-1, 1, 1, -1][i], dy = [-1, -1, 1, 1][i]; cut(s, 'Touch / cut-paper shard', x + dx * (size / 2 + 7) - 4, y + dy * (size / 2 + 7) - 2, 9, 4, i % 2 ? colors.gold : tone, 3); }
  shape(s, 'Touch / contact cross horizontal', x - 5, y, 10, 1, colors.paper); shape(s, 'Touch / contact cross vertical', x, y - 5, 1, 10, colors.paper);
}
/* Continuous 0–100% input with 24 visual segments, not 24 discrete settings. */
function lightDeck(s, value = 64, state = 'idle') {
  const unavailable = state === 'offline', pending = state === 'pending', failed = state === 'failed';
  cut(s, 'Light rail / outer bezel', 102, 370, 230, 58, '#514943', 8);
  cut(s, 'Light rail / recessed well', 104, 372, 226, 54, '#0b0908', 6);
  const tone = failed ? colors.danger : unavailable ? colors.edge : colors.gold;
  for (let i = 0; i < 24; i++) shape(s, 'Light segment ' + i, 113 + i * 8.5, 384, 5, 25, !unavailable && i < Math.ceil(value / 100 * 24) ? tone : '#3a322c', 1);
  if (!unavailable) {
    const x = 111 + 196 * value / 100;
    cut(s, 'Light cursor / travelling keycap', x - 9, state === 'drag' ? 373 : 375, 20, 42, colors.paper, 5);
    shape(s, 'Light cursor / slit', x - 1, 384, 3, 24, colors.ink);
    shape(s, 'Light cursor / foot', x - 5, 420, 11, 2, tone);
    if (pending) copy(s, 'SENDING…', 143, 443, 10, colors.gold);
    if (failed) copy(s, 'Not applied · tap to retry', 107, 443, 10, colors.danger);
  }
  [['0', 108], ['25', 156], ['50', 208], ['75', 258], ['100', 307]].forEach(([v, x]) => copy(s, v, x, 430, 9, colors.muted));
  if (unavailable) copy(s, 'Controls unavailable · reconnecting', 32, 452, 10, colors.gold);
}
function chart(s, title, unit, points, limit, max, color = colors.teal) {
  copy(s, title, 50, 109, 16); copy(s, unit, 640, 113, 12, colors.muted);
  for (let i = 0; i < 4; i++) {
    const y = 160 + i * 40; shape(s, 'Grid', 82, y, 652, 1, colors.edge);
    copy(s, String(Math.round(max * (1 - i / 3))), 30, y - 6, 11, colors.muted);
  }
  const y = 280 - limit / max * 120;
  for (let x = 82; x < 734; x += 14) shape(s, 'Configured threshold', x, y, 7, 1, colors.gold);
  s.nodes.push({ type: 'line', name: 'Sensor series', points: points.map((v, i) => [82 + i * 652 / (points.length - 1), 280 - v / max * 120]), fill: color });
  copy(s, '13:32', 82, 292, 12, colors.muted); copy(s, '14:02', 384, 292, 12, colors.muted); copy(s, '14:32', 683, 292, 12, colors.muted);
}
function overview(s, mode = 'day', corner = '') {
  const night = mode === 'night', offline = mode === 'offline', calm = mode === 'calm';
  tag(s, 'STUDIO / LYON', 20, 14); copy(s, 'TUE 14 OCT', 330, 12, 13, colors.muted);
  copy(s, offline ? 'OFFLINE / 8 MIN' : '● LIVE · 14:32', 638, 12, 12, offline ? colors.gold : colors.teal);
  copy(s, night ? '22:08' : '14:32', 20, 40, 80, colors.paper, 'VT323');
  copy(s, 'Tuesday, 14 October', 20, 124, 13, colors.muted);
  copy(s, night ? 'Sunrise in 9h 18m' : 'Sunset in 4h 13m', 20, 149, 13, colors.gold);
  shape(s, 'Outside', 256, 38, 348, 136, colors.panel, 5); tag(s, 'OUTSIDE · LYON', 268, 47);
  copy(s, offline ? '—' : night ? '12°' : '17°', 268, 65, 38, colors.paper, 'VT323');
  copy(s, offline ? 'Weather unavailable' : night ? 'Clear night' : 'Passing showers', 339, 69, 14);
  copy(s, '↓12° ↑19°', 339, 93, 11, colors.muted);
  copy(s, offline ? 'Next rain —' : night ? 'Next rain 09:10' : calm ? 'Next rain 16:10' : 'Next rain 14:44', 413, 93, 10, offline ? colors.muted : colors.teal);
  shape(s, night ? 'Moon' : 'Sun', 556, 48, 30, 30, night ? colors.muted : colors.gold, 15);
  if (night) shape(s, 'Moon cutout', 564, 44, 28, 28, colors.panel, 14);
  copy(s, night ? '07:26' : '18:45', 539, 85, 12, colors.gold);
  copy(s, offline ? 'FORECAST STALE · NOT NO RAIN' : night || calm ? 'NO RAIN / NEXT HOUR' : 'RAIN IN 12 MIN · mm/h', 268, 114, 11, offline ? colors.gold : colors.teal);
  const rain = [0, 0, 0, 0, 0, 0, 0, .1, .3, .6, .7, .9, 1.1, .9, .8, .7, .8, 1, .8, .6, .4, .3, .2, .1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
  rain.forEach((_, i) => { const v = i >= 12 ? rain[i - 5] : 0; shape(s, 'Rain minute ' + (i + 1), 268 + i * 5.3, 147 - (night || calm || offline ? 2 : Math.max(2, v * 16)), 3.5, night || calm || offline ? 2 : Math.max(2, v * 16), v && !night && !calm && !offline ? colors.teal : colors.edge, 1); });
  [['now', 268], ['+15', 338], ['+30', 417], ['+45', 496], ['+60', 559]].forEach(([v, x]) => copy(s, v, x, 153, 10, colors.muted));
  shape(s, 'Room', 616, 38, 164, 136, colors.panel, 5); tag(s, 'IN THE ROOM', 628, 47);
  copy(s, offline ? '—' : '21.4°', 628, 68, 38, colors.paper, 'VT323'); copy(s, offline ? 'SENSOR STALE' : 'RH 46%', 628, 108, 12, colors.muted);
  copy(s, offline ? 'HEAT UNKNOWN' : night ? '● HEAT IDLE' : '● HEATING', 628, 131, 12, colors.gold); copy(s, 'Target 22.0°', 628, 151, 12, colors.muted);
  shape(s, 'Agenda', 20, 186, 398, 126, colors.panel, 5); tag(s, 'AGENDA', 32, 195);
  copy(s, calm || night ? 'No more meetings today' : 'Product sync', 32, 217, 19, colors.paper, 'JetBrains Mono', 600);
  copy(s, calm || night ? 'A little room to breathe.' : '14:00–14:45', 32, 244, 13, colors.muted);
  if (!calm && !night) { copy(s, '13m', 320, 216, 37, colors.teal, 'VT323'); copy(s, 'to end', 321, 251, 12, colors.muted); shape(s, 'Meeting elapsed track', 32, 269, 374, 3, colors.edge); shape(s, 'Meeting elapsed', 32, 269, 266, 3, colors.teal); copy(s, 'NEXT  Design review', 32, 285, 13); copy(s, '15:00–15:30 · in 28m', 235, 286, 11, colors.muted); }
  shape(s, 'Room light', 20, 324, 398, 140, colors.panel, 5); tag(s, 'ROOM LIGHT', 32, 334); copy(s, offline ? '—' : '64%', 339, 331, 27, colors.gold, 'VT323');
  key(s, '0%', 32, 369, 60, offline ? '' : 'light-off', 'quiet', 58);
  lightDeck(s, 64, offline ? 'offline' : 'idle');
  key(s, '100%', 344, 369, 62, offline ? '' : 'light-full', offline ? 'quiet' : 'gold', 58);
  shape(s, 'Immediate actions', 430, 186, 350, 130, '#2b2115', 5); tag(s, calm ? 'ACT NOW / ALL CLEAR' : 'ACT NOW / 02', 442, 195, calm ? colors.teal : colors.gold);
  if (calm) copy(s, 'Nothing needs you right now.', 442, 234, 14, colors.muted);
  else [['Printer at clogging point', 'PLA chamber 42°C · cool enclosure', 'immediate-printer'], ['Ventilate the room', 'CO₂ 1,420 ppm · rising', 'immediate-co2']].forEach(([title, evidence, target], i) => { shape(s, 'Immediate rail', 430, 220 + i * 49, 3, 45, colors.gold); copy(s, title, 442, 220 + i * 49, 14, colors.paper, 'JetBrains Mono', 600); copy(s, evidence, 442, 242 + i * 49, 10, colors.muted); key(s, 'VIEW', 716, 218 + i * 49, 52, target, 'quiet', 48); });
  shape(s, 'Maintenance', 430, 324, 350, 140, colors.panel, 5); tag(s, 'MAINTENANCE / 04', 442, 333); s.links.push({ label: 'Maintenance queue', x: 440, y: 324, w: 330, h: 31, target: 'maintenance-queue' });
  [['Re-humidify humidor', 'RH 61% · target 68–72%', 'maintenance-snooze'], ['Update air sensor', 'Firmware 1.8.2 available', 'update-review']].forEach(([title, detail, target], i) => { copy(s, title, 442, 358 + i * 47, 14); copy(s, detail, 442, 379 + i * 47, 10, colors.muted); key(s, i ? 'GO' : 'zZ', 720, 355 + i * 47, 48, target, i ? 'gold' : 'quiet', 44); });
  copy(s, '+2 queued · battery & filter', 442, 452, 10, colors.muted);
  if (corner) { shape(s, 'Corner shelf backing', 590, 0, 210, 64, colors.ink); key(s, corner === 'critical' ? '! FIRE · REOPEN' : 'ACT NOW · REOPEN', 596, 8, 188, corner === 'critical' ? 'critical-fire' : 'immediate-printer', corner === 'critical' ? 'rust' : 'gold', 48); }
}
function sheet(s, heading, hint) {
  overview(s); shape(s, 'Modal scrim', 0, 0, 800, 480, colors.ink); shape(s, 'Choice sheet', 60, 84, 680, 316, colors.panel, 6);
  tag(s, heading, 84, 108, colors.gold); copy(s, hint, 84, 142, 22, colors.paper, 'JetBrains Mono', 600);
}
overview(makeScene('overview', '01 / Overview · active day', 'All primary categories, two immediate actions and routine stack.'));
overview(makeScene('overview-night', '02 / Overview · night', 'Moon, sunrise countdown and empty agenda.'), 'night');
overview(makeScene('overview-calm', '03 / Overview · no immediate action', 'Routine maintenance remains quiet.'), 'calm');
overview(makeScene('overview-offline', '04 / Overview · stale / offline', 'Unknown is not safe or no rain. Controls are unavailable.'), 'offline');
const queue = makeScene('maintenance-queue', '05 / Maintenance queue', 'Routine actions only; fixed pages, never scroll.');
sheet(queue, 'MAINTENANCE / 04 TASKS', 'Do these when it suits you.');
[['Re-humidify humidor', 'RH 61% · target 68–72%'], ['Replace sensor battery', 'Study window · battery 9%'], ['Update air sensor firmware', 'Version 1.8.2 available']].forEach(([title, hint], i) => { copy(queue, title, 84, 188 + i * 56, 15); copy(queue, hint, 84, 210 + i * 56, 11, colors.muted); key(queue, 'Later', 632, 183 + i * 56, 84, 'maintenance-snooze', 'quiet', 48); });
key(queue, 'Back', 84, 350, 100, 'overview'); copy(queue, '1 / 2', 520, 367, 13, colors.muted); key(queue, 'Next →', 604, 350, 112, 'maintenance-queue-next');
const q2 = makeScene('maintenance-queue-next', '06 / Maintenance queue · next page', 'One remaining task, no artificial filler.'); sheet(q2, 'MAINTENANCE / PAGE 2 OF 2', 'Check air filter'); copy(q2, 'Service interval reached · air purifier', 84, 195, 14, colors.muted); key(q2, 'Later', 632, 184, 84, 'maintenance-snooze'); key(q2, '← Previous', 84, 350, 144, 'maintenance-queue'); key(q2, 'Back to overview', 502, 350, 214, 'overview');
for (const [id, title, action] of [['maintenance-snooze', '07 / Routine snooze', 'Re-humidify humidor'], ['maintenance-battery', '08 / Routine snooze · battery', 'Replace sensor battery']]) {
  const s = makeScene(id, title, 'Evening or next occurrence; recurrence depends on the rule.'); sheet(s, 'REMIND ME LATER / MAINTENANCE', action);
  copy(s, 'This evening: remind at 18:00 local time.', 84, 196, 14, colors.muted); copy(s, 'Next occurrence: wait for clear → new occurrence.', 84, 221, 14, colors.muted);
  key(s, 'This evening · 18:00', 84, 292, 248, 'maintenance-deferred', 'gold'); key(s, 'Next occurrence', 344, 292, 220, 'maintenance-deferred'); key(s, 'Cancel', 576, 292, 140, 'overview'); copy(s, 'If evening has passed, show the next explicit reminder time.', 84, 364, 11, colors.muted);
}
const deferred = makeScene('maintenance-deferred', '09 / Maintenance · deferred', 'Confirmation distinguishes suppression from resolution.'); sheet(deferred, 'REMINDER SAVED', 'You can get back to your day.'); copy(deferred, 'The routine occurrence is snoozed, not resolved.', 84, 205, 16, colors.muted); copy(deferred, 'The selected rule controls when it appears again.', 84, 233, 14, colors.muted); key(deferred, 'Back to overview', 84, 316, 264, 'overview', 'gold');
function alert(s, critical, title, source, value, evidence, instruction, detail, discardTarget) {
  if (!critical) { overview(s); shape(s, 'Backdrop', 0, 0, 800, 480, colors.ink); shape(s, 'Immediate panel', 48, 54, 704, 368, colors.panel, 4); }
  else { shape(s, 'Critical frame', 0, 0, 800, 480, colors.rust); shape(s, 'Critical interior', 8, 8, 784, 464, '#190c09'); }
  const x = critical ? 32 : 72, y = critical ? 26 : 76, tone = critical ? colors.danger : colors.gold;
  for (let i = 0; i < (critical ? 25 : 22); i++) shape(s, 'Hazard segment', (critical ? 8 : 48) + i * 32, critical ? 8 : 54, 16, 12, critical ? colors.rust : colors.gold);
  tag(s, critical ? 'CRITICAL / PHYSICAL DANGER' : 'ACT NOW / IMMEDIATE ACTION', x, y, tone); copy(s, source, x, y + 27, 12, colors.muted);
  copy(s, title, x, y + 57, critical ? 86 : 60, colors.paper, 'VT323');
  if (value) { copy(s, value, x, y + 133, 58, tone, 'VT323'); copy(s, evidence, x + 174, y + 149, 13, colors.muted); }
  else copy(s, evidence, x, y + 158, 14, colors.muted);
  copy(s, instruction, x, y + 207, critical ? 23 : 18, tone);
  const ky = critical ? 350 : 337;
  key(s, 'Snooze · corner', x, ky, 238, critical ? 'critical-corner' : 'immediate-corner', critical ? 'rust' : 'gold', 56);
  key(s, 'Discard', x + 250, ky, 190, discardTarget, 'quiet', 56);
  copy(s, 'See more ↗', x + 468, ky + 20, 12, colors.muted); s.links.push({ label: 'See more', x: x + 458, y: ky, w: 154, h: 56, target: detail });
  copy(s, critical ? 'DISPLAY ONLY · DOES NOT SILENCE THE PHYSICAL ALARM' : 'SNOOZE = CORNER / DISCARD = UNTIL NEXT OCCURRENCE', x, critical ? 437 : 395, 10, colors.muted);
}
alert(makeScene('immediate-printer', '10 / Immediate · PLA clogging', '42°C is the user-confirmed clogging point in this printer.'), false, 'PLA CLOGGING POINT', 'PRINTER / ENCLOSURE / PLA PRINT ACTIVE', '42°C', 'Your printer’s clogging threshold', 'Cool the enclosure to prevent a clog.', 'detail-printer', 'discard-immediate');
alert(makeScene('immediate-co2', '11 / Immediate · room CO₂', 'Ventilation need, not a physical-harm alarm.'), false, 'VENTILATE THE ROOM', 'ROOM AIR / STUDIO / RISING', '1,420', 'ppm CO₂ · example reading', 'Ventilate if outdoor air conditions permit.', 'detail-co2', 'discard-immediate');
alert(makeScene('immediate-voc', '12 / Immediate · room VOC', 'VOC index is not a toxic-gas concentration.'), false, 'CHECK THE AIR SOURCE', 'ROOM AIR / VOC INDEX / RISING', '245', 'VOC index · example reading', 'Check sources; ventilate if appropriate.', 'detail-voc', 'discard-immediate');
alert(makeScene('critical-fire', '13 / Critical · fire alarm', 'Full takeover; display actions never silence alarm hardware.'), true, 'FIRE ALARM', 'SMOKE DETECTOR / STUDIO / ALARM ACTIVE', '', 'Alarm reported active · follow your emergency plan.', 'Leave the area. Get to safety.', 'detail-fire', 'discard-critical');
alert(makeScene('critical-toxic', '14 / Critical · toxic air', 'Use a dedicated validated alarm, not the generic VOC index.'), true, 'TOXIC AIR ALARM', 'DEDICATED CO ALARM / STUDIO / ALARM ACTIVE', '', 'Source reports danger · do not infer safety from a VOC sensor.', 'Leave the area. Get to fresh air.', 'detail-toxic', 'discard-critical');
overview(makeScene('immediate-corner', '15 / Immediate · snoozed to corner', 'Persistent unresolved badge; tap to reopen.'), 'day', 'immediate');
overview(makeScene('critical-corner', '16 / Critical · snoozed to corner', 'Critical retains crown colour and danger wording.'), 'day', 'critical');
const multi = makeScene('corner-multiple', '17 / Corner · multiple unresolved alerts', 'Critical wins the corner; expand the other alerts without scrolling.'); overview(multi, 'day', 'critical'); key(multi, '+2 ACT NOW', 596, 64, 188, 'immediate-co2', 'gold', 48);
for (const [id, title, critical] of [['discard-immediate', '18 / Discard · immediate occurrence', false], ['discard-critical', '19 / Discard · critical occurrence', true]]) {
  const s = makeScene(id, title, 'Confirm discarding this occurrence, not clearing the source.'); sheet(s, critical ? 'DISCARD / CRITICAL OCCURRENCE' : 'DISCARD / THIS OCCURRENCE', 'Hide this alert until it happens again?');
  copy(s, 'The pop-in and corner indicator will disappear.', 84, 201, 14, colors.muted); copy(s, 'It re-arms only after clear → new occurrence.', 84, 228, 14, colors.muted);
  if (critical) copy(s, 'This does NOT silence the physical alarm.', 84, 262, 15, colors.danger);
  key(s, 'Discard occurrence', 84, 322, 290, 'occurrence-discarded', critical ? 'rust' : 'gold'); key(s, 'Keep alert', 386, 322, 250, critical ? 'critical-fire' : 'immediate-printer');
}
const discarded = makeScene('occurrence-discarded', '20 / Occurrence discarded', 'No badge remains. The discarded printer occurrence is also absent from Act now.'); overview(discarded);
discarded.nodes = discarded.nodes.filter(n => !(n.x >= 430 && n.y >= 218 && n.y <= 266));
discarded.links = discarded.links.filter(l => l.target !== 'immediate-printer');
copy(discarded, 'Selected occurrence hidden', 442, 231, 12, colors.muted);
shape(discarded, 'Toast', 100, 418, 600, 46, colors.paper, 4); copy(discarded, 'Occurrence hidden · re-arms after clear → new occurrence', 114, 433, 12, colors.ink);
function details(id, title, heading, reading, unit, values, limit, max, alertId, note) {
  const s = makeScene(id, title, note); tag(s, 'DETAIL / SENSOR HISTORY', 28, 22, colors.teal); copy(s, heading, 28, 56, 28, colors.paper, 'JetBrains Mono', 600); copy(s, reading, 600, 50, 38, colors.gold, 'VT323');
  chart(s, 'LAST HOUR / 1 MIN SAMPLES', unit, values, limit, max); copy(s, 'Dashed: configured threshold · sample data', 82, 325, 12, colors.gold); copy(s, note, 28, 355, 12, colors.muted); key(s, '← Back to alert', 28, 410, 232, alertId); key(s, 'Overview', 568, 410, 204, 'overview'); return s;
}
details('detail-printer', '21 / Detail · enclosure temperature', 'Printer enclosure / PLA', '42°C', '°C', [28, 29, 31, 32, 34, 35, 37, 38, 40, 41, 42], 42, 50, 'immediate-printer', '42°C = this printer’s PLA clogging point. Not a physical-harm alarm.');
details('detail-co2', '22 / Detail · CO₂ graph', 'Studio / carbon dioxide', '1,420', 'ppm', [610, 640, 720, 820, 890, 940, 1070, 1140, 1210, 1350, 1420], 1200, 1800, 'immediate-co2', 'Example ventilation threshold. Outdoor air quality changes the recommended action.');
details('detail-voc', '23 / Detail · VOC graph', 'Studio / VOC index', '245', 'index', [90, 93, 105, 102, 120, 135, 150, 171, 191, 218, 245], 200, 300, 'immediate-voc', 'VOC index is relative. It cannot identify a toxin or certify that air is safe.');
details('detail-pm', '24 / Detail · PM2.5 context', 'Studio / particles', '38', 'µg/m³', [10, 12, 15, 17, 22, 25, 29, 31, 35, 37, 38], 25, 60, 'immediate-co2', 'OUTSIDE 52 µg/m³ · prefer filtering; opening the window may worsen indoor air.');
for (const [id, title, heading, back] of [['detail-fire', '25 / Detail · fire alarm source', 'Smoke detector / studio', 'critical-fire'], ['detail-toxic', '26 / Detail · toxic alarm source', 'Dedicated CO alarm / studio', 'critical-toxic']]) {
  const s = makeScene(id, title, 'Show the actual source status, not a made-up analogue smoke graph.'); tag(s, 'DETAIL / CRITICAL ALARM SOURCE', 28, 22, colors.danger); copy(s, heading, 28, 60, 28, colors.paper, 'JetBrains Mono', 600); shape(s, 'Active state', 28, 118, 744, 68, '#2b1510', 4); copy(s, 'ALARM ACTIVE / SOURCE REPORTED', 48, 137, 25, colors.danger, 'VT323');
  [['14:32:08', 'Alarm changed: inactive → active'], ['14:32:09', 'Screensight received the event'], ['14:32:10', 'Critical pop-in displayed']].forEach(([time, event], i) => { copy(s, time, 48, 222 + i * 43, 14, colors.gold); copy(s, event, 180, 222 + i * 43, 14); }); copy(s, 'Illustrative event log. Display controls do not silence this source.', 28, 357, 12, colors.muted); key(s, '← Back to alarm', 28, 410, 240, back); }
const resolved = makeScene('resolved', '27 / Condition resolved', 'A source clear removes snoozed badges and re-arms future occurrences.'); overview(resolved, 'calm'); shape(resolved, 'Resolved toast', 150, 414, 500, 50, colors.panel, 4); copy(resolved, 'Condition cleared · corner indicator removed', 166, 431, 13, colors.teal);
function pairingBase(s, step, title) {
  tag(s, 'SCREENSIGHT / FIRST CONNECTION', 28, 22, colors.teal); copy(s, step, 652, 22, 12, colors.muted);
  copy(s, title, 28, 65, 56, colors.paper, 'VT323'); shape(s, 'Pairing divider', 28, 132, 744, 2, colors.edge);
}
const pair = makeScene('pairing', '28 / Pairing · unpaired', 'Proposed pairing UX, not a claim about an implemented protocol. Example code is not live.');
pairingBase(pair, 'SETUP / 01', 'A small window on your home.');
copy(pair, 'Pair this display', 28, 165, 23, colors.paper, 'JetBrains Mono', 600);
copy(pair, '1  Open Home Assistant.', 28, 213, 16);
copy(pair, '2  Add the Screensight integration.', 28, 247, 16);
copy(pair, '3  Select this display and confirm the code.', 28, 281, 16);
shape(pair, 'Pairing code', 526, 168, 246, 174, colors.panel, 5); tag(pair, 'PAIRING CODE', 544, 188, colors.gold); copy(pair, '724 196', 544, 223, 48, colors.gold, 'VT323'); copy(pair, 'EXAMPLE / NOT A LIVE CODE', 544, 299, 10, colors.muted);
copy(pair, 'Device: Screensight Studio', 28, 346, 13, colors.muted); copy(pair, 'Local network connected · waiting for your home', 28, 371, 12, colors.teal);
key(pair, 'Waiting for confirmation…', 28, 410, 350, 'pairing-confirm', 'quiet'); key(pair, 'Connection help', 542, 410, 230, 'pairing-help');
const pc = makeScene('pairing-confirm', '29 / Pairing · confirm home', 'Touch confirmation avoids pairing to an unexpected requesting home.'); pairingBase(pc, 'SETUP / 02', 'Is this your home?');
tag(pc, 'PAIRING REQUEST RECEIVED', 28, 163, colors.teal); copy(pc, 'Home Assistant / My home', 28, 204, 25, colors.paper, 'JetBrains Mono', 600); copy(pc, 'Confirm that the same code appears there.', 28, 256, 16, colors.muted); copy(pc, '724 196', 28, 292, 64, colors.gold, 'VT323'); key(pc, 'Yes · pair this display', 28, 410, 352, 'pairing-success', 'gold'); key(pc, 'Not my home', 542, 410, 230, 'pairing');
const ps = makeScene('pairing-success', '30 / Pairing · success', 'Success gives way to the dashboard, not a permanent platform label.'); pairingBase(ps, 'SETUP / DONE', 'Your room has a little window.'); tag(ps, 'PAIRING COMPLETE', 28, 166, colors.teal); copy(ps, 'Screensight Studio', 28, 213, 31, colors.paper, 'JetBrains Mono', 600); copy(ps, 'Display preferences are configured in your home.', 28, 273, 16, colors.muted); copy(ps, 'Time, weather, agenda, room controls and reminders.', 28, 302, 14, colors.muted); key(ps, 'Open overview', 28, 410, 300, 'overview', 'gold');
const pt = makeScene('pairing-timeout', '31 / Pairing · timeout / retry', 'Fresh-code and troubleshooting states without a scrollable wizard.'); pairingBase(pt, 'SETUP / RETRY', 'Still waiting for your home.'); tag(pt, 'PAIRING REQUEST TIMED OUT', 28, 167, colors.gold); copy(pt, 'The example code has expired.', 28, 213, 20); copy(pt, 'Keep this display and your home on the same network.', 28, 266, 14, colors.muted); copy(pt, 'Then request a new code and try again.', 28, 294, 14, colors.muted); key(pt, 'Get a new code', 28, 410, 300, 'pairing', 'gold'); key(pt, 'Connection help', 542, 410, 230, 'pairing-help');
const ph = makeScene('pairing-help', '32 / Pairing · connection help', 'One fixed help page; avoid promising a specific network provisioning mechanism.'); pairingBase(ph, 'SETUP / HELP', 'Let’s make the connection.');
[['01', 'Same local network', 'Connect the display and home server to the same network.'], ['02', 'Screensight integration', 'Install and add the integration, then select this display.'], ['03', 'Still not discovered?', 'Check connectivity and discovery permissions in your home.']].forEach(([number, title, hint], i) => { copy(ph, number, 28, 162 + i * 73, 30, colors.gold, 'VT323'); copy(ph, title, 86, 164 + i * 73, 17, colors.paper, 'JetBrains Mono', 600); copy(ph, hint, 86, 193 + i * 73, 12, colors.muted); }); key(ph, '← Back to pairing', 28, 410, 280, 'pairing'); key(ph, 'Retry connection', 542, 410, 230, 'pairing');
for (const [id, title, value, state, effect] of [
  ['light-off', '33 / Light deck · off', 0, 'idle', ''],
  ['light-full', '34 / Light deck · full', 100, 'idle', ''],
  ['light-drag', '35 / Light deck · drag / target 78%', 78, 'drag', 'drag'],
  ['light-pending', '36 / Light deck · command pending', 78, 'pending', ''],
  ['light-failed', '37 / Light deck · command failed', 64, 'failed', ''],
  ['touch-press', '38 / Signal cast · contact / 0–80 ms', 64, 'idle', 'press'],
  ['touch-release', '39 / Signal cast · release / 80–320 ms', 64, 'idle', 'release'],
  ['touch-reduced', '40 / Signal cast · reduced motion', 64, 'idle', 'reduced']
]) {
  const s = makeScene(id, title, 'Instrument deck and Minitel keys. Touch effect is feedback, not confirmed command success.'); overview(s);
  s.nodes = s.nodes.filter(n => !(n.name.startsWith('Light ') || n.type === 'text' && ['64%', '0', '25', '50', '75', '100'].includes(n.text)));
  lightDeck(s, value, state); copy(s, state === 'pending' ? '78%…' : value + '%', 339, 331, 27, state === 'failed' ? colors.danger : colors.gold, 'VT323');
  if (effect) cast(s, effect === 'drag' ? 111 + 196 * .78 : 375, 397, effect, colors.gold);
}
const motion = makeScene('motion-language', '41 / Signal cast · motion language', 'Clipped quadrilateral wavefronts, four shards, no circular spell sigil.');
tag(motion, 'EXPRESSION / SIGNAL CAST', 28, 22, colors.teal); copy(motion, 'A touch of terminal sorcery.', 28, 62, 52, colors.paper, 'VT323');
for (const [label, x, phase] of [['CONTACT / 0–80 MS', 135, 'press'], ['RELEASE / 80–320 MS', 395, 'release'], ['REDUCED MOTION', 660, 'reduced']]) { tag(motion, label, x - 102, 166); key(motion, 'CONFIRM', x - 94, 235, 188, '', 'gold', 58); cast(motion, x, 264, phase); }
copy(motion, 'Press: 2 px key travel + compact contact lock.', 28, 357, 14, colors.muted); copy(motion, 'Release: two angular wavefronts + four cut-paper shards. Settle by 320 ms.', 28, 388, 13, colors.muted); copy(motion, 'Drag: cursor tether, no particle stream. Reduced: static brackets only.', 28, 419, 13, colors.muted);
const ur = makeScene('update-review', '42 / Actionable maintenance · update review', 'Explicit action before sending a Home Assistant command. Illustrative capability.');
sheet(ur, 'MAINTENANCE / ACTION AVAILABLE', 'Update studio air sensor');
copy(ur, 'Installed 1.8.1 → available 1.8.2', 84, 198, 17, colors.teal);
copy(ur, 'The sensor may be temporarily unavailable during the update.', 84, 238, 13, colors.muted);
copy(ur, 'Start only when it is a good time to interrupt readings.', 84, 263, 13, colors.muted);
key(ur, 'Start update', 84, 322, 250, 'update-starting', 'gold'); key(ur, 'Later', 346, 322, 160, 'maintenance-snooze'); key(ur, 'Back', 518, 322, 198, 'overview');
for (const [id, title, state] of [['update-starting', '43 / Update · starting / indeterminate', 'starting'], ['update-progress', '44 / Update · 46% in overview', 'progress'], ['update-success', '45 / Update · completed', 'success'], ['update-failed', '46 / Update · failed / retry', 'failed']]) {
  const s = makeScene(id, title, 'Progress reflects reported state; never an animation-generated percentage.'); overview(s);
  s.nodes = s.nodes.filter(n => !(n.y >= 400 && n.y < 448 && n.x >= 430));
  s.links = s.links.filter(l => !(l.y >= 400 && l.y < 448 && l.x >= 430));
  const tone = state === 'failed' ? colors.danger : state === 'success' ? colors.teal : colors.gold;
  copy(s, state === 'failed' ? 'Air sensor update failed' : state === 'success' ? 'Air sensor updated' : 'Updating air sensor', 442, 403, 13, colors.paper, 'JetBrains Mono', 600);
  if (state === 'progress' || state === 'starting') {
    shape(s, 'Update progress / track', 442, 429, 274, 5, colors.edge, 1);
    shape(s, state === 'starting' ? 'Update indeterminate marker' : 'Update progress / actual 46%', state === 'starting' ? 470 : 442, 429, state === 'starting' ? 50 : 126, 5, tone, 1);
    copy(s, state === 'starting' ? 'Starting…' : '46%', state === 'starting' ? 706 : 722, 439, 10, tone);
    s.links.push({ label: 'View update progress', x: 440, y: 400, w: 330, h: 44, target: 'update-detail' });
  } else {
    copy(s, state === 'success' ? 'Version 1.8.2 · complete' : 'Sensor did not confirm completion', 442, 426, 10, tone);
    key(s, state === 'failed' ? 'RETRY' : 'OK', 714, 401, 56, state === 'failed' ? 'update-review' : 'overview', state === 'failed' ? 'quiet' : 'gold', 44);
  }
}
const ud = makeScene('update-detail', '47 / Update · progress detail', 'Only expose cancellation if the integration supports it.');
sheet(ud, 'MAINTENANCE / RUNNING ACTION', 'Updating studio air sensor');
copy(ud, '46%', 84, 189, 64, colors.gold, 'VT323'); copy(ud, 'Installing firmware 1.8.2', 254, 208, 16);
shape(ud, 'Progress / track', 84, 274, 632, 10, colors.edge, 2); shape(ud, 'Progress / 46%', 84, 274, 291, 10, colors.gold, 2);
copy(ud, 'Progress reported by the integration · keep the sensor powered.', 84, 302, 12, colors.muted);
key(ud, 'Back to overview', 84, 342, 264, 'update-progress');
/* Empty/loading/error sections retain their allocated space and explain why.
 * Unknown observations must never be formatted as zero or an all-clear. */
function replaceRegion(s, x, y, w, h) {
  s.nodes = s.nodes.filter(n => !(n.x >= x && n.x < x + w && n.y >= y && n.y < y + h));
  s.links = s.links.filter(l => !(l.x >= x && l.x < x + w && l.y >= y && l.y < y + h));
  shape(s, 'Section / reserved layout', x, y, w, h, colors.panel, 5);
}
function emptyOverview(id, title, region, label, heading, hint, unavailable = false) {
  const s = makeScene(id, title, 'Empty ≠ loading ≠ unavailable. Preserve the spatial layout.'); overview(s, 'calm');
  const [x, y, w, h] = region; replaceRegion(s, x, y, w, h);
  tag(s, label, x + 12, y + 10, unavailable ? colors.gold : colors.muted);
  copy(s, heading, x + 12, y + 43, w < 200 ? 14 : 17, colors.paper);
  copy(s, hint, x + 12, y + 77, w < 200 ? 10 : 12, colors.muted); return s;
}
const nr = emptyOverview('weather-no-rain', '48 / Weather · no predicted rain', [256, 38, 348, 136], 'OUTSIDE · LYON', '17°  ·  No rain forecast', 'No rain in the available forecast horizon.');
copy(nr, '↓12° ↑19°', 268, 145, 11, colors.muted);
emptyOverview('weather-no-location', '49 / Weather · no location', [256, 38, 348, 136], 'OUTSIDE / NOT CONFIGURED', 'Choose a weather location', 'Set the location in your display preferences.');
emptyOverview('weather-loading', '50 / Weather · loading', [256, 38, 348, 136], 'OUTSIDE / LOADING', 'Fetching forecast…', 'Next rain and chart appear when data arrives.');
emptyOverview('weather-unavailable', '51 / Weather · unavailable', [256, 38, 348, 136], 'OUTSIDE / UNAVAILABLE', 'Forecast unavailable', 'Next rain unknown · not “no rain”.', true);
emptyOverview('room-empty', '52 / Room · no sensors', [616, 38, 164, 136], 'ROOM / SETUP', 'No room sensors', 'Choose room entities.');
emptyOverview('room-unavailable', '53 / Room · unavailable', [616, 38, 164, 136], 'ROOM / STALE', 'Readings unknown', 'Last report 8 min ago.', true);
const hi = makeScene('heating-idle', '54 / Room · heating idle', 'Idle is a known state; unavailable is not idle.'); overview(hi, 'calm');
hi.nodes.forEach(n => { if (n.text === '● HEATING') { n.text = '● HEAT IDLE'; n.fill = colors.muted; } });
const hu = makeScene('heating-unavailable', '55 / Room · heating unavailable', 'Room temperature may be valid while heating state is unknown.'); overview(hu, 'calm');
hu.nodes.forEach(n => { if (n.text === '● HEATING') { n.text = 'HEAT UNKNOWN'; n.fill = colors.gold; } if (n.text === 'Target 22.0°') n.text = 'Target —'; });
const nextOnly = emptyOverview('agenda-next-only', '56 / Agenda · no current meeting', [20, 186, 398, 126], 'AGENDA / NEXT', 'Design review', '15:00–15:30 · starts in 28 min');
copy(nextOnly, '28m', 325, 221, 35, colors.teal, 'VT323');
emptyOverview('agenda-empty', '57 / Agenda · no meetings', [20, 186, 398, 126], 'AGENDA / ALL CLEAR', 'No more meetings today', 'A little room to breathe.');
emptyOverview('agenda-not-configured', '58 / Agenda · not configured', [20, 186, 398, 126], 'AGENDA / SETUP', 'Choose a calendar', 'Add a calendar in your display preferences.');
emptyOverview('agenda-unavailable', '59 / Agenda · unavailable', [20, 186, 398, 126], 'AGENDA / UNAVAILABLE', 'Calendar unavailable', 'Upcoming meetings are unknown, not absent.', true);
const em = emptyOverview('maintenance-empty', '60 / Maintenance · empty', [430, 324, 350, 140], 'MAINTENANCE / 00', 'Nothing to maintain', 'You’re up to date.');
const eq = makeScene('maintenance-queue-empty', '61 / Maintenance queue · empty', 'Explicit empty list; no orphaned pagination.');
sheet(eq, 'MAINTENANCE / 00 TASKS', 'Nothing waiting for you.'); copy(eq, 'New routine tasks will appear here.', 84, 212, 15, colors.muted); key(eq, 'Back to overview', 84, 322, 264, 'maintenance-empty', 'gold');
emptyOverview('maintenance-unavailable', '62 / Maintenance · unavailable', [430, 324, 350, 140], 'MAINTENANCE / UNKNOWN', 'Rules unavailable', 'Cannot confirm whether tasks are pending.', true);
const nc = makeScene('light-not-configured', '63 / Lights · not configured', 'No enabled placeholder controls.'); overview(nc, 'calm'); replaceRegion(nc, 20, 324, 398, 140); tag(nc, 'ROOM LIGHT / SETUP', 32, 334); copy(nc, 'Choose the room light', 32, 369, 19); copy(nc, 'Select a light in your display preferences.', 32, 411, 12, colors.muted);
const hc = makeScene('history-empty', '64 / History · no samples yet', 'A blank graph does not draw a fictional line at zero.'); tag(hc, 'DETAIL / SENSOR HISTORY', 28, 22, colors.teal); copy(hc, 'Studio / carbon dioxide', 28, 56, 28); shape(hc, 'Empty plot', 28, 124, 744, 230, colors.panel, 4); copy(hc, 'No history samples yet', 190, 192, 24, colors.paper, 'VT323'); copy(hc, 'Current readings can appear before history is collected.', 190, 236, 12, colors.muted); key(hc, '← Back to alert', 28, 410, 232, 'immediate-co2');
const hs = makeScene('history-unavailable', '65 / History · unavailable', 'Keep the alert’s current source state; history failure is not resolution.'); tag(hs, 'DETAIL / HISTORY UNAVAILABLE', 28, 22, colors.gold); copy(hs, 'Couldn’t load sensor history', 28, 70, 36, colors.paper, 'VT323'); copy(hs, 'The active alert is unchanged. No graph is available.', 28, 160, 16, colors.muted); key(hs, 'Retry history', 28, 410, 232, 'history-empty', 'gold'); key(hs, 'Back to alert', 540, 410, 232, 'immediate-co2');
const su = makeScene('alarm-source-unavailable', '66 / Alarm source · unavailable', 'Never auto-resolve an alarm because its source stopped reporting.'); tag(su, 'CRITICAL / SOURCE UNAVAILABLE', 28, 22, colors.danger); copy(su, 'Last known state: ALARM ACTIVE', 28, 72, 44, colors.paper, 'VT323'); copy(su, 'The source is not reporting. Resolution is not confirmed.', 28, 166, 16, colors.muted); copy(su, 'Check the physical alarm and follow your emergency plan.', 28, 210, 16, colors.danger); key(su, 'Back to alarm', 28, 410, 260, 'critical-fire', 'rust');
const pn = makeScene('pairing-no-home', '67 / Pairing · no home discovered', 'No discovered home is different from a timed-out request.'); pairingBase(pn, 'SETUP / WAIT', 'No home found yet.'); copy(pn, 'This display is ready to pair.', 28, 178, 23); copy(pn, 'Open the Screensight integration in your home to continue.', 28, 232, 15, colors.muted); copy(pn, 'Check that the display and server share a local network.', 28, 266, 14, colors.muted); key(pn, 'Retry discovery', 28, 410, 300, 'pairing', 'gold'); key(pn, 'Connection help', 542, 410, 230, 'pairing-help');
