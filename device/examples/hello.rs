// Minimal static GPUI window used to bisect a rendering problem: no canvas,
// no animation, no text atlas churn. If this is clean but the main app is not,
// the issue is in the animated canvas scene.

use gpui::{
    div, point, prelude::*, px, rgb, size, App, Application, Bounds, Context, Render, Window,
    WindowBounds, WindowOptions,
};

struct Hello;

impl Render for Hello {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x202040))
            .justify_center()
            .items_center()
            .child(div().size(px(200.)).bg(rgb(0xff0000)).rounded_md())
            .child(div().size(px(120.)).bg(rgb(0x00ff00)))
            .child(div().text_color(rgb(0xffffff)).child("HELLO GPUI STATIC"))
    }
}

fn main() {
    env_logger::init();
    Application::new().run(|cx: &mut App| {
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
            |_window, cx| cx.new(|_cx| Hello),
        )
        .unwrap();
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
