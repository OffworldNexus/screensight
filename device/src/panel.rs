//! GPUI panel shell for the physical Screensight display.
//!
//! GPUI's glyph-atlas text pipeline hangs the Pi's V3D GPU, so this module
//! never lets GPUI draw text. It receives the current [`Screen`], CPU-composes
//! a full BGRA frame with [`crate::screens`], wraps it in a [`RenderImage`] and
//! blits it as a single image. The frame and image are cached until the screen
//! (or a pressed key) changes.
//!
//! Touch comes from the direct evdev reader in [`crate::touch`], because GPUI
//! 0.2.2 receives no touch events under cage. Samples are polled on a soft
//! (25 Hz) timer and hit-tested against the screen's key regions.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, App, Application, Bounds, Context, Corners, CursorStyle, IntoElement, MouseButton,
    MouseDownEvent, Render, RenderImage, Window, WindowBounds, WindowOptions,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::raster::{HEIGHT, WIDTH};
use crate::runtime::{Runtime, Screen};
use crate::screens::{self, HitAction, HitRegion};
use crate::touch::{self, TouchSample};

/// The channel to the running GPUI app, populated when [`run`] starts.
static SCREEN_TX: OnceLock<UnboundedSender<Screen>> = OnceLock::new();

/// Latest screen handed to [`set_screen`] before the app started.
static LATEST: Mutex<Option<Screen>> = Mutex::new(None);

/// Hand the latest screen to the panel. Safe to call from any thread; if the
/// GPUI app is not running yet the value is stored and replayed at startup.
pub fn set_screen(screen: Screen) {
    match SCREEN_TX.get() {
        Some(tx) => {
            let _ = tx.send(screen);
        }
        None => {
            *LATEST.lock().expect("panel latest mutex poisoned") = Some(screen);
        }
    }
}

/// Run the GPUI panel on the calling (main) thread until the window closes.
pub fn run(runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<Screen>();
    let _ = SCREEN_TX.set(tx.clone());
    if let Some(pending) = LATEST.lock().expect("panel latest mutex poisoned").take() {
        let _ = tx.send(pending);
    }

    let touches = Arc::new(Mutex::new(Vec::<TouchSample>::new()));
    touch::spawn_reader(touches.clone());

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds {
            origin: gpui::point(gpui::px(0.), gpui::px(0.)),
            size: gpui::size(gpui::px(WIDTH as f32), gpui::px(HEIGHT as f32)),
        };
        let view = cx.new(|cx| Panel::new(rx, runtime, touches, cx));
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: true,
                ..Default::default()
            },
            move |_window, _cx| view,
        );
        if let Err(err) = opened {
            log::error!("failed to open panel window: {err:#}");
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
    Ok(())
}

/// Default duration of the CRT screen-change transition (seconds). Override at
/// runtime with `SCREENSIGHT_TRANSITION_MS` to tune the feel without rebuilding.
const DEFAULT_TRANSITION_SECS: f32 = 0.4;

/// The transition duration: `SCREENSIGHT_TRANSITION_MS` (clamped to 50 ms–10 s)
/// or [`DEFAULT_TRANSITION_SECS`].
fn transition_secs() -> f32 {
    static SECS: OnceLock<f32> = OnceLock::new();
    *SECS.get_or_init(|| {
        std::env::var("SCREENSIGHT_TRANSITION_MS")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .map(|ms| (ms / 1000.0).clamp(0.05, 10.0))
            .unwrap_or(DEFAULT_TRANSITION_SECS)
    })
}

/// Whether to hide the mouse pointer. The device's systemd unit sets
/// `SCREENSIGHT_HIDE_CURSOR`; the desktop emulator leaves it unset so the
/// pointer stays visible for clicking.
fn hide_cursor() -> bool {
    static HIDE: OnceLock<bool> = OnceLock::new();
    *HIDE.get_or_init(|| std::env::var_os("SCREENSIGHT_HIDE_CURSOR").is_some())
}

/// Cached frame for a given view (screen + pressed key).
type FrameCache = Option<(Screen, Option<HitAction>, Arc<RenderImage>)>;

