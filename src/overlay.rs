use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, prelude::*, pulsating_between, px, rgb, rgba,
    Animation, AnimationExt, Context, FontWeight, ObjectFit, RenderImage, SharedString, Window,
};

/// Level history; the waveform mirrors it outward from the centre bar.
pub const LEVEL_BARS: usize = 11;
const WAVE_BARS: usize = LEVEL_BARS * 2 - 1;
const WAVE_H: f32 = 40.;
const BAR_W: f32 = 5.;
const BAR_MIN: f32 = 6.;
const SHIMMER_MS: u64 = 1400;
const CAM_H: f32 = 168.;

pub struct OverlayView {
    pub title: SharedString,
    pub detail: SharedString,
    pub color: u32,
    /// Recent input levels (0..1), newest last — drawn as a live waveform
    /// while recording. Unused while transcribing.
    pub levels: [f32; LEVEL_BARS],
    pub live: bool,
    /// Lips mode: show the camera so you can check your framing.
    pub cam: bool,
    frame: Option<(u64, Arc<RenderImage>)>,
    face: bool,
}

impl OverlayView {
    pub fn new(title: impl Into<SharedString>, detail: impl Into<SharedString>, color: u32, live: bool, cam: bool) -> Self {
        OverlayView {
            title: title.into(),
            detail: detail.into(),
            color,
            levels: [0.0; LEVEL_BARS],
            live,
            cam,
            frame: None,
            face: false,
        }
    }

    /// Swap in the newest camera frame; the old texture is freed at once.
    /// The last frame stays (frozen) while decoding.
    fn refresh_frame(&mut self, window: &mut Window) {
        let next = crate::camera::LIVE.lock().unwrap().as_ref().and_then(|l| {
            self.face = l.face;
            let stale = self.frame.as_ref().map_or(true, |f| f.0 != l.preview_seq);
            (!l.preview.is_empty() && stale).then(|| (l.preview_seq, l.preview_h, l.preview.clone()))
        });
        let Some((seq, h, data)) = next else { return };
        let Some(img) = image::RgbaImage::from_raw(crate::camera::PREVIEW_W as u32, h as u32, data) else { return };
        let img = Arc::new(RenderImage::new(vec![image::Frame::new(img)]));
        if let Some((_, old)) = self.frame.replace((seq, img)) {
            let _ = window.drop_image(old);
        }
    }

    fn camera_view(&self) -> gpui::Div {
        let screen = div()
            .relative()
            .w_full()
            .h(px(CAM_H))
            .rounded(px(18.))
            .overflow_hidden()
            .bg(rgb(0x111113))
            .border_1()
            .border_color(rgba(if self.face && self.live { 0x22c55e66 } else { 0xffffff14 }));
        let Some((_, frame)) = &self.frame else {
            return screen.flex().items_center().justify_center().child(
                div().text_size(px(11.)).text_color(rgb(0x71717a)).child("Starting camera…"),
            );
        };
        let (dot, label) = match (self.live, self.face) {
            (false, _) => (0xa1a1aa, "Decoding"),
            (true, true) => (0x22c55e, "Lips locked"),
            (true, false) => (0xf59e0b, "Find your face"),
        };
        screen
            .child(gpui::img(frame.clone()).size_full().object_fit(ObjectFit::Cover).opacity(if self.live { 1.0 } else { 0.45 }))
            .child(
                div()
                    .absolute()
                    .left(px(8.))
                    .top(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded_full()
                    .bg(rgba(0x000000b3))
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().size(px(6.)).rounded_full().bg(rgb(dot)))
                    .child(label),
            )
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
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.cam {
            self.refresh_frame(window);
        }
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
        let pill = div()
            .relative()
            .size_full()
            .when(self.cam, |d| d.rounded(px(28.)).p(px(10.)).pb(px(12.)).justify_end())
            .when(!self.cam, |d| d.rounded_full().justify_center())
            .bg(rgb(0x000000))
            .border_1()
            .border_color(rgba(0xffffff1f))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .text_color(rgb(0xfafafa))
            .font_family(".SystemUIFont");
        let pill = if self.cam { pill.child(self.camera_view()).child(div().flex_1()) } else { pill };
        pill.child(wave)
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
