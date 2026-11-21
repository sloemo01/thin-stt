use gpui::{div, prelude::*, rgb, Context, SharedString, Window};

pub struct OverlayView {
    pub text: SharedString,
    pub color: u32,
}

impl OverlayView {
    pub fn new(text: impl Into<SharedString>, color: u32) -> Self {
        OverlayView {
            text: text.into(),
            color,
        }
    }
}

impl Render for OverlayView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Fill the whole window with the status color: macOS always paints
        // its own rounded material behind the content, so a floating pill
        // inside shows a mismatched second box. A full-window fill reads as
        // one solid colored toast; the window's own corner rounding shows.
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded_xl()
            .bg(rgb(self.color))
            .text_color(rgb(0xffffff))
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(self.text.clone())
    }
}
