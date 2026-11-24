use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, prelude::*, pulsating_between, px, rgb, rgba,
    Animation, AnimationExt, Context, FontWeight, ObjectFit, RenderImage, SharedString, Window,
};

/// Level history; the waveform mirrors it outward from the centre bar.
pub const LEVEL_BARS: usize = 11;
const WAVE_BARS: usize = LEVEL_BARS * 2 - 1;
const WAVE_H: f32 = 52.;
const BAR_W: f32 = 6.;
const BAR_MIN: f32 = 6.;
const SHIMMER_MS: u64 = 1400;
const CAM_H: f32 = 196.;

/// The glass bubble sits inside a larger transparent window so its glow
/// can spill out underneath.
pub const BUBBLE_W: f32 = 420.;
pub const BUBBLE_H: f32 = 116.;
pub const BUBBLE_CAM_H: f32 = 330.;
pub const MARGIN_X: f32 = 30.;
pub const MARGIN_TOP: f32 = 14.;
pub const MARGIN_BOTTOM: f32 = 58.;
pub fn bubble_radius(cam: bool) -> f32 {
    if cam { 34. } else { BUBBLE_H / 2. }
}

/// Siri-style light: magenta, violet, blue, cyan, amber.
const SIRI: [u32; 5] = [0xff3d8b, 0xa74bff, 0x3d6bff, 0x2fd8ff, 0xff9a3d];

fn siri(t: f32) -> gpui::Rgba {
    let t = t.clamp(0., 1.) * (SIRI.len() - 1) as f32;
    let i = (t as usize).min(SIRI.len() - 2);
    let f = t - i as f32;
    let (a, b) = (rgb(SIRI[i]), rgb(SIRI[i + 1]));
    gpui::Rgba { r: a.r + (b.r - a.r) * f, g: a.g + (b.g - a.g) * f, b: a.b + (b.b - a.b) * f, a: 1. }
}

fn alpha(c: gpui::Rgba, a: f32) -> gpui::Hsla {
    gpui::Rgba { a, ..c }.into()
}

fn glow(color: gpui::Rgba, a: f32, blur: f32, spread: f32) -> gpui::BoxShadow {
    gpui::BoxShadow {
        color: alpha(color, a),
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    }
}

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
            .rounded(px(24.))
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

/// A waveform bar lit from inside: white core fading into its Siri hue.
fn bar(h: f32, a: f32, pos: f32) -> gpui::Div {
    let c = siri(pos);
    div()
        .w(px(BAR_W))
        .h(px(h))
        .rounded_full()
        .bg(gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(gpui::Hsla { h: 0., s: 0., l: 1., a }, 0.1),
            gpui::linear_color_stop(alpha(c, a), 1.),
        ))
        .shadow(vec![glow(c, 0.55 * a, 10., 0.)])
}

/// Soft blobs of Siri light drifting along the bubble's lower edge. Their
/// blurred shadows bleed past the glass, so the bubble glows from beneath.
fn light(width: f32, bottom: f32, energy: f32, busy: bool) -> gpui::AnyElement {
    let mut layer = div().absolute().left_0().top_0().size_full();
    let n = SIRI.len();
    let blob_w = width * 0.30;
    for (i, &c) in SIRI.iter().enumerate() {
        let c = rgb(c);
        let base = (i as f32 + 0.5) / n as f32;
        let period = if busy { 2600 } else { 5200 + 900 * i as u64 };
        let phase = i as f32 * 0.37;
        let swing = width * if busy { 0.22 } else { 0.12 };
        let lo = 0.12 * width;
        let hi = width - lo - blob_w;
        layer = layer.child(
            div()
                .absolute()
                .top(px(bottom - 24.))
                .w(px(blob_w))
                .h(px(22.))
                .rounded_full()
                .bg(alpha(c, 0.16 * energy))
                .shadow(vec![glow(c, energy, 28., 8.)])
                .with_animation(
                    gpui::ElementId::NamedInteger("siri-light".into(), i as u64),
                    Animation::new(Duration::from_millis(period)).repeat(),
                    move |d, t| {
                        let wave = ((t + phase) * std::f32::consts::TAU).sin();
                        let x = (base * width - blob_w / 2. + wave * swing).clamp(lo, hi);
                        d.left(px(x))
                    },
                ),
        );
    }
    layer.into_any_element()
}

