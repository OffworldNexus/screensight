//! Screen compositor: turns a [`Screen`] into a rasterised [`FrameCanvas`] and
//! the touch hit regions for its keys.
//!
//! The visuals follow the Screensight "06 · Device UI" design: a dark sand
//! deck, a Silkscreen kicker/step rail, VT323 display type and JetBrains Mono
//! body copy. All families are bundled and loaded by [`crate::raster`].

use crate::raster::{Color, FrameCanvas, TextAlign, TextStyle};
use crate::runtime::Screen;
use cosmic_text::Weight;

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
/// crown-500.
#[allow(dead_code)]
pub const CROWN_500: Color = Color::hex(0xc9541f);
/// Divider.
pub const DIVIDER: Color = Color::hex(0x514943);

/// Silkscreen family name as loaded from the bundled font.
pub const SILKSCREEN: &str = "Silkscreen";
/// VT323 family name as loaded from the bundled font.
pub const VT323: &str = "VT323";
/// JetBrains Mono family name as loaded from the bundled font.
pub const JETBRAINS_MONO: &str = "JetBrains Mono";

/// The kicker used on the pairing deck.
const KICKER: &str = "SCREENSIGHT / FIRST CONNECTION";

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
    /// Confirm the pending pairing.
    Confirm,
    /// Decline the pending pairing ("Not my home").
    Reject,
    /// Re-arm the pairing window ("Get a new code").
    Rearm,
    /// Open the connection-help screen.
    Help,
    /// Return from the help screen to the pairing screen.
    BackToPairing,
    /// Retry pairing from the help screen (re-arms the window).
    Retry,
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

