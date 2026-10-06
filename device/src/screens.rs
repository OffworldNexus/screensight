//! Screen compositor: turns a [`Screen`] into a rasterised [`FrameCanvas`] and
//! the touch hit regions for its keys.
//!
//! The device has a small set of faces — idle, the pairing loader, the SAS, the
//! confirmation, a single generic pairing error and the paired dashboard — and
//! only three actions between them (start, confirm, decline). There is
//! deliberately no step rail, help deck or other chrome: what the device needs
//! to say is said once, plainly.

use std::collections::BTreeMap;

use cosmic_text::Weight;

use crate::raster::{Color, FrameCanvas, TextAlign, TextStyle};
use crate::runtime::Screen;

/// Deck background.
pub const BG: Color = Color::hex(0x141110);
/// sand-800.
pub const SAND_800: Color = Color::hex(0x211c18);
/// sand-300.
pub const SAND_300: Color = Color::hex(0xc0b4a4);
/// sand-50.
pub const SAND_50: Color = Color::hex(0xfaf7f2);
/// plumage-500.
pub const PLUMAGE_500: Color = Color::hex(0x0bb2b3);
/// plumage-300, the bright raster scan carrier.
pub const PLUMAGE_300: Color = Color::hex(0x4fdbda);
/// gorget-500.
pub const GORGET_500: Color = Color::hex(0xf2a900);

/// VT323 family name as loaded from the bundled font.
pub const VT323: &str = "VT323";
/// JetBrains Mono family name as loaded from the bundled font.
pub const JETBRAINS_MONO: &str = "JetBrains Mono";

/// Horizontal content margin.
const MARGIN: f32 = 48.0;
/// Content width between the margins.
const CONTENT_W: f32 = crate::raster::WIDTH as f32 - MARGIN * 2.0;

/// A rectangular touch target in frame pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRegion {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl HitRegion {
    /// Whether `(x, y)` falls inside the region.
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What a tap on a hit region should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitAction {
    /// Open the pairing window ("Start pairing").
    StartPairing,
    /// Approve the pending pairing ("Pair").
    Confirm,
    /// Decline the pending pairing ("Cancel").
    Decline,
}

fn ts<'a>(family: &'a str, px: f32, color: Color, weight: Weight) -> TextStyle<'a> {
    TextStyle {
        family,
        px,
        line_height: (px * 1.25).round(),
        weight,
        color,
        max_width: 0.0,
        align: TextAlign::Left,
    }
}

/// A screen title.
fn title(canvas: &mut FrameCanvas, text: &str) {
    let mut s = ts(VT323, 48.0, SAND_50, Weight::NORMAL);
    s.line_height = 52.0;
    s.max_width = CONTENT_W;
    canvas.text(MARGIN, 64.0, text, &s);
}

/// A body line under the title.
fn body(canvas: &mut FrameCanvas, y: f32, text: &str) {
    let mut s = ts(JETBRAINS_MONO, 18.0, SAND_300, Weight::NORMAL);
    s.line_height = 26.0;
    s.max_width = CONTENT_W;
    canvas.text(MARGIN, y, text, &s);
}

/// A quiet status line (never a button).
fn status(canvas: &mut FrameCanvas, y: f32, text: &str) {
    let mut s = ts(JETBRAINS_MONO, 15.0, PLUMAGE_500, Weight::NORMAL);
    s.line_height = 20.0;
    s.max_width = CONTENT_W;
    canvas.text(MARGIN, y, text, &s);
}

/// Draw one key and return its hit region.
#[allow(clippy::too_many_arguments)]
fn key(
    canvas: &mut FrameCanvas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    primary: bool,
    pressed: bool,
) -> HitRegion {
    let (base, fg) = if primary {
        (GORGET_500, BG)
    } else {
        (SAND_800, SAND_50)
    };
    let bg = if pressed {
        base.mix(SAND_50, if primary { 0.25 } else { 0.15 })
    } else {
        base
    };
    canvas.rounded_rect(x as i32, y as i32, w as i32, h as i32, 3.0, bg);
    let mut s = ts(JETBRAINS_MONO, 15.0, fg, Weight::NORMAL);
    s.line_height = 20.0;
    s.max_width = w - 24.0;
    canvas.text(x + 12.0, y + 18.0, label, &s);
    HitRegion { x, y, w, h }
}

