//! GPUI GPU-acceleration PoC for a Raspberry Pi 4 (V3D / V3DV Vulkan).
//!
//! Everything you see is redrawn every frame through GPUI's Vulkan renderer:
//!
//!   * a field of orbiting rounded quads (procedural `canvas` paint),
//!   * a rotating ring of dots,
//!   * five bouncing balls,
//!   * a pulsing core,
//!   * a declarative animated overlay (bars),
//!   * GPU-rendered status text (see `font.rs`),
//!   * expanding touch ripples wherever the panel is touched.
//!
//! The `with_animation(..).repeat()` wrapper around the canvas is used as a
//! frame pump: it calls `Window::request_animation_frame()` forever, so GPUI
//! re-renders (and re-paints the canvas) on every vsync.
//!
//! Touch is read directly from the touchscreen's evdev device (see `touch.rs`)
//! because GPUI 0.2.2 has no Wayland `wl_touch` support.
//!
//! The scene can be trimmed at runtime for driver bisection, e.g.:
//!   GPUI_PARTICLES=0 GPUI_RING=0 GPUI_BALLS=0 GPUI_BARS=0 GPUI_TEXT=0
//!   GPUI_PUMP=0

use std::cell::{Cell, RefCell};
use std::f32::consts::TAU;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, App, Application, BorderStyle, Bounds, Context, Hsla, Render,
    Window, WindowBounds, WindowOptions, bounce, canvas, div, ease_in_out, hsla, linear, point,
    prelude::*, px, quad, size, transparent_black,
};

mod font;
mod touch;

/// Runtime-selectable scene configuration (see module docs).
#[derive(Clone, Copy)]
struct Config {
    particles: usize,
    ring: bool,
    balls: bool,
    bars: bool,
    text: bool,
    pump: bool,
}

impl Config {
    fn from_env() -> Self {
        fn flag(name: &str, default: bool) -> bool {
            std::env::var(name)
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(default)
        }
        Self {
            particles: std::env::var("GPUI_PARTICLES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1200),
            ring: flag("GPUI_RING", true),
            balls: flag("GPUI_BALLS", true),
            bars: flag("GPUI_BARS", true),
            text: flag("GPUI_TEXT", true),
            pump: flag("GPUI_PUMP", true),
        }
    }
}

/// A cheap deterministic hash -> [0.0, 1.0).
fn hash01(i: usize, salt: u32) -> f32 {
    let mut x = (i as u32)
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add(salt.wrapping_mul(0x85EB_CA77));
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x & 0x00FF_FFFF) as f32 / 0x0100_0000 as f32
}

/// Lifetime of a touch ripple, in seconds.
const RIPPLE_LIFETIME: f32 = 1.1;

/// A touch ripple, stored in normalized `[0, 1]` screen coordinates.
struct Ripple {
    fx: f32,
    fy: f32,
    born: Instant,
    hue: f32,
}

struct Scene {
    started: Instant,
    config: Config,
    /// Smoothed frames-per-second, measured between canvas paints.
    fps: Rc<Cell<f32>>,
    /// Time (seconds) of the previous paint, used to compute frame deltas.
    last_paint: Rc<Cell<f64>>,
    /// Active touch ripples.
    ripples: Rc<RefCell<Vec<Ripple>>>,
    /// Touch samples streamed from the evdev reader thread.
    touch_queue: Arc<Mutex<Vec<touch::TouchSample>>>,
}

impl Scene {
    fn new() -> Self {
        let touch_queue = Arc::new(Mutex::new(Vec::new()));
        touch::spawn_reader(touch_queue.clone());
        Self {
            started: Instant::now(),
            config: Config::from_env(),
            fps: Rc::new(Cell::new(0.0)),
            last_paint: Rc::new(Cell::new(0.0)),
            ripples: Rc::new(RefCell::new(Vec::new())),
            touch_queue,
        }
    }
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config;
        let started = self.started;
        let fps = self.fps.clone();
        let last_paint = self.last_paint.clone();
        let ripples = self.ripples.clone();
        let touch_queue = self.touch_queue.clone();