/// Paint the shared deck chrome: background, kicker, step, title and divider.
fn deck(canvas: &mut FrameCanvas, kicker: &str, step: &str, title: &str) {
    canvas.fill(BG);
    if !kicker.is_empty() {
        let mut s = ts(SILKSCREEN, 12.0, PLUMAGE_500, Weight::NORMAL);
        s.line_height = 16.0;
        canvas.text(28.0, 22.0, kicker, &s);
    }
    if !step.is_empty() {
        let mut s = ts(JETBRAINS_MONO, 12.0, SAND_300, Weight::NORMAL);
        s.line_height = 16.0;
        s.max_width = 772.0;
        s.align = TextAlign::Right;
        canvas.text(0.0, 22.0, step, &s);
    }
    let mut t = ts(VT323, 56.0, SAND_50, Weight::NORMAL);
    t.line_height = 56.0;
    canvas.text(28.0, 65.0, title, &t);
    canvas.rect(28, 132, 744, 2, DIVIDER);
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
    let mut s = ts(JETBRAINS_MONO, 13.0, fg, Weight::NORMAL);
    s.line_height = 18.0;
    s.max_width = w - 24.0;
    canvas.text(x + 12.0, y + 17.0, label, &s);
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
pub fn frame_for(screen: &Screen, model: &str) -> (FrameCanvas, Vec<(HitRegion, HitAction)>) {
    frame_for_with(screen, model, None)
}

/// Compose the frame for `screen`, optionally highlighting a pressed key.
#[must_use]
pub fn frame_for_with(
    screen: &Screen,
    model: &str,
    pressed: Option<HitAction>,
) -> (FrameCanvas, Vec<(HitRegion, HitAction)>) {
    let mut canvas = FrameCanvas::new();
    let mut hits: Vec<(HitRegion, HitAction)> = Vec::new();
    match screen {
        Screen::Unpaired | Screen::NoHome => no_home(&mut canvas),
        Screen::Pairing { code } => pairing(&mut canvas, model, code, pressed, &mut hits),
        Screen::Confirm { code, ha_name } => {
            confirm(&mut canvas, code, ha_name, pressed, &mut hits)
        }
        Screen::Timeout => timeout(&mut canvas, pressed, &mut hits),
        Screen::Display { text } => display(&mut canvas, text),
    }
    (canvas, hits)
}

/// The no-home / unpaired state.
fn no_home(canvas: &mut FrameCanvas) {
    deck(canvas, "", "SETUP / WAIT", "No home found yet.");
    let mut body = ts(JETBRAINS_MONO, 16.0, SAND_300, Weight::NORMAL);
    body.line_height = 24.0;
    body.max_width = 744.0;
    canvas.text(
        28.0,
        216.0,
        "Open the Screensight integration in your home.",
        &body,
    );
}

/// The pairing window state.
fn pairing(
    canvas: &mut FrameCanvas,
    model: &str,
    code: &str,
    pressed: Option<HitAction>,
    hits: &mut Vec<(HitRegion, HitAction)>,
) {
    deck(canvas, KICKER, "SETUP / 01", "A small window on your home.");

    let mut lead = ts(JETBRAINS_MONO, 23.0, SAND_50, Weight::BOLD);
    lead.line_height = 30.0;
    canvas.text(28.0, 160.0, "Pair this display", &lead);

    let mut step = ts(JETBRAINS_MONO, 16.0, SAND_50, Weight::NORMAL);
    step.line_height = 24.0;
    step.max_width = 500.0;
    canvas.text(28.0, 213.0, "1  Open Home Assistant.", &step);
    canvas.text(28.0, 247.0, "2  Add the Screensight integration.", &step);
    canvas.text(
        28.0,
        281.0,
        "3  Select this display and confirm the code.",
        &step,
    );

    let mut label = ts(SILKSCREEN, 12.0, GORGET_500, Weight::NORMAL);
    label.line_height = 16.0;
    canvas.text(544.0, 188.0, "PAIRING CODE", &label);

    let mut code_style = ts(VT323, 48.0, GORGET_500, Weight::NORMAL);
    code_style.line_height = 52.0;
    canvas.text(544.0, 223.0, &format_code(code), &code_style);

    let mut device = ts(JETBRAINS_MONO, 13.0, SAND_300, Weight::NORMAL);
    device.line_height = 18.0;
    device.max_width = 228.0;
    canvas.text(544.0, 346.0, &format!("Device: {model}"), &device);

    let mut waiting = ts(JETBRAINS_MONO, 12.0, PLUMAGE_500, Weight::NORMAL);
    waiting.line_height = 16.0;
    waiting.max_width = 228.0;
    canvas.text(
        544.0,
        371.0,
        "Local network connected · waiting for your home",
        &waiting,
    );

    let region = key(
        canvas,
        28.0,
        410.0,
        300.0,
        52.0,
        "Connection help",
        false,
        pressed == Some(HitAction::Help),
    );
    hits.push((region, HitAction::Help));
}

/// The confirm-on-panel state.
fn confirm(
    canvas: &mut FrameCanvas,
    code: &str,
    ha_name: &str,
    pressed: Option<HitAction>,
    hits: &mut Vec<(HitRegion, HitAction)>,
) {
    deck(canvas, KICKER, "SETUP / 02", "Is this your home?");

    let mut received = ts(SILKSCREEN, 12.0, PLUMAGE_500, Weight::NORMAL);
    received.line_height = 16.0;
    canvas.text(28.0, 163.0, "PAIRING REQUEST RECEIVED", &received);

    let mut name = ts(JETBRAINS_MONO, 25.0, SAND_50, Weight::BOLD);
    name.line_height = 32.0;
    name.max_width = 744.0;
    canvas.text(28.0, 204.0, ha_name, &name);

    let mut body = ts(JETBRAINS_MONO, 16.0, SAND_300, Weight::NORMAL);
    body.line_height = 24.0;
    body.max_width = 744.0;
    canvas.text(
        28.0,
        256.0,
        "Confirm that the same code appears there.",
        &body,
    );

    let mut code_style = ts(VT323, 64.0, GORGET_500, Weight::NORMAL);
    code_style.line_height = 68.0;
    canvas.text(28.0, 292.0, &format_code(code), &code_style);

    let yes = key(
        canvas,
        28.0,
        410.0,
        352.0,
        52.0,
        "Yes · pair this display",
        true,
        pressed == Some(HitAction::Confirm),
    );
    hits.push((yes, HitAction::Confirm));
    let no = key(
        canvas,
        542.0,
        410.0,
        230.0,
        52.0,
        "Not my home",
        false,
        pressed == Some(HitAction::Reject),
    );
    hits.push((no, HitAction::Reject));
}

/// The expired pairing window state.
fn timeout(
    canvas: &mut FrameCanvas,
    pressed: Option<HitAction>,
    hits: &mut Vec<(HitRegion, HitAction)>,
) {
    deck(
        canvas,
        KICKER,
        "SETUP / RETRY",
        "Still waiting for your home.",
    );

    let mut expired = ts(SILKSCREEN, 12.0, GORGET_500, Weight::NORMAL);
    expired.line_height = 16.0;
    canvas.text(28.0, 167.0, "PAIRING REQUEST TIMED OUT", &expired);

    let mut headline = ts(JETBRAINS_MONO, 20.0, SAND_50, Weight::NORMAL);
    headline.line_height = 28.0;
    headline.max_width = 744.0;
    canvas.text(28.0, 213.0, "The example code has expired.", &headline);

    let mut body = ts(JETBRAINS_MONO, 14.0, SAND_300, Weight::NORMAL);
    body.line_height = 20.0;
    body.max_width = 744.0;
    canvas.text(
        28.0,
        266.0,
        "Keep the display and home on the same network.",
        &body,
    );
    canvas.text(28.0, 294.0, "Then request a new code and try again.", &body);

    let rearm = key(
        canvas,
        28.0,
        410.0,
        300.0,
        52.0,
        "Get a new code",
        true,
        pressed == Some(HitAction::Rearm),
    );
    hits.push((rearm, HitAction::Rearm));
    let help = key(
        canvas,
        542.0,
        410.0,
        230.0,
        52.0,
        "Connection help",
        false,
        pressed == Some(HitAction::Help),
    );
    hits.push((help, HitAction::Help));
}

/// The paired display state.
fn display(canvas: &mut FrameCanvas, text: &Option<String>) {
    canvas.fill(BG);
    let mut kicker = ts(SILKSCREEN, 12.0, PLUMAGE_500, Weight::NORMAL);
    kicker.line_height = 16.0;
    canvas.text(28.0, 22.0, "SCREENSIGHT", &kicker);

    let mut step = ts(JETBRAINS_MONO, 12.0, SAND_300, Weight::NORMAL);
    step.line_height = 16.0;
    step.max_width = 772.0;
    step.align = TextAlign::Right;
    canvas.text(0.0, 22.0, "DISPLAY", &step);

    match text {
        Some(text) if !text.is_empty() => {
            let large = text.chars().count() <= 42;
            let (family, px, line_height) = if large {
                (VT323, 56.0, 60.0)
            } else {
                (JETBRAINS_MONO, 24.0, 32.0)
            };
            let mut style = ts(family, px, SAND_50, Weight::NORMAL);
            style.line_height = line_height;
            style.max_width = 744.0;
            style.align = TextAlign::Left;
            let (_, h) = canvas.measure(text, &style);
            let y = ((crate::raster::HEIGHT as f32 - h) / 2.0).max(150.0);
            canvas.text(28.0, y, text, &style);
        }
        _ => {
            let rest = "3615 SCREENSIGHT / Rien à signaler";
            let mut style = ts(JETBRAINS_MONO, 16.0, SAND_300, Weight::NORMAL);
            style.line_height = 24.0;
            style.max_width = 744.0;
            style.align = TextAlign::Center;
            let (_, h) = canvas.measure(rest, &style);
            let y = ((crate::raster::HEIGHT as f32 - h) / 2.0).max(150.0);
            canvas.text(28.0, y, rest, &style);
        }
    }
}

/// The connection-help deck (Figma `18:1888`). Returned as its own frame so the
/// panel can overlay it without changing the runtime's pairing state.
#[must_use]
pub fn help_frame(pressed: Option<HitAction>) -> (FrameCanvas, Vec<(HitRegion, HitAction)>) {
    let mut canvas = FrameCanvas::new();
    let mut hits = Vec::new();
    deck(
        &mut canvas,
        KICKER,
        "SETUP / HELP",
        "Let's make the connection.",
    );

    let items: [(&str, &str, &str); 3] = [
        (
            "01",
            "Same local network",
            "Display and home server on the same network.",
        ),
        (
            "02",
            "Screensight integration",
            "Install and add the integration; select this display.",
        ),
        (
            "03",
            "Still not discovered?",
            "Check connectivity and discovery permissions.",
        ),
    ];
    let mut y = 162.0;
    for (number, title, body) in items {
        let mut n = ts(VT323, 30.0, GORGET_500, Weight::NORMAL);
        n.line_height = 34.0;
        canvas.text(28.0, y, number, &n);

        let mut t = ts(JETBRAINS_MONO, 17.0, SAND_50, Weight::BOLD);
        t.line_height = 22.0;
        t.max_width = 660.0;
        canvas.text(86.0, y, title, &t);

        let mut b = ts(JETBRAINS_MONO, 12.0, SAND_300, Weight::NORMAL);
        b.line_height = 16.0;
        b.max_width = 660.0;
        canvas.text(86.0, y + 29.0, body, &b);
        y += 73.0;
    }

    let back = key(
        &mut canvas,
        28.0,
        410.0,
        300.0,
        52.0,
        "← Back to pairing",
        false,
        pressed == Some(HitAction::BackToPairing),
    );
    hits.push((back, HitAction::BackToPairing));
    let retry = key(
        &mut canvas,
        542.0,
        410.0,
        230.0,
        52.0,
        "Retry connection",
        true,
        pressed == Some(HitAction::Retry),
    );
    hits.push((retry, HitAction::Retry));
    (canvas, hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_screens() -> Vec<Screen> {
        vec![
            Screen::Unpaired,
            Screen::NoHome,
            Screen::Pairing {
                code: "123456".to_owned(),
            },
            Screen::Confirm {
                code: "123456".to_owned(),
                ha_name: "My home".to_owned(),
            },
            Screen::Timeout,
            Screen::Display {
                text: Some("Hello world".to_owned()),
            },
            Screen::Display { text: None },
        ]
    }

    #[test]
    fn every_screen_renders_a_full_frame_with_content() {
        for screen in sample_screens() {
            let (canvas, _hits) = frame_for(&screen, "Screensight Studio");
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
    fn confirm_screen_hit_regions_map_to_actions() {
        let screen = Screen::Confirm {
            code: "123456".to_owned(),
            ha_name: "My home".to_owned(),
        };
        let (_, hits) = frame_for(&screen, "Test");
        let find = |action: HitAction| {
            hits.iter()
                .find(|(_, a)| *a == action)
                .map(|(r, _)| *r)
                .unwrap_or_else(|| panic!("missing hit region for {action:?}"))
        };
        let yes = find(HitAction::Confirm);
        let no = find(HitAction::Reject);
        assert!(yes.contains(yes.x + 1.0, yes.y + 1.0));
        assert!(no.contains(no.x + no.w - 1.0, no.y + no.h - 1.0));
        assert!(!yes.contains(no.x + 1.0, no.y + 1.0));
    }

    #[test]
    fn pairing_code_is_drawn_in_gorget_pixels() {
        let screen = Screen::Pairing {
            code: "123456".to_owned(),
        };
        let (canvas, _hits) = frame_for(&screen, "Test");
        // The pairing code is VT323 48px gorget-500 in the right column
        // (x 544..772, y 223..280).
        let mut found = false;
        for y in 223..290usize {
            for x in 544..772usize {
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
            "expected gorget-coloured code pixels in the right column"
        );
    }

    #[test]
    fn format_code_inserts_space() {
        assert_eq!(format_code("123456"), "123 456");
        assert_eq!(format_code("1234"), "12 34");
    }

    #[test]
    fn pairing_and_timeout_offer_help() {
        let (_, pairing) = frame_for(
            &Screen::Pairing {
                code: "123456".to_owned(),
            },
            "Test",
        );
        assert!(pairing.iter().any(|(_, a)| *a == HitAction::Help));
        let (_, timeout) = frame_for(&Screen::Timeout, "Test");
        assert!(timeout.iter().any(|(_, a)| *a == HitAction::Help));
    }

    #[test]
    fn help_screen_renders_and_its_keys_are_actionable() {
        let (canvas, hits) = help_frame(None);
        assert_eq!(
            canvas.frame().len(),
            crate::raster::WIDTH * crate::raster::HEIGHT * 4
        );
        let bg = BG.to_bgra();
        assert!(
            canvas.frame().as_chunks::<4>().0.iter().any(|px| px != &bg),
            "help screen drew no content"
        );
        assert!(hits.iter().any(|(_, a)| *a == HitAction::BackToPairing));
        assert!(hits.iter().any(|(_, a)| *a == HitAction::Retry));
    }
}
