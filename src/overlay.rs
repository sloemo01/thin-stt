use std::time::Duration;

use gpui::{
    div, prelude::*, pulsating_between, px, rgb, Animation, AnimationExt, Context, FontWeight,
    SharedString, Window,
};

pub const LEVEL_BARS: usize = 14;

pub struct OverlayView {
    pub title: SharedString,
    pub detail: SharedString,
    pub color: u32,
    /// Recent mic levels (0..1), newest last — drawn as a live meter while
    /// recording. Empty while transcribing/polishing.
    pub levels: [f32; LEVEL_BARS],
    pub live: bool,
}

impl OverlayView {
    pub fn new(title: impl Into<SharedString>, detail: impl Into<SharedString>, color: u32, live: bool) -> Self {
        OverlayView {
            title: title.into(),
            detail: detail.into(),
            color,
            levels: [0.0; LEVEL_BARS],
            live,
        }
    }

    pub fn push_level(&mut self, level: f32) {
        self.levels.rotate_left(1);
        self.levels[LEVEL_BARS - 1] = level;
    }
}

impl Render for OverlayView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Fill the whole window: macOS always paints its own rounded
        // material behind the content, so a floating pill inside shows a
        // mismatched second box. The window's own corner rounding shows.
        let indicator = if self.live {
            let mut bars = div().flex().items_center().gap(px(2.)).h(px(20.));
            for l in self.levels {
                // speech RMS sits around 0.02–0.2; sqrt spreads it visually
                let h = 3. + 17. * (l * 6.).sqrt().min(1.0);
                bars = bars.child(div().w(px(3.)).h(px(h)).rounded_full().bg(rgb(self.color)));
            }
            bars.into_any_element()
        } else {
            let mut dots = div().flex().items_center().gap(px(4.));
            for i in 0..3 {
                dots = dots.child(
                    div().size(px(5.)).rounded_full().bg(rgb(self.color)).with_animation(
                        gpui::ElementId::NamedInteger("busy".into(), i as u64),
                        Animation::new(Duration::from_millis(900 + 150 * i as u64))
                            .repeat()
                            .with_easing(pulsating_between(0.25, 1.0)),
                        |d, t| d.opacity(t),
                    ),
                );
            }
            dots.into_any_element()
        };

        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .bg(rgb(0x141418))
            .text_color(rgb(0xffffff))
            .font_family(".SystemUIFont")
            .child(
                div().size(px(8.)).rounded_full().bg(rgb(self.color)).with_animation(
                    "pulse",
                    Animation::new(Duration::from_millis(1200))
                        .repeat()
                        .with_easing(pulsating_between(0.35, 1.0)),
                    |d, t| d.opacity(t),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0x9898a6))
                            .truncate()
                            .child(self.detail.clone()),
                    ),
            )
            .child(indicator)
    }
}
