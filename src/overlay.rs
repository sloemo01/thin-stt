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
        div()
            .flex()
            .size_full()
            .flex_col()
            .justify_center()
            .items_center()
            .child(
                div()
                    .px_4()
                    .py_2()
                    .rounded_full()
                    .bg(rgb(self.color))
                    .shadow_lg()
                    .text_color(rgb(0xffffff))
                    .text_sm()
                    .child(self.text.clone()),
            )
    }
}