/// Stack two same-size frames into a single image: `top` on top, `bottom`
/// below. The CRT transition samples both halves from this one atlas tile, which
/// is what keeps the effect working once the sprite atlas has several pages.
pub fn stacked_frame(top: &RenderImage, bottom: &RenderImage) -> Option<Arc<RenderImage>> {
    let top_bytes = top.as_bytes(0)?;
    let bottom_bytes = bottom.as_bytes(0)?;
    if top_bytes.len() != bottom_bytes.len() {
        return None;
    }
    let mut bytes = Vec::with_capacity(top_bytes.len() * 2);
    bytes.extend_from_slice(top_bytes);
    bytes.extend_from_slice(bottom_bytes);
    let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        WIDTH as u32,
        (HEIGHT * 2) as u32,
        bytes,
    )?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

struct Panel {
    screen: Screen,
    runtime: Arc<Runtime>,
    hits: Vec<(HitRegion, HitAction)>,
    cache: FrameCache,
    pressed: Option<(HitAction, Instant)>,
    /// Outgoing frame kept alive while a screen-change transition runs.
    prev_image: Option<Arc<RenderImage>>,
    /// Stacked outgoing+incoming frame uploaded for the CRT transition.
    transition_image: Option<Arc<RenderImage>>,
    /// Wall-clock start of the running transition, if any.
    transition_start: Option<Instant>,
    /// Last screen, used to detect view changes to animate.
    last_view: Option<Screen>,
}