/// Format a six-digit pairing code as `123 456`.
fn format_code(code: &str) -> String {
    let digits: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let chars: Vec<char> = digits.chars().collect();
    if chars.len() == 6 {
        format!(
            "{} {}",
            chars[..3].iter().collect::<String>(),
            chars[3..].iter().collect::<String>()
        )
    } else if chars.len() >= 2 {
        let mid = chars.len() / 2;
        format!(
            "{} {}",
            chars[..mid].iter().collect::<String>(),
            chars[mid..].iter().collect::<String>()
        )
    } else {
        digits
    }
}

/// Compose the frame for `screen`, without press feedback.
#[must_use]
pub fn frame_for(screen: &Screen) -> (FrameCanvas, Vec<(HitRegion, HitAction)>) {
    frame_for_with(screen, None, 0.0)
}

/// Compose the frame for `screen`, optionally highlighting a pressed key.
///
/// `elapsed` is the seconds since the current screen appeared; only the loader
/// screens use it (for the Signal-search sweep and pulse).
#[must_use]
pub fn frame_for_with(
    screen: &Screen,
    pressed: Option<HitAction>,
    elapsed: f32,
) -> (FrameCanvas, Vec<(HitRegion, HitAction)>) {
    let mut canvas = FrameCanvas::new();
    let mut hits: Vec<(HitRegion, HitAction)> = Vec::new();
    match screen {
        Screen::Splash { name } => splash(&mut canvas, name),
        Screen::Idle => idle(&mut canvas, pressed, &mut hits),
        Screen::PairingWaiting { name } => {
            loader(
                &mut canvas,
                name,
                "Waiting for Home Assistant\u{2026}",
                elapsed,
                2.4,
            );
        }
        Screen::PairingHandshake { name } => {
            loader(&mut canvas, name, "Exchanging keys\u{2026}", elapsed, 1.2);
        }
        Screen::PairingCode { sas } => pairing_code(&mut canvas, sas),
        Screen::Confirm { ha_name } => confirm(&mut canvas, ha_name, pressed, &mut hits),
        Screen::PairingError => pairing_error(&mut canvas, pressed, &mut hits),
        Screen::Dashboard { values } => dashboard(&mut canvas, values),
    }
    (canvas, hits)
}

/// Boot splash: paired, waiting to hear from Home Assistant.
fn splash(canvas: &mut FrameCanvas, name: &str) {
    canvas.fill(BG);
    let mut title = ts(VT323, 48.0, SAND_50, Weight::NORMAL);
    title.line_height = 52.0;
    title.max_width = CONTENT_W;
    title.align = TextAlign::Center;
    canvas.text(MARGIN, 176.0, name, &title);

    let mut hint = ts(JETBRAINS_MONO, 15.0, PLUMAGE_500, Weight::NORMAL);
    hint.line_height = 20.0;
    hint.max_width = CONTENT_W;
    hint.align = TextAlign::Center;
    canvas.text(MARGIN, 244.0, "Starting\u{2026}", &hint);
}

/// Not paired, and no pairing window open. One action: start pairing.
fn idle(
    canvas: &mut FrameCanvas,
    pressed: Option<HitAction>,
    hits: &mut Vec<(HitRegion, HitAction)>,
) {
    canvas.fill(BG);
    title(canvas, "Not paired");
    body(
        canvas,
        150.0,
        "This display isn't paired with Home Assistant yet.",
    );
    let region = key(
        canvas,
        MARGIN,
        376.0,
        260.0,
        56.0,
        "Start pairing",
        true,
        pressed == Some(HitAction::StartPairing),
    );
    hits.push((region, HitAction::StartPairing));
}

