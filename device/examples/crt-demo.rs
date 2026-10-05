//! Visual smoke test for the vendored-gpui CRT screen transition.
//!
//! Runs the real `paint_crt_transition` shader path against two generated
//! frames on whatever GPU the host has, looping the transition for a few
//! seconds. It exists to exercise blade's pipeline creation / struct-size
//! assert and to eyeball the effect:
//!
//! ```sh
//! cargo run -p screensight --features gui --example crt-demo
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, App, Application, Bounds, Context, IntoElement, Render, RenderImage, Window, WindowBounds,
    WindowOptions,
};

use screensight::raster::{HEIGHT, WIDTH};

/// Seconds for one full A→B transition.
const LOOP_SECS: f32 = 2.0;
/// How long the demo stays on screen before quitting.
const TOTAL_SECS: f32 = 8.0;

struct CrtDemo {
    stacked: Arc<RenderImage>,
    start: Instant,
}

/// Build a distinct 800×480 BGRA pattern so the CRT distortion is obvious.
fn make_frame(seed: u8) -> Arc<RenderImage> {
    let mut bytes = vec![0u8; WIDTH * HEIGHT * 4];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = (y * WIDTH + x) * 4;
            // Gradient plus a grid, in BGRA order (matching GPUI's atlas).
            let grid = if x % 64 < 2 || y % 64 < 2 { 70 } else { 0 };
            let b = (x as u32 * 255 / WIDTH as u32) as u8;
            let g = (y as u32 * 255 / HEIGHT as u32) as u8;
            bytes[i] = b.saturating_add(seed).saturating_add(grid);
            bytes[i + 1] = g.saturating_add(grid);
            bytes[i + 2] = seed.saturating_add(grid);
            bytes[i + 3] = 255;
        }
    }
    let buffer =
        image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(WIDTH as u32, HEIGHT as u32, bytes)
            .expect("valid frame buffer");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

impl CrtDemo {
    fn new(cx: &mut Context<Self>) -> Self {
        let start = Instant::now();
        let executor = cx.background_executor().clone();
        // ~60 fps repaint pump, then quit.
        cx.spawn(async move |this, cx| loop {
            executor.timer(Duration::from_millis(16)).await;
            let done = this
                .update(cx, |_demo, cx| {
                    cx.notify();
                    false
                })
                .unwrap_or(true);
            if done {
                break;
            }
        })
        .detach();

        let quit_executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            quit_executor
                .timer(Duration::from_secs_f32(TOTAL_SECS))
                .await;
            let _ = this.update(cx, |_demo, cx| cx.quit());
        })
        .detach();

        Self {
            stacked: screensight::panel::stacked_frame(&make_frame(0x20), &make_frame(0xc0))
                .expect("stacked demo frames"),
            start,
        }
    }
}

impl Render for CrtDemo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let elapsed = self.start.elapsed().as_secs_f32();
        // Triangle wave so it sweeps A→B→A.
        let phase = (elapsed / LOOP_SECS).fract();
        let progress = if phase < 0.5 {
            phase * 2.0
        } else {
            2.0 - phase * 2.0
        };
        let stacked = self.stacked.clone();

        div().size_full().child(
            gpui::canvas(
                move |_bounds, _window, _cx| stacked,
                move |bounds, stacked, window, _cx| {
                    if let Err(err) = window.paint_crt_transition(bounds, stacked, progress) {
                        log::error!("crt-demo: paint_crt_transition failed: {err:#}");
                    }
                },
            )
            .size_full(),
        )
    }
}

fn main() {
    env_logger::init();
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds {
            origin: gpui::point(gpui::px(0.), gpui::px(0.)),
            size: gpui::size(gpui::px(WIDTH as f32), gpui::px(HEIGHT as f32)),
        };
        let view = cx.new(CrtDemo::new);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: true,
                ..Default::default()
            },
            move |_window, _cx| view,
        )
        .expect("open crt-demo window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