impl Render for OverlayView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.cam && self.live {
            self.refresh_frame(window);
        }
        let mut wave = div().flex().items_center().justify_center().gap(px(5.)).h(px(WAVE_H));
        let now = (self.levels[LEVEL_BARS - 1] * 6.).sqrt().min(1.0);
        if self.live {
            for i in 0..WAVE_BARS {
                let pos = i as f32 / (WAVE_BARS - 1) as f32;
                let age = (i as isize - (WAVE_BARS / 2) as isize).unsigned_abs();
                let l = self.levels[LEVEL_BARS - 1 - age];
                // speech RMS sits around 0.02–0.2; sqrt spreads it visually
                let amp = (l * 6.).sqrt().min(1.0) * envelope(i);
                // a little per-bar texture so neighbours never match exactly
                let texture = 0.8 + 0.2 * ((i * 7 % 5) as f32 / 4.);
                wave = wave.child(bar(BAR_MIN + (WAVE_H - BAR_MIN) * amp * texture, 0.6 + 0.4 * amp, pos));
            }
        } else {
            // Transcribing: a bright crest sweeps across a resting wave.
            for i in 0..WAVE_BARS {
                let pos = i as f32 / (WAVE_BARS - 1) as f32;
                let rest = BAR_MIN + 6. * envelope(i);
                let c = siri(pos);
                wave = wave.child(
                    bar(rest, 0.3, pos).with_animation(
                        gpui::ElementId::NamedInteger("shimmer-bar".into(), i as u64),
                        Animation::new(Duration::from_millis(SHIMMER_MS)).repeat(),
                        move |d, t| {
                            let x = (t * 1.6 - 0.3 - pos).abs();
                            let crest = (1.0 - x / 0.22).max(0.0);
                            let crest = crest * crest * (3.0 - 2.0 * crest);
                            let a = 0.3 + 0.7 * crest;
                            d.h(px(rest + 24. * crest * envelope(i)))
                                .bg(gpui::linear_gradient(
                                    180.,
                                    gpui::linear_color_stop(gpui::Hsla { h: 0., s: 0., l: 1., a }, 0.1),
                                    gpui::linear_color_stop(alpha(c, a), 1.),
                                ))
                                .shadow(vec![glow(c, 0.7 * crest, 12., 0.)])
                        },
                    ),
                );
            }
        }

        let model: SharedString = self.detail.split(" · ").next().unwrap_or("").to_string().into();
        let bh = if self.cam { BUBBLE_CAM_H } else { BUBBLE_H };
        let radius = bubble_radius(self.cam);
        // Liquid Glass sits natively behind this view; darken its top so
        // text stays legible and let the lower half stay clear for the light.
        let glass = crate::glass_available();
        let shade = if glass {
            gpui::linear_gradient(
                180.,
                gpui::linear_color_stop(rgba(0x000000c8), 0.),
                gpui::linear_color_stop(rgba(0x140a2a59), 1.),
            )
        } else {
            gpui::linear_gradient(
                180.,
                gpui::linear_color_stop(rgba(0x050507f5), 0.),
                gpui::linear_color_stop(rgba(0x0b0b12f0), 1.),
            )
        };
        let energy = if self.live { 0.45 + 0.55 * now } else { 0.8 };
        let bubble = div()
            .absolute()
            .left(px(MARGIN_X))
            .top(px(MARGIN_TOP))
            .w(px(BUBBLE_W))
            .h(px(bh))
            .rounded(px(radius))
            .bg(shade)
            .border_1()
            .border_color(rgba(0xffffff2e))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .text_color(rgb(0xfafafa))
            .font_family(".SystemUIFont")
            .when(self.cam, |d| d.p(px(12.)).pb(px(16.)).justify_end())
            .when(!self.cam, |d| d.justify_center())
            // a hairline of specular light across the top edge
            .child(
                div()
                    .absolute()
                    .top(px(1.))
                    .left(px(radius))
                    .right(px(radius))
                    .h(px(1.))
                    .bg(gpui::linear_gradient(
                        90.,
                        gpui::linear_color_stop(rgba(0xffffff00), 0.),
                        gpui::linear_color_stop(rgba(0xffffff59), 0.5),
                    )),
            )
            .child(light(BUBBLE_W, bh, energy, !self.live));
        let bubble = if self.cam { bubble.child(self.camera_view()).child(div().flex_1()) } else { bubble };
        let bubble = bubble.child(wave).child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(
                    div().size(px(6.)).rounded_full().bg(rgb(self.color)).with_animation(
                        "pulse",
                        Animation::new(Duration::from_millis(1200))
                            .repeat()
                            .with_easing(pulsating_between(0.35, 1.0)),
                        |d, t| d.opacity(t),
                    ),
                )
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(model))
                .child(div().text_size(px(12.)).text_color(rgb(0xa1a1aa)).child(self.title.clone())),
        );
        // A wide halo under the bubble, like light pooling beneath it.
        let halo = div()
            .absolute()
            .left(px(MARGIN_X + BUBBLE_W * 0.18))
            .top(px(MARGIN_TOP + bh - 6.))
            .w(px(BUBBLE_W * 0.64))
            .h(px(8.))
            .rounded_full()
            .shadow(vec![
                glow(siri(0.05), 0.55 * energy, 26., 4.),
                glow(siri(0.55), 0.45 * energy, 40., 0.),
            ]);
        div().relative().size_full().child(halo).child(bubble)
    }
}