/// The pairing loader ("Signal search"): device name, an instruction, a small
/// raster-scanned globe and a status line. Used for both the waiting and the
/// handshake variants, which differ only in `status_text` and sweep period.
fn loader(canvas: &mut FrameCanvas, name: &str, status_text: &str, elapsed: f32, period: f32) {
    canvas.fill(BG);
    title(canvas, name);
    let mut instruction = ts(JETBRAINS_MONO, 18.0, SAND_300, Weight::NORMAL);
    instruction.line_height = 26.0;
    instruction.max_width = 460.0;
    canvas.text(
        MARGIN,
        140.0,
        "Open Home Assistant and select",
        &instruction,
    );
    canvas.text(
        MARGIN,
        174.0,
        "this display to begin pairing.",
        &instruction,
    );

    globe(canvas, elapsed, period);

    let mut hint = ts(JETBRAINS_MONO, 15.0, PLUMAGE_500, Weight::NORMAL);
    hint.line_height = 22.0;
    hint.max_width = CONTENT_W;
    canvas.text(MARGIN, 404.0, status_text, &hint);
}

/// Figma's 168px raster orb, at (552, 184). The deliberately sparse cell rows
/// trace meridians and parallels rather than filling a disc; boundary cells are
/// 4×3px, interior cells 3×3px, all on the reference's 7px grid.
fn globe(canvas: &mut FrameCanvas, elapsed: f32, period: f32) {
    const X: i32 = 552;
    const Y: i32 = 184;
    const ROWS: &[&[i32]] = &[
        &[84],
        &[56, 70, 84, 98, 112],
        &[42, 63, 84, 105, 126],
        &[35, 56, 84, 112, 133],
        &[
            28, 35, 42, 49, 56, 63, 70, 77, 84, 91, 98, 105, 112, 119, 126, 133, 140,
        ],
        &[28, 49, 84, 119, 140],
        &[21, 49, 84, 119, 147],
        &[21, 49, 84, 119, 147],
        &[
            21, 28, 35, 42, 49, 56, 63, 70, 77, 84, 91, 98, 105, 112, 119, 126, 133, 140, 147,
        ],
        &[21, 42, 49, 84, 119, 126, 147],
        &[14, 42, 49, 84, 119, 126, 154],
        &[21, 42, 49, 84, 119, 126, 147],
        &[
            21, 28, 35, 42, 49, 56, 63, 70, 77, 84, 91, 98, 105, 112, 119, 126, 133, 140, 147,
        ],
        &[21, 49, 84, 119, 147],
        &[21, 49, 84, 119, 147],
        &[28, 49, 84, 119, 140],
        &[
            28, 35, 42, 49, 56, 63, 70, 77, 84, 91, 98, 105, 112, 119, 126, 133, 140,
        ],
        &[35, 56, 84, 112, 133],
        &[42, 63, 84, 105, 126],
        &[56, 70, 84, 98, 112],
        &[84],
    ];
    for (row, columns) in ROWS.iter().enumerate() {
        for (index, x) in columns.iter().enumerate() {
            let width = if index == 0 || index == columns.len() - 1 {
                4
            } else {
                3
            };
            canvas.rect(X + x, Y + 14 + row as i32 * 7, width, 3, PLUMAGE_500);
        }
    }

    // The scan travels 126px by 92% of the cycle, fades before the reset,
    // then holds invisibly. Keep the twelve separate 5×3px carrier cells.
    let phase = (elapsed / period).rem_euclid(1.0);
    let opacity = if phase < 0.05 {
        phase / 0.05
    } else if phase <= 0.85 {
        1.0
    } else {
        ((0.92 - phase) / 0.07).max(0.0)
    };
    let sweep_y = Y + 19 + (126.0 * (phase / 0.92).min(1.0)).round() as i32;
    if opacity > 0.0 {
        let scan = Color::rgba(
            PLUMAGE_300.0,
            PLUMAGE_300.1,
            PLUMAGE_300.2,
            (opacity * 255.0).round() as u8,
        );
        for column in 0..12 {
            canvas.rect(X + 42 + column * 7, sweep_y, 5, 3, scan);
        }
    }

    // The gold square indicates activity, not trust. Both half-cycles use
    // Figma's CSS ease-in-out curve, from 50% to 100% opacity and back.
    let progress = if phase <= 0.5 {
        phase * 2.0
    } else {
        (1.0 - phase) * 2.0
    };
    let pulse = 0.5 + 0.5 * ease_in_out(progress);
    let core = Color::rgba(
        GORGET_500.0,
        GORGET_500.1,
        GORGET_500.2,
        (pulse * 255.0).round() as u8,
    );
    canvas.rect(X + 81, Y + 80, 8, 8, core);
}

