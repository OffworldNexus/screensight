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
    div, App, Application, Bounds, Context, Corners, IntoElement, MouseButton, MouseDownEvent,
    Render, RenderImage, Window, WindowBounds, WindowOptions,
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
pub fn run(runtime: Arc<Runtime>, model: String) -> anyhow::Result<()> {
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
        let view = cx.new(|cx| Panel::new(rx, runtime, model, touches, cx));
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

/// Cached frame for a given view (screen + pressed key + help overlay).
type FrameCache = Option<(Screen, Option<HitAction>, bool, Arc<RenderImage>)>;

struct Panel {
    screen: Screen,
    model: String,
    runtime: Arc<Runtime>,
    hits: Vec<(HitRegion, HitAction)>,
    cache: FrameCache,
    pressed: Option<(HitAction, Instant)>,
    /// Whether the connection-help overlay is showing instead of the screen.
    show_help: bool,
}

impl Panel {
    fn new(
        rx: UnboundedReceiver<Screen>,
        runtime: Arc<Runtime>,
        model: String,
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
                        // A runtime state change always leaves the help overlay.
                        panel.show_help = false;
                        panel.cache = None;
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

        // Soft 1 Hz heartbeat only while awaiting a Home Assistant, so the
        // pairing deck stays live without a 60 fps render loop.
        let heartbeat_executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| loop {
            heartbeat_executor.timer(Duration::from_secs(1)).await;
            if this
                .update(cx, |panel, cx| {
                    if matches!(
                        panel.screen,
                        Screen::Pairing { .. } | Screen::Confirm { .. }
                    ) {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();

        Self {
            screen: Screen::NoHome,
            model,
            runtime,
            hits: Vec::new(),
            cache: None,
            pressed: None,
            show_help: false,
        }
    }

    /// Build (or reuse) the `RenderImage` for the current view.
    fn render_image(&mut self) -> Option<Arc<RenderImage>> {
        let pressed = self.pressed.map(|(action, _)| action);
        if let Some((screen, cached_pressed, cached_help, image)) = &self.cache {
            if screen == &self.screen
                && *cached_pressed == pressed
                && *cached_help == self.show_help
            {
                return Some(image.clone());
            }
        }
        let (canvas, hits) = if self.show_help {
            screens::help_frame(pressed)
        } else {
            screens::frame_for_with(&self.screen, &self.model, pressed)
        };
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
        self.cache = Some((self.screen.clone(), pressed, self.show_help, image.clone()));
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
            HitAction::Confirm => {
                if let Err(err) = self.runtime.confirm_pairing() {
                    log::warn!("panel: confirm_pairing failed: {err:#}");
                }
            }
            HitAction::Reject => {
                if let Err(err) = self.runtime.reject_pairing() {
                    log::warn!("panel: reject_pairing failed: {err:#}");
                }
            }
            HitAction::Rearm => {
                if let Err(err) = self.runtime.arm_pairing() {
                    log::warn!("panel: arm_pairing failed: {err:#}");
                }
            }
            // Open the designed help deck; it replaces the current screen until
            // the user goes back or the runtime state changes.
            HitAction::Help => self.show_help = true,
            HitAction::BackToPairing => self.show_help = false,
            HitAction::Retry => {
                if let Err(err) = self.runtime.arm_pairing() {
                    log::warn!("panel: arm_pairing failed: {err:#}");
                }
                self.show_help = false;
            }
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let image = self.render_image();
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
                    move |_bounds, _window, _cx| image,
                    move |bounds, image, window, _cx| {
                        if let Some(image) = image {
                            let corners = Corners::all(gpui::px(0.0));
                            if let Err(err) = window.paint_image(bounds, corners, image, 0, false) {
                                log::warn!("panel: paint_image failed: {err:#}");
                            }
                        }
                    },
                )
                .size_full(),
            )
    }
}
