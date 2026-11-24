use std::time::Duration;

use gpui::{
    div, prelude::*, pulsating_between, px, rgb, rgba,
    Animation, AnimationExt, Context, FontWeight, SharedString, Window,
};

/// Level history; the waveform mirrors it outward from the centre bar.
pub const LEVEL_BARS: usize = 11;
const WAVE_BARS: usize = LEVEL_BARS * 2 - 1;
const WAVE_H: f32 = 40.;
const BAR_W: f32 = 5.;
const BAR_MIN: f32 = 6.;
const SHIMMER_MS: u64 = 1400;

pub struct OverlayView {
    pub title: SharedString,
    pub detail: SharedString,
    pub color: u32,
    /// Recent input levels (0..1), newest last — drawn as a live waveform
    /// while recording. Unused while transcribing.
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
        // Ease toward the new level so the wave breathes instead of flickering.
        let prev = self.levels[LEVEL_BARS - 1];
        self.levels.rotate_left(1);
        self.levels[LEVEL_BARS - 1] = prev * 0.35 + level * 0.65;
    }
}

/// Bell envelope so the outer bars stay shorter, like a voice-memo wave.
fn envelope(i: usize) -> f32 {
    let d = (i as f32 - (WAVE_BARS / 2) as f32).abs() / (WAVE_BARS / 2) as f32;
    1.0 - 0.55 * d * d
}

fn bar(h: f32, alpha: f32) -> gpui::Div {
    div()
        .w(px(BAR_W))
        .h(px(h))
        .rounded_full()
        .bg(gpui::Hsla { h: 0., s: 0., l: 0.98, a: alpha })
}

impl Render for OverlayView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut wave = div().flex().items_center().justify_center().gap(px(4.)).h(px(WAVE_H));
        if self.live {
            for i in 0..WAVE_BARS {
                let age = (i as isize - (WAVE_BARS / 2) as isize).unsigned_abs();
                let l = self.levels[LEVEL_BARS - 1 - age];
                // speech RMS sits around 0.02–0.2; sqrt spreads it visually
                let amp = (l * 6.).sqrt().min(1.0) * envelope(i);
                // a little per-bar texture so neighbours never match exactly
                let texture = 0.8 + 0.2 * ((i * 7 % 5) as f32 / 4.);
                wave = wave.child(bar(BAR_MIN + (WAVE_H - BAR_MIN) * amp * texture, 0.55 + 0.45 * amp));
            }
        } else {
            // Transcribing: a bright crest sweeps across a resting wave.
            for i in 0..WAVE_BARS {
                let pos = i as f32 / (WAVE_BARS - 1) as f32;
                let rest = BAR_MIN + 6. * envelope(i);
                wave = wave.child(
                    bar(rest, 0.25).with_animation(
                        gpui::ElementId::NamedInteger("shimmer-bar".into(), i as u64),
                        Animation::new(Duration::from_millis(SHIMMER_MS)).repeat(),
                        move |d, t| {
                            let x = (t * 1.6 - 0.3 - pos).abs();
                            let crest = (1.0 - x / 0.22).max(0.0);
                            let crest = crest * crest * (3.0 - 2.0 * crest);
                            d.h(px(rest + 20. * crest * envelope(i)))
                                .bg(gpui::Hsla { h: 0., s: 0., l: 0.98, a: 0.25 + 0.75 * crest })
                        },
                    ),
                );
            }
        }

        let model: SharedString = self.detail.split(" · ").next().unwrap_or("").to_string().into();
        // Solid black pill filling the window; same radius as the native backing.
        div()
            .relative()
            .size_full()
            .rounded_full()
            .bg(rgb(0x000000))
            .border_1()
            .border_color(rgba(0xffffff1f))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .text_color(rgb(0xfafafa))
            .font_family(".SystemUIFont")
            .child(wave)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div().size(px(6.)).rounded_full().bg(rgb(self.color)).with_animation(
                            "pulse",
                            Animation::new(Duration::from_millis(1200))
                                .repeat()
                                .with_easing(pulsating_between(0.35, 1.0)),
                            |d, t| d.opacity(t),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(model),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0xa1a1aa))
                            .child(self.title.clone()),
                    ),
            )
    }
}