/// Evaluate CSS cubic-bezier(0.42, 0, 0.58, 1), solving its time axis first so
/// the CPU animation uses the same easing as the Figma prototype.
fn ease_in_out(progress: f32) -> f32 {
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..16 {
        let t = (low + high) * 0.5;
        let x = 3.0 * (1.0 - t).powi(2) * t * 0.42 + 3.0 * (1.0 - t) * t * t * 0.58 + t * t * t;
        if x < progress {
            low = t;
        } else {
            high = t;
        }
    }
    let t = (low + high) * 0.5;
    3.0 * (1.0 - t) * t * t + t * t * t
}

/// The SAS: the eight digits the user types into Home Assistant, shown only
/// once the handshake has produced them. No actions.
fn pairing_code(canvas: &mut FrameCanvas, sas: &str) {
    canvas.fill(BG);
    title(canvas, "Pair this display");
    body(canvas, 140.0, "Enter this code in Home Assistant to pair:");

    let mut code_style = ts(VT323, 96.0, GORGET_500, Weight::NORMAL);
    code_style.line_height = 100.0;
    canvas.text(MARGIN, 196.0, &format_code(sas), &code_style);

    status(canvas, 404.0, "Waiting for confirmation\u{2026}");
}

/// The single generic pairing failure screen: one message, one action that
/// starts the flow over.
fn pairing_error(
    canvas: &mut FrameCanvas,
    pressed: Option<HitAction>,
    hits: &mut Vec<(HitRegion, HitAction)>,
) {
    canvas.fill(BG);
    title(canvas, "Pairing failed");
    body(canvas, 150.0, "Something went wrong during pairing.");
    let region = key(
        canvas,
        MARGIN,
        376.0,
        300.0,
        56.0,
        "Start pairing again",
        true,
        pressed == Some(HitAction::StartPairing),
    );
    hits.push((region, HitAction::StartPairing));
}

/// A Home Assistant typed the correct code; ask the user to approve it.
fn confirm(
    canvas: &mut FrameCanvas,
    ha_name: &str,
    pressed: Option<HitAction>,
    hits: &mut Vec<(HitRegion, HitAction)>,
) {
    canvas.fill(BG);
    title(canvas, "Pair this display?");
    body(
        canvas,
        150.0,
        "Home Assistant wants to pair with this display:",
    );

    let mut name = ts(JETBRAINS_MONO, 30.0, SAND_50, Weight::BOLD);
    name.line_height = 38.0;
    name.max_width = CONTENT_W;
    canvas.text(MARGIN, 190.0, ha_name, &name);

    let yes = key(
        canvas,
        MARGIN,
        376.0,
        200.0,
        56.0,
        "Pair",
        true,
        pressed == Some(HitAction::Confirm),
    );
    hits.push((yes, HitAction::Confirm));
    let no = key(
        canvas,
        MARGIN + 216.0,
        376.0,
        160.0,
        56.0,
        "Cancel",
        false,
        pressed == Some(HitAction::Decline),
    );
    hits.push((no, HitAction::Decline));
}