impl Panel {
    fn new(
        rx: UnboundedReceiver<Screen>,
        runtime: Arc<Runtime>,
        touches: Arc<Mutex<Vec<TouchSample>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Screen updates: await the runtime channel and repaint.
        let mut rx = rx;
        cx.spawn(async move |this, cx| {
            while let Some(screen) = rx.recv().await {
                if this
                    .update(cx, |panel, cx| {
                        panel.screen = screen;
                        // The frame cache is left in place so `render` can still
                        // see the previous frame and animate to the new one.
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        // Touch polling: the evdev thread pushes samples, we drain and dispatch.
        let touch_executor = cx.background_executor().clone();
        let touch_queue = touches.clone();
        cx.spawn(async move |this, cx| loop {
            touch_executor.timer(Duration::from_millis(40)).await;
            let samples = {
                let mut queue = touch_queue.lock().expect("touch queue poisoned");
                std::mem::take(&mut *queue)
            };
            let Some(sample) = samples.last().copied() else {
                continue;
            };
            let x = sample.fx * WIDTH as f32;
            let y = sample.fy * HEIGHT as f32;
            if this
                .update(cx, |panel, cx| panel.on_touch(x, y, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();

        // Frame pump: repaint at ~60 fps *only* while a screen transition is
        // running. Spawned once here rather than from `render`, where the
        // spawned task would not reliably drive repaints.
        let pump_executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| loop {
            pump_executor.timer(Duration::from_millis(16)).await;
            if this
                .update(cx, |panel, cx| {
                    if panel.transition_start.is_some() {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();

        // Start on exactly what the runtime currently wants (usually the
        // splash on a paired boot) so no stale frame flashes before the first
        // `set_screen` arrives.
        let initial = runtime.tick(Instant::now());
        Self {
            screen: initial,
            runtime,
            hits: Vec::new(),
            cache: None,
            pressed: None,
            prev_image: None,
            transition_image: None,
            transition_start: None,
            last_view: None,
        }
    }

    /// Build (or reuse) the `RenderImage` for the current screen.
    fn render_image(&mut self) -> Option<Arc<RenderImage>> {
        let pressed = self.pressed.map(|(action, _)| action);
        if let Some((screen, cached_pressed, image)) = &self.cache {
            if screen == &self.screen && *cached_pressed == pressed {
                return Some(image.clone());
            }
        }
        let (canvas, hits) = screens::frame_for_with(&self.screen, pressed);
        self.hits = hits;
        // GPUI's atlas wants BGRA bytes; its own loader stores them in an
        // `Rgba`-typed buffer after swapping R/B. We feed the BGRA bytes in
        // directly, exactly as that loader leaves them.
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
            WIDTH as u32,
            HEIGHT as u32,
            canvas.frame().to_vec(),
        )?;
        let frame = image::Frame::new(buffer);
        let image = Arc::new(RenderImage::new(vec![frame]));
        self.cache = Some((self.screen.clone(), pressed, image.clone()));
        Some(image)
    }

    /// Hit-test a touch in frame pixels and dispatch the matching action.
    fn on_touch(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        let action = self
            .hits
            .iter()
            .find(|(region, _)| region.contains(x, y))
            .map(|(_, action)| *action);
        let Some(action) = action else {
            return;
        };

        match action {
            HitAction::StartPairing => {
                if let Err(err) = self.runtime.arm_pairing() {
                    log::warn!("panel: arm_pairing failed: {err:#}");
                }
            }
            HitAction::Confirm => {
                if let Err(err) = self.runtime.confirm_pairing() {
                    log::warn!("panel: confirm_pairing failed: {err:#}");
                }
            }
            HitAction::Decline => self.runtime.reject_pairing(),
        }

        // Press feedback: highlight the key for a beat.
        self.pressed = Some((action, Instant::now()));
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(80)).await;
            let _ = this.update(cx, |panel, cx| {
                panel.pressed = None;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Detect a screen change and animate it. Pressed highlights also rebuild
        // the frame, but must not trigger a transition.
        let view_changed = self.last_view.as_ref() != Some(&self.screen);
        let old_cache = self.cache.as_ref().map(|c| c.2.clone());

        if view_changed {
            // Keep the outgoing frame so we can stack it with the incoming one.
            self.prev_image = old_cache.clone();
            self.transition_image = None;
            self.transition_start = Some(Instant::now());
            self.last_view = Some(self.screen.clone());
        }

        let image = self.render_image();

        // Build the stacked CRT frame once, from the outgoing and incoming
        // frames. Its pixels live in the stacked image, so release the original
        // outgoing frame afterwards.
        if self.transition_start.is_some() && self.transition_image.is_none() {
            if let (Some(prev), Some(cur)) = (self.prev_image.clone(), image.clone()) {
                self.transition_image = stacked_frame(&prev, &cur);
            }
            self.prev_image = None;
        }

        // Drop any cached frame that is neither the settled image nor the
        // stacked transition frame, so the sprite atlas stays small.
        if let Some(old) = &old_cache {
            let still_current = image.as_ref().is_some_and(|i| Arc::ptr_eq(i, old));
            let is_transition = self
                .transition_image
                .as_ref()
                .is_some_and(|t| Arc::ptr_eq(t, old));
            if !still_current && !is_transition {
                let _ = window.drop_image(old.clone());
            }
        }

        // Finish the transition once it has run its course.
        let secs = transition_secs();
        if let Some(elapsed) = self.transition_start.map(|t| t.elapsed().as_secs_f32()) {
            if elapsed / secs >= 1.0 {
                if let Some(combined) = self.transition_image.take() {
                    let _ = window.drop_image(combined);
                }
                self.transition_start = None;
            }
        }

        let transition_image = self.transition_image.clone();
        let progress = self
            .transition_start
            .map(|t| (t.elapsed().as_secs_f32() / secs).clamp(0.0, 1.0));

        // Mouse/pointer input is what makes the desktop ("emulated Pi") panel
        // clickable: the physical device has no pointer events under cage, so it
        // uses the evdev reader instead, but both funnel into `on_touch`.
        div()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|panel, event: &MouseDownEvent, _window, cx| {
                    let x: f32 = event.position.x.into();
                    let y: f32 = event.position.y.into();
                    panel.on_touch(x, y, cx);
                }),
            )
            .child(
                gpui::canvas(
                    move |_bounds, _window, _cx| (image, transition_image, progress),
                    move |bounds, (image, transition_image, progress), window, _cx| {
                        // On the device (kiosk, no mouse) hide the pointer: cage
                        // lacks `cursor-shape-v1`, so GPUI would otherwise set its
                        // own client cursor that overrides the blank compositor
                        // theme. The desktop emulator leaves this unset so the
                        // pointer stays usable for clicking.
                        if hide_cursor() {
                            window.set_window_cursor_style(CursorStyle::None);
                        }
                        let corners = Corners::all(gpui::px(0.0));
                        match (image, transition_image, progress) {
                            (Some(_image), Some(combined), Some(p)) if p < 1.0 => {
                                if let Err(err) = window.paint_crt_transition(bounds, combined, p) {
                                    log::warn!("panel: paint_crt_transition failed: {err:#}");
                                }
                            }
                            (Some(image), _, _) => {
                                if let Err(err) =
                                    window.paint_image(bounds, corners, image, 0, false)
                                {
                                    log::warn!("panel: paint_image failed: {err:#}");
                                }
                            }
                            _ => {}
                        }
                    },
                )
                .size_full(),
            )
    }
}