        // ----- the procedural, every-frame GPU scene -----------------------
        let scene_canvas = canvas(
            |_bounds: Bounds<gpui::Pixels>, _window: &mut Window, _cx: &mut App| {},
            move |bounds: Bounds<gpui::Pixels>, _state: (), window: &mut Window, _cx: &mut App| {
                let now = started.elapsed().as_secs_f64();
                let prev = last_paint.get();
                if prev > 0.0 {
                    let dt = (now - prev) as f32;
                    if dt > 0.0 {
                        let instantaneous = 1.0 / dt;
                        let smoothed = fps.get();
                        fps.set(if smoothed == 0.0 {
                            instantaneous
                        } else {
                            smoothed * 0.9 + instantaneous * 0.1
                        });
                    }
                }
                last_paint.set(now);

                // Ingest touch samples from the evdev reader as ripples.
                if let Ok(mut queue) = touch_queue.lock() {
                    for sample in queue.drain(..) {
                        let hue = (sample.fx * 1.7 + sample.fy * 0.9).fract();
                        ripples.borrow_mut().push(Ripple {
                            fx: sample.fx,
                            fy: sample.fy,
                            born: Instant::now(),
                            hue,
                        });
                    }
                }

                paint_scene(
                    window,
                    bounds,
                    started.elapsed().as_secs_f32(),
                    config,
                    fps.get(),
                    &ripples,
                );
            },
        )
        .size_full();

        let scene_canvas = if config.pump {
            // Infinite, no-op animation used as a frame pump.
            scene_canvas
                .with_animation(
                    "scene-frame-pump",
                    Animation::new(Duration::from_secs(3600)).repeat(),
                    |el, _delta| el,
                )
                .into_any_element()
        } else {
            scene_canvas.into_any_element()
        };

        let mut root = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(hsla(0.66, 0.45, 0.06, 1.0))
            .child(scene_canvas);

        // ----- declarative animated overlay --------------------------------
        if config.bars {
            root = root.child(
                div()
                    .absolute()
                    .top(px(10.))
                    .left(px(10.))
                    .flex()
                    .gap_2()
                    .children((0..8usize).map(|i| {
                        let duration = 700 + i as u64 * 160;
                        let easing: Box<dyn Fn(f32) -> f32> = match i % 3 {
                            0 => Box::new(bounce(ease_in_out)),
                            1 => Box::new(ease_in_out),
                            _ => Box::new(linear),
                        };
                        div()
                            .id(i + 1)
                            .size(px(22.))
                            .rounded_md()
                            .bg(hsla(i as f32 / 8.0, 0.85, 0.6, 0.95))
                            .with_animation(
                                i + 100,
                                Animation::new(Duration::from_millis(duration))
                                    .repeat()
                                    .with_easing(move |d| easing(d)),
                                move |el, delta| el.top(px(70.0 * delta)),
                            )
                    })),
            );
        }

        root
    }
}