/// The paired dashboard.
fn dashboard(canvas: &mut FrameCanvas, values: &BTreeMap<String, String>) {
    canvas.fill(BG);
    match values.get("text").map(String::as_str) {
        Some(text) if !text.is_empty() => {
            let large = text.chars().count() <= 42;
            let (family, px, line_height) = if large {
                (VT323, 56.0, 60.0)
            } else {
                (JETBRAINS_MONO, 24.0, 32.0)
            };
            let mut style = ts(family, px, SAND_50, Weight::NORMAL);
            style.line_height = line_height;
            style.max_width = CONTENT_W;
            style.align = TextAlign::Center;
            let (_, h) = canvas.measure(text, &style);
            let y = ((crate::raster::HEIGHT as f32 - h) / 2.0).max(48.0);
            canvas.text(MARGIN, y, text, &style);
        }
        _ => {
            let mut style = ts(JETBRAINS_MONO, 16.0, SAND_300, Weight::NORMAL);
            style.line_height = 24.0;
            style.max_width = CONTENT_W;
            style.align = TextAlign::Center;
            let placeholder = "Waiting for Home Assistant\u{2026}";
            let (_, h) = canvas.measure(placeholder, &style);
            let y = ((crate::raster::HEIGHT as f32 - h) / 2.0).max(48.0);
            canvas.text(MARGIN, y, placeholder, &style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_screens() -> Vec<Screen> {
        vec![
            Screen::Splash {
                name: "Brave Otter".to_owned(),
            },
            Screen::Idle,
            Screen::PairingWaiting {
                name: "Brave Otter".to_owned(),
            },
            Screen::PairingHandshake {
                name: "Brave Otter".to_owned(),
            },
            Screen::PairingCode {
                sas: "12345678".to_owned(),
            },
            Screen::PairingError,
            Screen::Confirm {
                ha_name: "My home".to_owned(),
            },
            Screen::Dashboard {
                values: BTreeMap::from([("text".to_owned(), "Hello world".to_owned())]),
            },
            Screen::Dashboard {
                values: BTreeMap::new(),
            },
        ]
    }

    fn find(hits: &[(HitRegion, HitAction)], action: HitAction) -> HitRegion {
        hits.iter()
            .find(|(_, a)| *a == action)
            .map(|(r, _)| *r)
            .unwrap_or_else(|| panic!("missing hit region for {action:?}"))
    }

    #[test]
    fn every_screen_renders_a_full_frame_with_content() {
        for screen in sample_screens() {
            let (canvas, _hits) = frame_for(&screen);
            assert_eq!(
                canvas.frame().len(),
                crate::raster::WIDTH * crate::raster::HEIGHT * 4
            );
            let bg = BG.to_bgra();
            assert!(
                canvas.frame().as_chunks::<4>().0.iter().any(|px| px != &bg),
                "screen {screen:?} drew no content beyond the background"
            );
        }
    }

    #[test]
    fn only_idle_confirm_and_error_offer_actions() {
        assert!(frame_for(&Screen::Idle)
            .1
            .iter()
            .any(|(_, a)| *a == HitAction::StartPairing));
        assert!(frame_for(&Screen::PairingCode {
            sas: "12345678".into(),
        })
        .1
        .is_empty());
        assert!(frame_for(&Screen::PairingWaiting {
            name: "Brave Otter".into()
        })
        .1
        .is_empty());
        let (_, confirm) = frame_for(&Screen::Confirm {
            ha_name: "Home".into(),
        });
        assert!(confirm.iter().any(|(_, a)| *a == HitAction::Confirm));
        assert!(confirm.iter().any(|(_, a)| *a == HitAction::Decline));
        let (_, error) = frame_for(&Screen::PairingError);
        assert!(error.iter().any(|(_, a)| *a == HitAction::StartPairing));
        assert!(frame_for(&Screen::Dashboard {
            values: BTreeMap::new()
        })
        .1
        .is_empty());
    }

    #[test]
    fn confirm_hit_regions_do_not_overlap() {
        let (_, hits) = frame_for(&Screen::Confirm {
            ha_name: "My home".to_owned(),
        });
        let yes = find(&hits, HitAction::Confirm);
        let no = find(&hits, HitAction::Decline);
        assert!(yes.contains(yes.x + 1.0, yes.y + 1.0));
        assert!(no.contains(no.x + no.w - 1.0, no.y + no.h - 1.0));
        assert!(!yes.contains(no.x + 1.0, no.y + 1.0));
    }

    #[test]
    fn sas_is_drawn_in_gorget_pixels() {
        let (canvas, _hits) = frame_for(&Screen::PairingCode {
            sas: "12345678".to_owned(),
        });
        // The SAS is VT323 96px gorget-500 in the upper-left block.
        let mut found = false;
        for y in 196..304usize {
            for x in 48..704usize {
                let idx = (y * crate::raster::WIDTH + x) * 4;
                let b = canvas.frame()[idx];
                let g = canvas.frame()[idx + 1];
                let r = canvas.frame()[idx + 2];
                if r > 180 && g > 110 && b < 90 {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(
            found,
            "expected gorget-coloured SAS pixels in the pairing block"
        );
    }

    #[test]
    fn eight_digit_sas_fits_the_content_width() {
        let mut canvas = FrameCanvas::new();
        let style = ts(VT323, 96.0, GORGET_500, Weight::NORMAL);
        let (width, _) = canvas.measure(&format_code("12345678"), &style);
        assert!(
            width <= CONTENT_W,
            "SAS {width}px wider than the {CONTENT_W}px content column"
        );
    }

    #[test]
    fn loader_animates_and_stays_within_the_frame() {
        // Two points in the sweep cycle produce different pixels, i.e. the
        // loader is genuinely animated.
        let screen = Screen::PairingWaiting {
            name: "Brave Otter".to_owned(),
        };
        let (a, _) = frame_for_with(&screen, None, 0.0);
        let (b, _) = frame_for_with(&screen, None, 1.2);
        assert_ne!(
            a.frame(),
            b.frame(),
            "loader frames should differ over time"
        );
        assert!(Screen::PairingWaiting { name: "x".into() }.is_animated());
        assert!(Screen::PairingHandshake { name: "x".into() }.is_animated());
        assert!(!Screen::PairingCode {
            sas: "12345678".into()
        }
        .is_animated());
    }

    #[test]
    fn loader_matches_figma_wireframe_geometry() {
        let mut canvas = FrameCanvas::new();
        canvas.fill(BG);
        globe(&mut canvas, 0.0, 2.4);
        let pixel = |x, y| {
            let index = (y * crate::raster::WIDTH + x) * 4;
            &canvas.frame()[index..index + 4]
        };
        // Figma's pole, equator boundary, and a sparse interior meridian.
        assert_eq!(pixel(636, 198), PLUMAGE_500.to_bgra());
        assert_eq!(pixel(566, 268), PLUMAGE_500.to_bgra());
        assert_eq!(pixel(601, 240), PLUMAGE_500.to_bgra());
        assert_eq!(pixel(606, 240), BG.to_bgra());
        // The 8px carrier must not grow into the former 30px gold disc.
        assert_ne!(pixel(633, 264), BG.to_bgra());
        assert_eq!(pixel(632, 264), BG.to_bgra());
        assert_eq!(pixel(641, 264), BG.to_bgra());
        // Everything belongs to the 168×168px slot on the right.
        for y in 0..crate::raster::HEIGHT {
            for x in 0..crate::raster::WIDTH {
                if !(552..720).contains(&x) || !(184..352).contains(&y) {
                    assert_eq!(pixel(x, y), BG.to_bgra());
                }
            }
        }
    }

    #[test]
    fn loader_scan_fades_before_reset_and_pulse_loops() {
        let render_orb = |elapsed| {
            let mut canvas = FrameCanvas::new();
            canvas.fill(BG);
            globe(&mut canvas, elapsed, 2.4);
            canvas
        };
        let initial = render_orb(0.0);
        let repeated = render_orb(2.4);
        assert_eq!(initial.frame(), repeated.frame());
        let peak = render_orb(1.2);
        let core = (264 * crate::raster::WIDTH + 633) * 4;
        assert_eq!(&peak.frame()[core..core + 4], GORGET_500.to_bgra());
        assert_ne!(
            &initial.frame()[core..core + 4],
            &peak.frame()[core..core + 4]
        );
        let has_scan = |canvas: &FrameCanvas| {
            canvas
                .frame()
                .as_chunks::<4>()
                .0
                .contains(&PLUMAGE_300.to_bgra())
        };
        assert!(has_scan(&peak));
        assert!(!has_scan(&render_orb(2.3)));
        assert!(!has_scan(&initial));
    }

    #[test]
    fn format_code_inserts_space() {
        assert_eq!(format_code("123456"), "123 456");
        assert_eq!(format_code("12345678"), "1234 5678");
        assert_eq!(format_code("1234"), "12 34");
    }
}
