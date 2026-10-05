//! Screen compositor: turns a [`Screen`] into a rasterised [`FrameCanvas`] and
//! the touch hit regions for its keys.
//!
//! The device has exactly four faces — idle, pairing, confirm and the paired
//! dashboard — and only three actions between them (start, confirm, decline).
//! There is deliberately no step rail, help deck or other chrome: what the
//! device needs to say is said once, plainly.

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
    frame_for_with(screen, None)
}

/// Compose the frame for `screen`, optionally highlighting a pressed key.
#[must_use]
pub fn frame_for_with(
    screen: &Screen,
    pressed: Option<HitAction>,
) -> (FrameCanvas, Vec<(HitRegion, HitAction)>) {
    let mut canvas = FrameCanvas::new();
    let mut hits: Vec<(HitRegion, HitAction)> = Vec::new();
    match screen {
        Screen::Splash { name } => splash(&mut canvas, name),
        Screen::Idle => idle(&mut canvas, pressed, &mut hits),
        Screen::Pairing { code, name } => pairing(&mut canvas, name, code),
        Screen::Confirm { ha_name } => confirm(&mut canvas, ha_name, pressed, &mut hits),
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

/// The pairing window: the device's name and the code Home Assistant must be
/// told. No actions.
fn pairing(canvas: &mut FrameCanvas, name: &str, code: &str) {
    canvas.fill(BG);
    title(canvas, name);
    body(canvas, 140.0, "Enter this code in Home Assistant to pair:");

    let mut code_style = ts(VT323, 96.0, GORGET_500, Weight::NORMAL);
    code_style.line_height = 100.0;
    canvas.text(MARGIN, 196.0, &format_code(code), &code_style);

    status(canvas, 404.0, "Waiting for Home Assistant\u{2026}");
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
            Screen::Pairing {
                code: "123456".to_owned(),
                name: "Brave Otter".to_owned(),
            },
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
    fn only_idle_and_confirm_offer_actions() {
        assert!(frame_for(&Screen::Idle)
            .1
            .iter()
            .any(|(_, a)| *a == HitAction::StartPairing));
        assert!(frame_for(&Screen::Pairing {
            code: "123456".into(),
            name: "Brave Otter".into()
        })
        .1
        .is_empty());
        let (_, confirm) = frame_for(&Screen::Confirm {
            ha_name: "Home".into(),
        });
        assert!(confirm.iter().any(|(_, a)| *a == HitAction::Confirm));
        assert!(confirm.iter().any(|(_, a)| *a == HitAction::Decline));
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
    fn pairing_code_is_drawn_in_gorget_pixels() {
        let (canvas, _hits) = frame_for(&Screen::Pairing {
            code: "123456".to_owned(),
            name: "Brave Otter".to_owned(),
        });
        // The code is VT323 96px gorget-500 in the upper-left block.
        let mut found = false;
        for y in 196..304usize {
            for x in 48..400usize {
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
            "expected gorget-coloured code pixels in the pairing block"
        );
    }

    #[test]
    fn format_code_inserts_space() {
        assert_eq!(format_code("123456"), "123 456");
        assert_eq!(format_code("1234"), "12 34");
    }
}