/// Paint the whole scene into the current frame.
fn paint_scene(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    t: f32,
    config: Config,
    fps: f32,
    ripples: &Rc<RefCell<Vec<Ripple>>>,
) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let cx = ox + w * 0.5;
    let cy = oy + h * 0.5;

    let draw_disc = |window: &mut Window, x: f32, y: f32, diameter: f32, color: Hsla| {
        let radius = diameter * 0.5;
        let b = Bounds {
            origin: point(px(x - radius), px(y - radius)),
            size: size(px(diameter), px(diameter)),
        };
        window.paint_quad(quad(
            b,
            px(radius),
            color,
            px(0.),
            transparent_black(),
            BorderStyle::default(),
        ));
    };

    // --- orbiting particle field ------------------------------------------
    for i in 0..config.particles {
        let a0 = hash01(i, 1) * TAU;
        let speed = 0.10 + hash01(i, 2) * 0.55; // revolutions / second
        let base_radius = 30.0 + hash01(i, 3) * 165.0;
        let squish = 0.55 + hash01(i, 4) * 0.45;
        let diameter = 2.0 + hash01(i, 5) * 9.0;
        let alpha = 0.20 + hash01(i, 6) * 0.75;
        let hue0 = hash01(i, 7);
        let wobble = 5.0 + hash01(i, 8) * 22.0;
        let wobble_speed = 0.4 + hash01(i, 9) * 1.4;

        let angle = a0 + t * TAU * speed;
        let radius = base_radius + wobble * (t * TAU * wobble_speed + a0).sin();
        let x = cx + angle.cos() * radius;
        let y = cy + angle.sin() * radius * squish;

        let hue = (hue0 + t * 0.05).fract();
        draw_disc(window, x, y, diameter, hsla(hue, 0.85, 0.62, alpha));
    }

    // --- rotating ring of dots --------------------------------------------
    if config.ring {
        let ring_radius = (h * 0.42).min(w * 0.28);
        for k in 0..48 {
            let a = t * TAU * 0.75 + k as f32 * TAU / 48.0;
            let x = cx + a.cos() * ring_radius;
            let y = cy + a.sin() * ring_radius;
            let hue = (k as f32 / 48.0 + t * 0.1).fract();
            draw_disc(window, x, y, 9.0, hsla(hue, 0.9, 0.65, 0.9));
        }
    }

    // --- bouncing balls ----------------------------------------------------
    if config.balls {
        for i in 0..5 {
            let fi = i as f32;
            let ball_x = ox + w * (fi + 1.0) / 6.0;
            let amplitude = h * 0.5 - 34.0;
            let ball_y = cy + amplitude * (t * TAU * (0.55 + fi * 0.17) + fi * 1.3).sin();
            let diameter = 40.0 + fi * 6.0;
            draw_disc(
                window,
                ball_x,
                ball_y,
                diameter,
                hsla((fi / 5.0 + t * 0.08).fract(), 0.9, 0.6, 0.95),
            );
        }
    }

    // --- pulsing core ------------------------------------------------------
    let pulse = 46.0 + 18.0 * (t * TAU * 0.5).sin();
    draw_disc(window, cx, cy, pulse, hsla((0.5 + t * 0.1).fract(), 0.85, 0.7, 0.85));

    // --- on-screen status text, drawn as GPU quads (see src/font.rs) -------
    if config.text {
        let scale = 1.0;
        let text = format!(
            "GPUI + VULKAN ON PI4 (V3D)   {} PARTICLES   {:.0} FPS",
            config.particles, fps
        )
        .to_uppercase();
        let text_y = oy + h - 12.0 - font::FONT_H as f32 * scale;
        draw_text(window, ox + 12.0, text_y, scale, &text, gpui::white());
    }

    // --- touch ripples -----------------------------------------------------
    let now = Instant::now();
    let mut active = ripples.borrow_mut();
    active.retain(|r| now.duration_since(r.born).as_secs_f32() < RIPPLE_LIFETIME);
    for r in active.iter() {
        let age = now.duration_since(r.born).as_secs_f32();
        let p = (age / RIPPLE_LIFETIME).clamp(0.0, 1.0);
        let x = ox + r.fx * w;
        let y = oy + r.fy * h;

        // Three staggered expanding rings of dots.
        for ring in 0..3u32 {
            let rp = p - ring as f32 * 0.14;
            if rp <= 0.0 {
                continue;
            }
            let radius = 6.0 + rp * 82.0;
            let alpha = (1.0 - p) * (1.0 - ring as f32 * 0.3) * 0.9;
            let dots = 40;
            for k in 0..dots {
                let ang = k as f32 / dots as f32 * TAU;
                draw_disc(
                    window,
                    x + ang.cos() * radius,
                    y + ang.sin() * radius,
                    4.5,
                    hsla(r.hue, 0.9, 0.65, alpha),
                );
            }
        }

        // Bright core that fades out.
        draw_disc(
            window,
            x,
            y,
            (1.0 - p) * 30.0,
            hsla(r.hue, 0.9, 0.85, (1.0 - p) * 0.8),
        );
    }
}

/// Blit `text` with the embedded bitmap font, one quad per run of set pixels.
fn draw_text(window: &mut Window, x: f32, y: f32, scale: f32, text: &str, color: Hsla) {
    let advance = (font::FONT_W as f32 + 1.0) * scale;
    let mut cursor = x;
    for ch in text.chars() {
        draw_char(window, cursor, y, scale, ch, color);
        cursor += advance;
    }
}

fn draw_char(window: &mut Window, x: f32, y: f32, scale: f32, ch: char, color: Hsla) {
    let code = ch as u32;
    if !(font::FONT_FIRST as u32..=font::FONT_LAST as u32).contains(&code) {
        return;
    }
    let glyph = &font::GLYPHS[(code - font::FONT_FIRST as u32) as usize];
    for (row, &bits) in glyph.iter().enumerate() {
        let mut col = 0usize;
        while col < font::FONT_W {
            let on = bits & (1u8 << (font::FONT_W - 1 - col)) != 0;
            if !on {
                col += 1;
                continue;
            }
            let start = col;
            while col < font::FONT_W && (bits & (1u8 << (font::FONT_W - 1 - col))) != 0 {
                col += 1;
            }
            let run = (col - start) as f32;
            let b = Bounds {
                origin: point(
                    px(x + start as f32 * scale),
                    px(y + row as f32 * scale),
                ),
                size: size(px(run * scale), px(scale)),
            };
            window.paint_quad(quad(
                b,
                px(0.),
                color,
                px(0.),
                transparent_black(),
                BorderStyle::default(),
            ));
        }
    }
}

fn main() {
    env_logger::init();
    Application::new().run(|cx: &mut App| {
        // The kiosk panel is an 800x480 DSI display; fill it from the origin.
        let bounds = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(800.), px(480.)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: true,
                ..Default::default()
            },
            |_window, cx| cx.new(|_cx| Scene::new()),
        )
        .unwrap();

        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
