use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    div, ease_out_quint, linear_color_stop, linear_gradient, prelude::*, pulsating_between, px,
    rgb, rgba, Animation, AnimationExt, AnyElement, Context, FocusHandle, FontWeight,
    KeyDownEvent, SharedString, Window,
};

use crate::{actions, audio, camera, config, hotkeys, lips, models, state, typer};

// Palette — shadcn zinc (dark) over a Liquid Glass window.
const BG: u32 = 0x09090b;
const SURFACE: u32 = 0x111113;
const SURFACE2: u32 = 0x18181b;
const SURFACE3: u32 = 0x27272a;
const HAIRLINE: u32 = 0x232326;
const FG: u32 = 0xfafafa;
const MUTED: u32 = 0xa1a1aa;
const SUBTLE: u32 = 0x71717a;
const ACCENT: u32 = 0xfafafa;
const ACCENT_HI: u32 = 0xd4d4d8;
const ON_ACCENT: u32 = 0x18181b;
const GREEN: u32 = 0x22c55e;
const AMBER: u32 = 0xf59e0b;
const DANGER: u32 = 0xef4444;
// Subtle white layers over a solid black window.
const GLASS_FILL: u32 = 0xffffff08;
const GLASS_EDGE: u32 = 0xffffff14;
const CONTENT_TINT: u32 = 0x000000ff;
const SIDEBAR_BG: u32 = 0x0a0a0a;

const SIDEBAR_W: f32 = 212.;
const TITLEBAR_H: f32 = 52.;
pub const SIDEBAR_WIDTH: f32 = SIDEBAR_W;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Dictate,
    Models,
    Settings,
}

pub struct ThinSttView {
    page: Page,
    status: String,
    status_error: bool,
    capturing_for: Option<String>,
    // Largest modifier set held so far during capture — committed when
    // everything is released, so Ctrl+Option assigns as one combo.
    capture_mods: Option<(u8, String)>,
    // Captured combo awaiting Save, and whether it came from a keyed press
    // (so releasing its modifiers doesn't overwrite it).
    capture_pending: Option<String>,
    capture_keyed: bool,
    progress: HashMap<String, (u64, u64)>,
    mic_open: bool,
    confirm_delete: bool,
    focus: FocusHandle,
    preview_img: Option<(u64, std::sync::Arc<gpui::RenderImage>)>,
}

impl Drop for ThinSttView {
    fn drop(&mut self) {
        let _ = camera::set_preview(false);
    }
}

impl ThinSttView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        ThinSttView {
            page: Page::Dictate,
            status: "Ready".into(),
            status_error: false,
            capturing_for: None,
            capture_mods: None,
            capture_pending: None,
            capture_keyed: false,
            progress: HashMap::new(),
            mic_open: false,
            confirm_delete: false,
            focus: cx.focus_handle(),
            preview_img: None,
        }
    }

    /// Swap in the newest camera frame; the old texture is freed at once.
    fn refresh_preview(&mut self, window: &mut Window) {
        if self.page != Page::Dictate && camera::previewing() {
            let _ = camera::set_preview(false);
        }
        let next = camera::LIVE.lock().unwrap().as_ref().and_then(|l| {
            let stale = self.preview_img.as_ref().map_or(true, |p| p.0 != l.preview_seq);
            (l.previewing && !l.preview.is_empty() && stale).then(|| (l.preview_seq, l.preview_h, l.preview.clone()))
        });
        let replace = match next {
            Some((seq, h, data)) => image::RgbaImage::from_raw(camera::PREVIEW_W as u32, h as u32, data)
                .map(|img| (seq, std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(img)])))),
            None if !camera::previewing() => None,
            None => return,
        };
        if let Some((_, old)) = std::mem::replace(&mut self.preview_img, replace) {
            let _ = window.drop_image(old);
        }
    }

    pub fn set_status(&mut self, text: String, error: bool) {
        self.status = text;
        self.status_error = error;
    }

    pub fn set_progress(&mut self, id: String, done: u64, total: u64) {
        self.progress.insert(id, (done, total));
    }

    fn is_downloading(&self, id: &str) -> bool {
        state::state().downloads.lock().unwrap().contains_key(id)
    }

    fn start_capture(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_capture();
        self.capturing_for = Some(id.to_string());
        window.focus(&self.focus);
        cx.notify();
    }
}

// ---------------------------------------------------------------- helpers

fn mb(bytes: u64) -> String {
    let m = bytes as f64 / (1024.0 * 1024.0);
    if m >= 1000.0 {
        format!("{:.1} GB", m / 1024.0)
    } else {
        format!("{:.0} MB", m)
    }
}

/// The app icon (small copy), shared so it's decoded once.
fn app_icon() -> std::sync::Arc<gpui::Image> {
    static ICON: std::sync::OnceLock<std::sync::Arc<gpui::Image>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/icon-128.png").to_vec(),
        ))
    })
    .clone()
}

fn mb_f(m: f64) -> String {
    mb((m * 1024.0 * 1024.0) as u64)
}

fn is_speech(m: &models::ModelInfo) -> bool {
    m.engine != "llm"
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "paste" => "Paste",
        "clipboard" => "Clipboard",
        _ => "Type",
    }
}

fn mode_desc(mode: &str) -> &'static str {
    match mode {
        "paste" => "Pastes with ⌘V, then restores what you had copied. Fastest for long text.",
        "clipboard" => "Leaves the transcript on the clipboard — paste it yourself.",
        _ => "Types the transcript as keystrokes. Your clipboard is never touched.",
    }
}

fn dot(color: u32, size: f32) -> gpui::Div {
    div().size(px(size)).rounded_full().bg(rgb(color)).flex_none()
}

fn icon(glyph: &'static str, color: u32) -> gpui::Div {
    div()
        .w(px(18.))
        .flex()
        .justify_center()
        .text_size(px(14.))
        .text_color(rgb(color))
        .child(glyph)
}

fn tag(label: impl Into<SharedString>, fg: u32, bg: u32) -> gpui::Div {
    div()
        .px(px(7.))
        .py(px(1.))
        .rounded(px(5.))
        .bg(rgba(bg))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(fg))
        .child(label.into())
}

fn button_base(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(30.))
        .px(px(12.))
        .rounded(px(8.))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(label.into())
}

fn btn_primary(label: impl Into<SharedString>) -> gpui::Div {
    button_base(label)
        .bg(rgb(ACCENT))
        .text_color(rgb(ON_ACCENT))
        .shadow_sm()
        .hover(|s| s.bg(rgb(ACCENT_HI)))
}

fn btn_secondary(label: impl Into<SharedString>) -> gpui::Div {
    button_base(label)
        .bg(rgba(GLASS_FILL))
        .border_1()
        .border_color(rgb(SURFACE3))
        .text_color(rgb(FG))
        .shadow_sm()
        .hover(|s| s.bg(rgb(SURFACE2)))
}

fn btn_ghost(label: impl Into<SharedString>) -> gpui::Div {
    button_base(label)
        .text_color(rgb(MUTED))
        .hover(|s| s.bg(rgba(0xffffff0d)).text_color(rgb(FG)))
}

fn btn_danger(label: impl Into<SharedString>) -> gpui::Div {
    button_base(label)
        .border_1()
        .border_color(rgba(0xef444466))
        .text_color(rgb(DANGER))
        .hover(|s| s.bg(rgba(0xef444422)))
}

fn keycap(label: String, big: bool) -> gpui::Div {
    let h = if big { 46. } else { 24. };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(h))
        .min_w(px(h))
        .px(px(if big { 12. } else { 7. }))
        .rounded(px(if big { 10. } else { 6. }))
        .bg(rgb(SURFACE2))
        .border_1()
        .border_b(px(if big { 3. } else { 2. }))
        .border_color(rgb(0x3f3f46))
        .shadow_sm()
        .text_size(px(if big { 21. } else { 13. }))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(FG))
        .child(label)
}

/// "ctrl+alt+9" → one keycap per key.
fn keycaps(spec: &str, big: bool) -> gpui::Div {
    let mut row = div().flex().items_center().gap(px(if big { 8. } else { 4. }));
    for part in spec.split('+').filter(|p| !p.is_empty()) {
        row = row.child(keycap(hotkeys::display(part), big));
    }
    row
}

fn toggle(on: bool) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .w(px(38.))
        .h(px(22.))
        .p(px(2.))
        .rounded_full()
        .cursor_pointer()
        .bg(rgb(if on { ACCENT } else { SURFACE3 }))
        .when(on, |d| d.justify_end())
        .child(div().size(px(18.)).rounded_full().bg(rgb(if on { BG } else { FG })).shadow_sm())
}

fn progress_bar(frac: f32, w: f32) -> gpui::Div {
    div()
        .w(px(w))
        .h(px(5.))
        .rounded_full()
        .bg(rgb(SURFACE3))
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(rgb(ACCENT))
                .w(gpui::relative(frac.clamp(0.0, 1.0))),
        )
}

fn card() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(12.))
        .bg(rgba(GLASS_FILL))
        .border_1()
        .border_color(rgba(GLASS_EDGE))
        .shadow_sm()
        .overflow_hidden()
}

/// A titled group of rows separated by hairlines (macOS settings style).
fn group(title: &str, caption: Option<&str>, rows: Vec<AnyElement>) -> gpui::Div {
    let mut body = card();
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        body = body.child(
            div()
                .when(i + 1 < n, |d| d.border_b_1().border_color(rgb(HAIRLINE)))
                .child(r),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .px(px(4.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(MUTED))
                        .child(title.to_string()),
                )
                .when_some(caption, |d, c| {
                    d.child(div().text_size(px(12.)).text_color(rgb(SUBTLE)).child(c.to_string()))
                }),
        )
        .child(body)
}

fn setting_row(
    title: impl IntoElement,
    subtitle: Option<SharedString>,
    control: impl IntoElement,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .px(px(16.))
        .py(px(12.))
        .min_h(px(52.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().text_size(px(13.)).text_color(rgb(FG)).child(title))
                .when_some(subtitle, |d, s| {
                    d.child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(s))
                }),
        )
        .child(control)
}

fn page_header(title: &str, subtitle: &str) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(26.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(FG))
                .child(title.to_string()),
        )
        .child(div().text_size(px(13.)).text_color(rgb(MUTED)).child(subtitle.to_string()))
}

// ---------------------------------------------------------------- pages

impl ThinSttView {
    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let installed = models::MODELS.iter().filter(|m| models::is_downloaded(m)).count();
        let mut nav = div().flex().flex_col().gap(px(2.)).px(px(10.));
        for (page, glyph, label, badge) in [
            (Page::Dictate, "◉", "Dictate", None),
            (Page::Models, "▦", "Models", Some(installed)),
            (Page::Settings, "⚙", "Settings", None),
        ] {
            let active = self.page == page;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("nav-{}", label)))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(active, |d| d.bg(rgba(0xffffff17)).text_color(rgb(FG)).shadow_sm())
                    .when(!active, |d| {
                        d.text_color(rgb(MUTED)).hover(|s| s.bg(rgba(0xffffff0a)).text_color(rgb(FG)))
                    })
                    .child(icon(glyph, if active { ACCENT_HI } else { SUBTLE }))
                    .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(label))
                    .when_some(badge.filter(|n| *n > 0), |d, n| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(SUBTLE))
                                .child(n.to_string()),
                        )
                    })
                    .on_click(cx.listener(move |v, _e, _w, cx| {
                        v.page = page;
                        v.mic_open = false;
                        v.confirm_delete = false;
                        cx.notify();
                    })),
            );
        }

        let status_color = if self.status_error { DANGER } else { GREEN };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_W))
            .h_full()
            .bg(rgb(SIDEBAR_BG))
            .border_r_1()
            .border_color(rgba(0xffffff12))
            .child(div().h(px(TITLEBAR_H)).flex_none())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(20.))
                    .pb(px(18.))
                    .child(
                        gpui::img(app_icon())
                            .size(px(30.))
                            .rounded(px(7.))
                            .shadow_md(),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(FG))
                                    .child("Thin STT"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(SUBTLE))
                                    .child("Local dictation"),
                            ),
                    ),
            )
            .child(nav)
            .child(div().flex_1())
            .child(
                div().px(px(10.)).pb(px(8.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .p(px(10.))
                        .rounded(px(10.))
                        .bg(rgba(0xffffff0a))
                        .border_1()
                        .border_color(rgba(GLASS_EDGE))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(dot(status_color, 6.))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(SUBTLE))
                                        .child("ACTIVITY"),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .line_clamp(3)
                                .text_color(rgb(if self.status_error { DANGER } else { FG }))
                                .child(self.status.clone()),
                        ),
                ),
            )
            .child(
                div()
                    .px(px(20.))
                    .pb(px(14.))
                    .text_size(px(11.))
                    .text_color(rgb(SUBTLE))
                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
            )
    }

    fn render_dictate(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let s = state::state();
        let cfg = s.cfg.lock().unwrap().clone();
        let speech: Vec<&'static models::ModelInfo> =
            models::MODELS.iter().filter(|m| is_speech(m)).collect();
        let downloaded: Vec<&'static models::ModelInfo> =
            speech.iter().copied().filter(|m| models::is_downloaded(m)).collect();
        let bound: Vec<(&'static models::ModelInfo, String)> = downloaded
            .iter()
            .filter_map(|m| cfg.hotkeys.get(m.id).map(|h| (*m, h.clone())))
            .collect();
        let trusted = typer::accessibility_trusted();
        let mode = cfg.paste_mode.clone().unwrap_or_else(|| "type".into());
        let polish_ready = models::polish_model_path().is_some();
        let polish_on = cfg.polish && polish_ready;

        let lips_mode = cfg.lips();
        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(page_header(
                "Dictate",
                if lips_mode {
                    "Hold your shortcut, mouth the words silently, release — the camera reads your lips."
                } else {
                    "Hold your shortcut in any app, speak, and release — the text lands at your cursor."
                },
            ))
            .child(source_switch(lips_mode, cx));
        if lips_mode {
            return page.children(self.lips_panel(bound.first().map(|b| b.1.clone()), downloaded.first().map(|m| m.id), cx));
        }

        // ---- hero
        let hero = div()
            .flex()
            .flex_col()
            .rounded(px(16.))
            .border_1()
            .border_color(rgba(GLASS_EDGE))
            .shadow_lg()
            .bg(linear_gradient(
                160.,
                linear_color_stop(rgba(0xffffff12), 0.),
                linear_color_stop(rgba(0xffffff03), 1.),
            ))
            .p(px(28.));
        let hero = if let Some((m, spec)) = bound.first() {
            hero.items_center()
                .gap(px(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(dot(GREEN, 7.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(GREEN))
                                .child(if trusted { "READY" } else { "READY — CLIPBOARD ONLY" }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(div().text_size(px(15.)).text_color(rgb(MUTED)).child("Hold"))
                        .child(keycaps(spec, true))
                        .child(div().text_size(px(15.)).text_color(rgb(MUTED)).child("to talk")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .flex_wrap()
                        .justify_center()
                        .child(tag(m.label, FG, 0xffffff14))
                        .child(tag(format!("{} at cursor", mode_label(&mode)), FG, 0xffffff14))
                        .child(tag(
                            if polish_on { "Polish on" } else { "Polish off" },
                            if polish_on { ACCENT_HI } else { MUTED },
                            if polish_on { 0xffffff26 } else { 0xffffff0d },
                        ))
                        .child(tag(
                            cfg.mic_device.clone().unwrap_or_else(|| "Default mic".into()),
                            MUTED,
                            0xffffff0d,
                        )),
                )
        } else {
            hero.flex_row()
                .items_center()
                .justify_between()
                .gap(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(FG))
                                .child("Three steps to your first dictation"),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(rgb(MUTED))
                                .child("Everything runs on this Mac — no account, no cloud."),
                        ),
                )
                .child(div().opacity(0.35).child(keycaps("ctrl+alt", true)))
        };
        page = page.child(hero);

        // ---- setup checklist (until every step is done)
        let step1 = !downloaded.is_empty();
        let step2 = !bound.is_empty();
        if !(step1 && step2 && trusted) {
            let tiny = models::by_id("whisper-tiny");
            let tiny_dl = tiny.map_or(false, |t| self.is_downloading(t.id));
            let step1_action: AnyElement = if step1 {
                div().into_any_element()
            } else if tiny_dl {
                let (done, total) = self.progress.get("whisper-tiny").copied().unwrap_or((0, 0));
                let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(progress_bar(frac, 120.))
                    .child(
                        div()
                            .w(px(34.))
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(format!("{:.0}%", frac * 100.0)),
                    )
                    .into_any_element()
            } else {
                let size = cfg
                    .remote_sizes
                    .get("whisper-tiny")
                    .map(|b| mb(*b))
                    .unwrap_or_else(|| "75 MB".into());
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        btn_ghost("Browse")
                            .id("setup-browse")
                            .on_click(cx.listener(|v, _e, _w, cx| {
                                v.page = Page::Models;
                                cx.notify();
                            })),
                    )
                    .child(
                        btn_primary(format!("Get Whisper Tiny · {}", size))
                            .id("setup-dl")
                            .on_click(cx.listener(|_v, _e, _w, cx| {
                                actions::start_download("whisper-tiny");
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            };
            let first_dl = downloaded.first().map(|m| m.id);
            let step2_action: AnyElement = match (step2, first_dl) {
                (false, Some(id)) => btn_primary("Choose shortcut")
                    .id("setup-hk")
                    .on_click(cx.listener(move |v, _e, w, cx| v.start_capture(id, w, cx)))
                    .into_any_element(),
                _ => div().into_any_element(),
            };
            let step3_action: AnyElement = if trusted {
                div().into_any_element()
            } else {
                btn_secondary("Grant access")
                    .id("setup-acc")
                    .on_click(cx.listener(|_v, _e, _w, cx| {
                        typer::request_accessibility();
                        cx.notify();
                    }))
                    .into_any_element()
            };
            let steps = vec![
                step_row(1, step1, true, "Download a speech model",
                    "Whisper Tiny is the quickest start; Parakeet is fastest for English.",
                    step1_action),
                step_row(2, step2, step1, "Choose a hold-to-talk shortcut",
                    "Modifier-only combos like ⌃⌥ work — nothing else gets hijacked.",
                    step2_action),
                step_row(3, trusted, true, "Allow typing into other apps",
                    "macOS Accessibility permission, so text can be typed at your cursor.",
                    step3_action),
            ];
            page = page.child(group("GET STARTED", None, steps));
        }

        // ---- shortcuts per installed model
        if !downloaded.is_empty() {
            let mut rows = Vec::new();
            for m in &downloaded {
                rows.push(self.shortcut_row(m, cfg.hotkeys.get(m.id).cloned(), cx).into_any_element());
            }
            page = page.child(group(
                "SHORTCUTS",
                Some("each installed model can have its own"),
                rows,
            ));
        }

        // ---- quick settings
        let quick = vec![
            setting_row(
                "Insert mode",
                Some(mode_desc(&mode).into()),
                mode_segments(&mode, cx),
            )
            .into_any_element(),
            setting_row(
                "Polish transcripts",
                Some(
                    if polish_ready {
                        "A local Qwen3 pass fixes punctuation, casing and filler words."
                    } else {
                        "Download the Qwen3 polish model in Models to enable."
                    }
                    .into(),
                ),
                polish_toggle(polish_on, polish_ready, cx),
            )
            .into_any_element(),
        ];
        page = page.child(group("OUTPUT", None, quick));
        page
    }

    fn shortcut_row(
        &mut self,
        m: &'static models::ModelInfo,
        hotkey: Option<String>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let id = m.id;
        let control = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(match &hotkey {
                Some(spec) => div()
                    .id(SharedString::from(format!("hkc-{}", id)))
                    .cursor_pointer()
                    .rounded(px(8.))
                    .p(px(3.))
                    .hover(|s| s.bg(rgb(SURFACE2)))
                    .child(keycaps(spec, false))
                    .on_click(cx.listener(move |v, _e, w, cx| v.start_capture(id, w, cx)))
                    .into_any_element(),
                None => btn_secondary("Set shortcut")
                    .id(SharedString::from(format!("hk-{}", id)))
                    .on_click(cx.listener(move |v, _e, w, cx| v.start_capture(id, w, cx)))
                    .into_any_element(),
            })
            .when(hotkey.is_some(), |d| {
                d.child(
                    btn_ghost("✕")
                        .px(px(8.))
                        .id(SharedString::from(format!("hkx-{}", id)))
                        .on_click(cx.listener(move |_v, _e, _w, cx| {
                            actions::clear_hotkey(id);
                            cx.notify();
                        })),
                )
            });
        setting_row(m.label, Some(m.desc.into()), control)
    }

    fn render_models(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let installed: Vec<_> = models::MODELS.iter().filter(|m| models::is_downloaded(m)).collect();
        let on_disk: f64 = models::MODELS.iter().map(|m| models::disk_size_mb(m.id)).sum();

        let mut page = div().flex().flex_col().gap(px(24.)).child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .child(page_header(
                    "Models",
                    "Everything runs on this Mac. A model loads only while you dictate.",
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(SUBTLE))
                        .child(format!("{} installed · {} on disk", installed.len(), mb_f(on_disk))),
                ),
        );

        for (title, caption, filter) in [
            ("WHISPER", "OpenAI · whisper.cpp on Metal", "whisper"),
            ("PARAKEET", "NVIDIA · ONNX Runtime", "parakeet"),
            ("LIPS", "silent lip reading from the camera · ONNX Runtime · research-only weights", "lips"),
            ("POLISH", "optional cleanup pass · llama.cpp", "llm"),
        ] {
            let mut rows = Vec::new();
            for m in models::MODELS.iter().filter(|m| m.engine == filter) {
                rows.push(self.model_row(m, cx).into_any_element());
            }
            page = page.child(group(title, Some(caption), rows));
        }
        page
    }

    fn model_row(&mut self, m: &'static models::ModelInfo, cx: &mut Context<Self>) -> gpui::Div {
        let s = state::state();
        let cfg = s.cfg.lock().unwrap().clone();
        let downloaded = models::is_downloaded(m);
        let downloading = self.is_downloading(m.id);
        let hotkey = cfg.hotkeys.get(m.id).cloned();
        let format = m.badge.split('•').next().unwrap_or("").trim().to_string();
        let id = m.id;

        let size_lbl = if downloaded {
            mb_f(models::disk_size_mb(m.id))
        } else {
            cfg.remote_sizes
                .get(m.id)
                .map(|b| mb(*b))
                .unwrap_or_else(|| m.badge.split('•').nth(1).unwrap_or("").trim().to_string())
        };

        let info = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(3.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(dot(
                        if downloaded {
                            GREEN
                        } else if downloading {
                            ACCENT
                        } else {
                            SURFACE3
                        },
                        7.,
                    ))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(FG))
                            .child(m.label),
                    )
                    .child(tag(format, MUTED, 0xffffff0d)),
            )
            .child(
                div()
                    .pl(px(15.))
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(format!("{} · {}", m.desc, size_lbl)),
            );

        let control: AnyElement = if downloading {
            let (done, total) = self.progress.get(m.id).copied().unwrap_or((0, 0));
            let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(px(4.))
                        .child(progress_bar(frac, 140.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child(if total > 0 {
                                    format!("{} of {} · {:.0}%", mb(done), mb(total), frac * 100.0)
                                } else if done == 0 {
                                    "Starting…".to_string()
                                } else {
                                    format!("{} downloaded", mb(done))
                                }),
                        ),
                )
                .child(
                    btn_ghost("Cancel")
                        .id(SharedString::from(format!("dlc-{}", id)))
                        .on_click(cx.listener(move |_v, _e, _w, cx| {
                            state::cancel_download(id);
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else if downloaded {
            let mut c = div().flex().items_center().gap(px(6.));
            if is_speech(m) {
                c = c.child(match &hotkey {
                    Some(spec) => div()
                        .id(SharedString::from(format!("mhk-{}", id)))
                        .cursor_pointer()
                        .rounded(px(8.))
                        .p(px(3.))
                        .hover(|s| s.bg(rgb(SURFACE2)))
                        .child(keycaps(spec, false))
                        .on_click(cx.listener(move |v, _e, w, cx| v.start_capture(id, w, cx)))
                        .into_any_element(),
                    None => btn_secondary("Set shortcut")
                        .id(SharedString::from(format!("mhks-{}", id)))
                        .on_click(cx.listener(move |v, _e, w, cx| v.start_capture(id, w, cx)))
                        .into_any_element(),
                });
            } else {
                let on = cfg.polish;
                c = c.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .mr(px(4.))
                        .child(if on { "On" } else { "Off" }),
                )
                .child(toggle(on).id("polish-row-toggle").on_click(cx.listener(
                    |_v, _e, _w, cx| {
                        actions::toggle_polish();
                        cx.notify();
                    },
                )));
            }
            c.child(
                btn_ghost("Remove")
                    .id(SharedString::from(format!("del-{}", id)))
                    .on_click(cx.listener(move |_v, _e, _w, cx| {
                        actions::delete_model(id);
                        cx.notify();
                    })),
            )
            .into_any_element()
        } else {
            btn_secondary("↓  Download")
                .id(SharedString::from(format!("dl-{}", id)))
                .on_click(cx.listener(move |_v, _e, _w, cx| {
                    actions::start_download(id);
                    cx.notify();
                }))
                .into_any_element()
        };

        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(16.))
            .py(px(12.))
            .min_h(px(60.))
            .child(info)
            .child(control)
    }

    fn render_settings(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let s = state::state();
        let cfg = s.cfg.lock().unwrap().clone();
        let mode = cfg.paste_mode.clone().unwrap_or_else(|| "type".into());
        let trusted = typer::accessibility_trusted();
        let polish_ready = models::polish_model_path().is_some();
        let polish_on = cfg.polish && polish_ready;

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(page_header("Settings", "Output, input, permissions and storage."));

        // ---- output
        page = page.child(group(
            "OUTPUT",
            None,
            vec![
                setting_row("Insert mode", Some(mode_desc(&mode).into()), mode_segments(&mode, cx))
                    .into_any_element(),
                setting_row(
                    "Polish transcripts",
                    Some(
                        if polish_ready {
                            "Qwen3 cleans each transcript; it loads only while polishing."
                        } else {
                            "Needs the Qwen3 polish model — download it in Models."
                        }
                        .into(),
                    ),
                    polish_toggle(polish_on, polish_ready, cx),
                )
                .into_any_element(),
            ],
        ));

        // ---- input
        let mic_label = cfg.mic_device.clone().unwrap_or_else(|| "System default".into());
        let cam_perm = camera::permission();
        let mut input_rows = vec![
            setting_row(
                "Input source",
                Some(
                    if cfg.lips() {
                        "Lips: the camera reads your mouth while the shortcut is held. No audio, no polish."
                    } else {
                        "Voice: the microphone records while the shortcut is held."
                    }
                    .into(),
                ),
                source_segments(cfg.lips(), cx),
            )
            .into_any_element(),
            setting_row(
                "Camera",
                Some("Used only in Lips mode while the shortcut is held. Frames stay in memory and are never saved.".into()),
                camera_access(cam_perm, cx),
            )
            .into_any_element(),
        ];
        input_rows.push(setting_row(
            "Microphone",
            Some("Used only while your shortcut is held. Audio never touches disk.".into()),
            btn_secondary(format!("{}  ▾", mic_label))
                .id("mic-toggle")
                .on_click(cx.listener(|v, _e, _w, cx| {
                    v.mic_open = !v.mic_open;
                    cx.notify();
                })),
        )
        .into_any_element());
        if self.mic_open {
            let mut list = div().flex().flex_col().gap(px(2.)).px(px(10.)).py(px(8.));
            let mut names = vec!["System default".to_string()];
            names.extend(audio::input_device_names());
            for name in names {
                let chosen = if name == "System default" { None } else { Some(name.clone()) };
                let selected = chosen == cfg.mic_device;
                list = list.child(
                    div()
                        .id(SharedString::from(format!("mic-{}", name)))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(30.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .cursor_pointer()
                        .text_color(rgb(if selected { FG } else { MUTED }))
                        .hover(|s| s.bg(rgb(SURFACE2)).text_color(rgb(FG)))
                        .child(
                            div()
                                .w(px(14.))
                                .text_color(rgb(ACCENT_HI))
                                .child(if selected { "✓" } else { "" }),
                        )
                        .child(name)
                        .on_click(cx.listener(move |v, _e, _w, cx| {
                            actions::set_mic(chosen.clone());
                            v.mic_open = false;
                            cx.notify();
                        })),
                );
            }
            input_rows.push(list.into_any_element());
        }
        page = page.child(group("INPUT", None, input_rows));

        // ---- permissions
        let acc: AnyElement = if trusted {
            tag("✓ Granted", GREEN, 0x22c55e1f).into_any_element()
        } else {
            btn_primary("Grant access")
                .id("acc")
                .on_click(cx.listener(|_v, _e, _w, cx| {
                    typer::request_accessibility();
                    cx.notify();
                }))
                .into_any_element()
        };
        page = page.child(group(
            "PERMISSIONS",
            None,
            vec![
                setting_row(
                    "Accessibility",
                    Some(
                        if trusted {
                            "Thin STT can type into the focused app."
                        } else {
                            "Required to type into other apps and to watch modifier-only shortcuts."
                        }
                        .into(),
                    ),
                    acc,
                )
                .into_any_element(),
                setting_row(
                    "Microphone",
                    Some("macOS asks the first time you dictate.".into()),
                    div(),
                )
                .into_any_element(),
            ],
        ));

        // ---- storage
        let cache_mb: f64 = models::MODELS.iter().map(|m| models::disk_size_mb(m.id)).sum();
        page = page.child(group(
            "STORAGE",
            None,
            vec![setting_row(
                "Model cache",
                Some(
                    format!(
                        "{} used. Clear cache removes partial downloads — installed models stay.",
                        mb_f(cache_mb)
                    )
                    .into(),
                ),
                div()
                    .flex()
                    .gap(px(6.))
                    .child(btn_ghost("Show in Finder").id("reveal").on_click(cx.listener(
                        |_v, _e, _w, _cx| {
                            let dir = config::models_dir();
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::process::Command::new("open").arg(dir).spawn();
                        },
                    )))
                    .child(btn_secondary("Clear cache").id("clear-cache").on_click(cx.listener(
                        |_v, _e, _w, cx| {
                            actions::clear_model_cache();
                            cx.notify();
                        },
                    ))),
            )
            .into_any_element()],
        ));

        // ---- updates
        let update_msg = s.update_available.lock().unwrap().clone();
        let mut about = vec![setting_row(
            format!("Thin STT {}", env!("CARGO_PKG_VERSION")),
            Some("Updates install in place and relaunch the app.".into()),
            btn_secondary("Check for updates")
                .id("check-update")
                .on_click(cx.listener(|_v, _e, _w, cx| {
                    actions::check_for_updates();
                    cx.notify();
                })),
        )
        .into_any_element()];
        if let Some(msg) = update_msg {
            about.push(
                setting_row(
                    div().text_color(rgb(ACCENT_HI)).child(msg),
                    None,
                    btn_primary("Update now")
                        .id("update-now")
                        .on_click(cx.listener(|_v, _e, _w, cx| {
                            actions::apply_update();
                            cx.notify();
                        })),
                )
                .into_any_element(),
            );
        }
        page = page.child(group("ABOUT", None, about));

        // ---- danger zone
        let danger: AnyElement = if self.confirm_delete {
            div()
                .flex()
                .gap(px(6.))
                .child(btn_ghost("Cancel").id("uninstall-no").on_click(cx.listener(
                    |v, _e, _w, cx| {
                        v.confirm_delete = false;
                        cx.notify();
                    },
                )))
                .child(btn_secondary("Keep models").id("uninstall-keep").on_click(cx.listener(
                    |_v, _e, _w, cx| {
                        actions::uninstall(true);
                        cx.quit();
                    },
                )))
                .child(
                    button_base("Delete everything")
                        .bg(rgb(DANGER))
                        .text_color(rgb(0xffffff))
                        .id("uninstall-yes")
                        .on_click(cx.listener(|_v, _e, _w, cx| {
                            actions::uninstall(false);
                            cx.quit();
                        })),
                )
                .into_any_element()
        } else {
            btn_danger("Delete app…")
                .id("uninstall")
                .on_click(cx.listener(|v, _e, _w, cx| {
                    v.confirm_delete = true;
                    cx.notify();
                }))
                .into_any_element()
        };
        page = page.child(group(
            "DANGER ZONE",
            None,
            vec![setting_row(
                "Delete Thin STT",
                Some(
                    if self.confirm_delete {
                        "Keep models saves them for a reinstall. The app and settings go either way."
                    } else {
                        "Removes the app and its settings, with or without your downloaded models."
                    }
                    .into(),
                ),
                danger,
            )
            .into_any_element()],
        ));

        page
    }

    fn cancel_capture(&mut self) {
        self.capturing_for = None;
        self.capture_mods = None;
        self.capture_pending = None;
        self.capture_keyed = false;
    }

    fn commit_capture(&mut self) {
        let (Some(id), Some(spec)) = (self.capturing_for.clone(), self.capture_pending.clone())
        else {
            return;
        };
        if !assess(&spec, &id).2 {
            return;
        }
        let canon = hotkeys::canonical(&spec);
        let taken: Vec<String> = {
            let cfg = state::state().cfg.lock().unwrap();
            cfg.hotkeys
                .iter()
                .filter(|(k, v)| **k != id && hotkeys::canonical(v) == canon)
                .map(|(k, _)| k.clone())
                .collect()
        };
        for other in taken {
            actions::clear_hotkey(&other);
        }
        actions::set_hotkey(&id, &spec);
        let label = models::by_id(&id).map(|m| m.label).unwrap_or("model");
        self.set_status(format!("{} · {}", label, hotkeys::display(&spec)), false);
        self.cancel_capture();
    }

    fn render_capture(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let id = self.capturing_for.clone().unwrap_or_default();
        let label = models::by_id(&id).map(|m| m.label).unwrap_or("model");
        let current = state::state().cfg.lock().unwrap().hotkeys.get(&id).cloned();
        let live = self.capture_mods.as_ref().map(|(_, spec)| spec.clone());
        let pending = self.capture_pending.clone();
        let verdict = pending.as_deref().map(|p| assess(p, &id));
        let can_save = verdict.as_ref().map_or(false, |v| v.2);

        // Stage: what the user is pressing right now, or what they captured.
        let (stage_border, keys, status_color, status_text, pulse): (u32, AnyElement, u32, String, bool) =
            match (&live, &pending, &verdict) {
                (Some(spec), _, _) => (
                    ACCENT,
                    stage_keys(spec, ACCENT).into_any_element(),
                    ACCENT_HI,
                    "Keep holding — release to set".into(),
                    true,
                ),
                (None, Some(spec), Some((color, msg, _))) => (
                    *color,
                    stage_keys(spec, *color).into_any_element(),
                    *color,
                    msg.clone(),
                    false,
                ),
                _ => (
                    HAIRLINE,
                    stage_placeholder().into_any_element(),
                    MUTED,
                    "Listening — press your keys".into(),
                    true,
                ),
            };

        let status_dot = dot(status_color, 7.);
        let status_dot: AnyElement = if pulse {
            status_dot
                .with_animation(
                    "capture-pulse",
                    Animation::new(Duration::from_millis(1100))
                        .repeat()
                        .with_easing(pulsating_between(0.3, 1.0)),
                    |d, t| d.opacity(t),
                )
                .into_any_element()
        } else {
            status_dot.into_any_element()
        };

        let stage = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .w_full()
            .h(px(150.))
            .rounded(px(14.))
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(stage_border))
            .when(live.is_none() && pending.is_none(), |d| d.border_dashed())
            .child(keys)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(16.))
                    .child(status_dot)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(status_color))
                            .child(status_text),
                    ),
            );

        let mut picks = div().flex().flex_wrap().gap(px(6.));
        for spec in ["ctrl+alt", "alt+shift", "ctrl+shift", "cmd+alt"] {
            let selected = pending.as_deref() == Some(spec);
            picks = picks.child(
                div()
                    .id(SharedString::from(format!("pick-{}", spec)))
                    .flex()
                    .items_center()
                    .p(px(4.))
                    .rounded(px(9.))
                    .cursor_pointer()
                    .bg(rgb(if selected { SURFACE3 } else { SURFACE2 }))
                    .border_1()
                    .border_color(rgb(if selected { ACCENT } else { SURFACE3 }))
                    .hover(|s| s.border_color(rgb(ACCENT)))
                    .child(keycaps(spec, false))
                    .on_click(cx.listener(move |v, _e, _w, cx| {
                        v.capture_pending = Some(spec.to_string());
                        v.capture_mods = None;
                        cx.notify();
                    })),
            );
        }

        let header = div()
            .flex()
            .items_start()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(38.))
                    .rounded(px(10.))
                    .bg(rgba(0xffffff0f))
                    .border_1()
                    .border_color(rgba(GLASS_EDGE))
                    .text_size(px(17.))
                    .text_color(rgb(ACCENT_HI))
                    .child("⌘"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(FG))
                            .child(if current.is_some() { "Change shortcut" } else { "Set a shortcut" }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(format!("Hold to dictate with {}", label))
                            .when_some(current.clone(), |d, spec| {
                                d.child(div().text_color(rgb(SUBTLE)).child("· now"))
                                    .child(keycaps(&spec, false))
                            }),
                    ),
            )
            .child(
                div()
                    .id("capture-x")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(SUBTLE))
                    .hover(|s| s.bg(rgb(SURFACE2)).text_color(rgb(FG)))
                    .child("✕")
                    .on_click(cx.listener(|v, _e, _w, cx| {
                        v.cancel_capture();
                        cx.notify();
                    })),
            );

        let footer_left: AnyElement = match &current {
            Some(_) => button_base("Remove shortcut")
                .text_color(rgb(DANGER))
                .hover(|s| s.bg(rgba(0xef44441f)).text_color(rgb(DANGER)))
                .id("capture-remove")
                .on_click(cx.listener(|v, _e, _w, cx| {
                    if let Some(id) = v.capturing_for.clone() {
                        actions::clear_hotkey(&id);
                    }
                    v.cancel_capture();
                    cx.notify();
                }))
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(keycap("esc".into(), false))
                .child(div().text_size(px(12.)).text_color(rgb(SUBTLE)).child("cancel"))
                .into_any_element(),
        };

        let secondary: gpui::Stateful<gpui::Div> = if pending.is_some() {
            btn_ghost("Try again").id("capture-retry").on_click(cx.listener(|v, _e, _w, cx| {
                v.capture_pending = None;
                v.capture_mods = None;
                cx.notify();
            }))
        } else {
            btn_ghost("Cancel").id("capture-cancel").on_click(cx.listener(|v, _e, _w, cx| {
                v.cancel_capture();
                cx.notify();
            }))
        };

        let save = button_base("Save")
            .id("capture-save")
            .child(
                div()
                    .text_size(px(11.))
                    .opacity(0.7)
                    .child("↩"),
            )
            .when(can_save, |d| {
                d.bg(rgb(ACCENT))
                    .text_color(rgb(ON_ACCENT))
                    .hover(|s| s.bg(rgb(ACCENT_HI)))
                    .on_click(cx.listener(|v, _e, _w, cx| {
                        v.commit_capture();
                        cx.notify();
                    }))
            })
            .when(!can_save, |d| {
                d.bg(rgb(SURFACE2)).text_color(rgb(SUBTLE)).cursor_default()
            });

        let card = div()
            .id("capture-card")
            .flex()
            .flex_col()
            .gap(px(18.))
            .w(px(468.))
            .p(px(22.))
            .rounded(px(18.))
            .bg(rgba(0x111113f2))
            .border_1()
            .border_color(rgba(0xffffff1f))
            .shadow_2xl()
            .on_mouse_down_out(cx.listener(|v, _e, _w, cx| {
                v.cancel_capture();
                cx.notify();
            }))
            .child(header)
            .child(stage)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(SUBTLE))
                                    .child("SUGGESTED"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(SUBTLE))
                                    .child("Modifier-only combos never steal a key"),
                            ),
                    )
                    .child(picks),
            )
            .child(div().h(px(1.)).bg(rgb(HAIRLINE)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(footer_left)
                    .child(div().flex().items_center().gap(px(8.)).child(secondary).child(save)),
            )
            .with_animation(
                "capture-in",
                Animation::new(Duration::from_millis(180)).with_easing(ease_out_quint()),
                |d, t| d.opacity(t).mt(px(10. * (1. - t))),
            );

        // Bare modifiers produce no key events — watch flags changes and
        // stage whatever modifier set was held once everything is released.
        let view = cx.entity();
        div()
            .id("capture")
            .track_focus(&self.focus)
            .focusable()
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x050508b3))
            .on_modifiers_changed(move |ev, _w, cx| {
                let m = &ev.modifiers;
                let bits = (m.control as u8) + (m.alt as u8) + (m.shift as u8) + (m.platform as u8);
                let _ = view.update(cx, |v, vc| {
                    if v.capturing_for.is_none() {
                        return;
                    }
                    if bits == 0 {
                        v.capture_keyed = false;
                        if let Some((_, spec)) = v.capture_mods.take() {
                            v.capture_pending = Some(spec);
                        }
                    } else if v.capture_keyed {
                        // Releasing the modifiers of a keyed combo — ignore.
                    } else if let Some(spec) = hotkeys::from_modifiers(m) {
                        // Keep the biggest set seen — releasing one key of a
                        // combo fires an intermediate single-modifier event
                        // that must not shrink the pending spec.
                        if v.capture_mods.as_ref().map_or(true, |(b, _)| bits > *b) {
                            v.capture_mods = Some((bits, spec));
                        }
                    }
                    vc.notify();
                });
            })
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, _w, cx| {
                let key = e.keystroke.key.to_lowercase();
                let m = &e.keystroke.modifiers;
                let bare = !(m.control || m.alt || m.shift || m.platform);
                if bare && (key == "escape" || key == "esc") {
                    v.cancel_capture();
                } else if bare && key == "enter" && v.capture_pending.is_some() {
                    v.commit_capture();
                } else if let Some(spec) = hotkeys::from_gpui(&e.keystroke) {
                    v.capture_pending = Some(spec);
                    v.capture_mods = None;
                    v.capture_keyed = !bare;
                }
                cx.notify();
            }))
            .child(card)
    }
}

/// Validate a captured shortcut: (tone, message, can save).
fn assess(spec: &str, model_id: &str) -> (u32, String, bool) {
    let Some(canon) = hotkeys::canonical(spec) else {
        return (DANGER, "That key can't be used globally — try another.".into(), false);
    };
    let shown = hotkeys::display(&canon);
    let parts: Vec<&str> = canon.split('+').collect();
    let mods = parts.iter().filter(|p| matches!(**p, "ctrl" | "alt" | "shift" | "cmd")).count();
    let key = parts.iter().find(|p| !matches!(**p, "ctrl" | "alt" | "shift" | "cmd"));
    let fkey = key.map_or(false, |k| k.len() > 1 && k.starts_with('f') && k[1..].parse::<u8>().is_ok());
    if key.is_some() && mods == 0 && !fkey {
        return (DANGER, "A key on its own would stop you typing it — add ⌃ or ⌥.".into(), false);
    }
    let cfg = state::state().cfg.lock().unwrap();
    if cfg.hotkeys.get(model_id).and_then(|c| hotkeys::canonical(c)).as_deref() == Some(&canon) {
        return (GREEN, "That's already your shortcut.".into(), true);
    }
    if let Some((other, _)) = cfg
        .hotkeys
        .iter()
        .find(|(k, v)| *k != model_id && hotkeys::canonical(v).as_deref() == Some(&canon))
    {
        let name = models::by_id(other).map(|m| m.label).unwrap_or("another model");
        return (AMBER, format!("Used by {} — saving moves it here.", name), true);
    }
    const SYSTEM: &[&str] = &[
        "cmd+space", "ctrl+space", "cmd+tab", "cmd+q", "cmd+w", "cmd+c", "cmd+v", "cmd+x",
        "cmd+z", "cmd+a", "cmd+s", "cmd+shift+3", "cmd+shift+4", "cmd+shift+5", "cmd+alt+escape",
    ];
    if SYSTEM.contains(&canon.as_str()) {
        return (AMBER, format!("{} is a common macOS shortcut — it will stop working.", shown), true);
    }
    if key.is_none() && mods == 1 {
        return (AMBER, "A single modifier fires easily — a pair like ⌃⌥ is safer.".into(), true);
    }
    (GREEN, format!("Hold {} anywhere to dictate.", shown), true)
}

fn stage_key(label: String, tone: u32) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(58.))
        .min_w(px(58.))
        .px(px(16.))
        .rounded(px(12.))
        .bg(rgb(SURFACE3))
        .border_1()
        .border_b(px(4.))
        .border_color(rgb(tone))
        .shadow_md()
        .text_size(px(24.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(FG))
        .child(label)
}

fn stage_keys(spec: &str, tone: u32) -> gpui::Div {
    let mut row = div().flex().items_center().gap(px(8.));
    for (i, part) in spec.split('+').filter(|p| !p.is_empty()).enumerate() {
        if i > 0 {
            row = row.child(div().text_size(px(16.)).text_color(rgb(SUBTLE)).child("+"));
        }
        row = row.child(stage_key(hotkeys::display(part), tone));
    }
    row
}

fn stage_placeholder() -> gpui::Div {
    let mut row = div().flex().items_center().gap(px(8.));
    for (i, g) in ["⌃", "⌥", "…"].into_iter().enumerate() {
        if i > 0 {
            row = row.child(div().text_size(px(16.)).text_color(rgb(SURFACE3)).child("+"));
        }
        row = row.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(58.))
                .rounded(px(12.))
                .border_1()
                .border_dashed()
                .border_color(rgb(SURFACE3))
                .text_size(px(22.))
                .text_color(rgb(SURFACE3))
                .child(g),
        );
    }
    row
}

fn step_row(
    n: u8,
    done: bool,
    enabled: bool,
    title: &str,
    subtitle: &str,
    action: AnyElement,
) -> AnyElement {
    let badge = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(24.))
        .rounded_full()
        .text_size(px(12.))
        .font_weight(FontWeight::BOLD)
        .when(done, |d| d.bg(rgb(FG)).text_color(rgb(ON_ACCENT)).child("✓"))
        .when(!done, |d| {
            d.border_1()
                .border_color(rgb(if enabled { ACCENT } else { SURFACE3 }))
                .text_color(rgb(if enabled { ACCENT_HI } else { SUBTLE }))
                .child(n.to_string())
        });
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(16.))
        .py(px(14.))
        .child(badge)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if done { MUTED } else { FG }))
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(SUBTLE))
                        .child(subtitle.to_string()),
                ),
        )
        .child(action)
        .into_any_element()
}

fn mode_segments(mode: &str, cx: &mut Context<ThinSttView>) -> gpui::Div {
    let mut segs = div()
        .flex()
        .flex_none()
        .items_center()
        .p(px(2.))
        .rounded(px(8.))
        .bg(rgba(0x0000004d))
        .border_1()
        .border_color(rgba(GLASS_EDGE));
    for id in ["type", "paste", "clipboard"] {
        let active = mode == id;
        segs = segs.child(
            div()
                .id(SharedString::from(format!("mode-{}", id)))
                .h(px(24.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |d| d.bg(rgb(SURFACE3)).text_color(rgb(FG)).shadow_sm())
                .when(!active, |d| d.text_color(rgb(MUTED)).hover(|s| s.text_color(rgb(FG))))
                .child(mode_label(id))
                .on_click(cx.listener(move |_v, _e, _w, cx| {
                    actions::set_paste_mode(id);
                    cx.notify();
                })),
        );
    }
    segs
}

const CAMERA_AUTHORIZED: i64 = 3;
const CAMERA_NOT_DETERMINED: i64 = 0;

fn source_switch(lips: bool, cx: &mut Context<ThinSttView>) -> gpui::Div {
    let mut row = div()
        .flex()
        .gap(px(6.))
        .p(px(4.))
        .rounded(px(12.))
        .bg(rgba(0x0000004d))
        .border_1()
        .border_color(rgba(GLASS_EDGE));
    for (id, title, sub, glyph) in [
        ("voice", "Voice", "Microphone · speak out loud", "◉"),
        ("lips", "Lips", "Camera · silent, no audio", "◐"),
    ] {
        let active = (id == "lips") == lips;
        row = row.child(
            div()
                .id(SharedString::from(format!("src-{}", id)))
                .flex_1()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(9.))
                .border_1()
                .cursor_pointer()
                .when(active, |d| d.bg(rgb(SURFACE3)).border_color(rgba(0xffffff1f)).shadow_sm())
                .when(!active, |d| d.border_color(rgba(0x00000000)).hover(|s| s.bg(rgba(0xffffff0a))))
                .child(
                    div()
                        .size(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.))
                        .bg(rgba(if active { 0xffffff1a } else { 0xffffff0a }))
                        .text_size(px(15.))
                        .text_color(rgb(if active { FG } else { SUBTLE }))
                        .child(glyph),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(if active { FG } else { MUTED }))
                                .child(title),
                        )
                        .child(div().text_size(px(12.)).text_color(rgb(SUBTLE)).child(sub)),
                )
                .when(active, |d| d.child(div().flex_1()).child(dot(GREEN, 6.)))
                .on_click(cx.listener(move |_v, _e, _w, cx| {
                    if id == "voice" {
                        let _ = camera::set_preview(false);
                    }
                    actions::set_input_source(id);
                    cx.notify();
                })),
        );
    }
    row
}

fn source_segments(lips: bool, cx: &mut Context<ThinSttView>) -> gpui::Div {
    let mut segs = div()
        .flex()
        .flex_none()
        .items_center()
        .p(px(2.))
        .rounded(px(8.))
        .bg(rgba(0x0000004d))
        .border_1()
        .border_color(rgba(GLASS_EDGE));
    for (id, label) in [("voice", "Voice"), ("lips", "Lips")] {
        let active = (id == "lips") == lips;
        segs = segs.child(
            div()
                .id(SharedString::from(format!("seg-src-{}", id)))
                .h(px(24.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |d| d.bg(rgb(SURFACE3)).text_color(rgb(FG)).shadow_sm())
                .when(!active, |d| d.text_color(rgb(MUTED)).hover(|s| s.text_color(rgb(FG))))
                .child(label)
                .on_click(cx.listener(move |_v, _e, _w, cx| {
                    actions::set_input_source(id);
                    cx.notify();
                })),
        );
    }
    segs
}

fn camera_access(perm: i64, cx: &mut Context<ThinSttView>) -> AnyElement {
    match perm {
        CAMERA_AUTHORIZED => tag("✓ Granted", GREEN, 0x22c55e1f).into_any_element(),
        CAMERA_NOT_DETERMINED => btn_primary("Allow camera")
            .id("cam-allow")
            .on_click(cx.listener(|_v, _e, _w, cx| {
                camera::request_permission(|_| state::emit(state::UiEvent::StateChanged));
                cx.notify();
            }))
            .into_any_element(),
        _ => btn_secondary("Open Privacy Settings")
            .id("cam-settings")
            .on_click(cx.listener(|_v, _e, _w, _cx| {
                let _ = std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Camera")
                    .spawn();
            }))
            .into_any_element(),
    }
}

impl ThinSttView {
    fn lips_panel(
        &mut self,
        spec: Option<String>,
        first_model: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let perm = camera::permission();
        let has_cam = camera::has_camera();
        let model = lips::best_model();
        let (previewing, face) = camera::LIVE
            .lock()
            .unwrap()
            .as_ref()
            .map_or((false, false), |l| (l.previewing, l.face));
        let ready = perm == CAMERA_AUTHORIZED && model.is_some() && spec.is_some() && has_cam;

        // ---- live framing preview
        let screen = div()
            .relative()
            .flex_none()
            .w(px(240.))
            .h(px(180.))
            .rounded(px(12.))
            .overflow_hidden()
            .border_1()
            .border_color(rgba(if previewing && face { 0x22c55e80 } else { GLASS_EDGE }))
            .bg(rgb(0x000000));
        let screen = match (&self.preview_img, previewing) {
            (Some((_, frame)), true) => screen
                .child(gpui::img(frame.clone()).size_full().object_fit(gpui::ObjectFit::Cover))
                .child(
                    div()
                        .absolute()
                        .left(px(8.))
                        .bottom(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .py(px(3.))
                        .rounded_full()
                        .bg(rgba(0x000000b3))
                        .child(dot(if face { GREEN } else { AMBER }, 6.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(FG))
                                .child(if face { "Lips in frame" } else { "Looking for your face…" }),
                        ),
                ),
            _ => screen.flex().flex_col().items_center().justify_center().gap(px(10.)).child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(SUBTLE))
                    .child(if !has_cam { "No camera found" } else if previewing { "Starting camera…" } else { "Camera off" }),
            ),
        };
        let toggle_preview: AnyElement = if previewing {
            btn_secondary("Stop preview")
                .id("cam-preview")
                .on_click(cx.listener(|_v, _e, _w, cx| {
                    let _ = camera::set_preview(false);
                    cx.notify();
                }))
                .into_any_element()
        } else if perm == CAMERA_AUTHORIZED && has_cam {
            btn_secondary("Check framing")
                .id("cam-preview")
                .on_click(cx.listener(|_v, _e, _w, cx| {
                    if let Err(e) = camera::set_preview(true) {
                        state::status_err(format!("camera: {}", e));
                    }
                    cx.notify();
                }))
                .into_any_element()
        } else {
            camera_access(perm, cx)
        };

        let headline = match &spec {
            Some(spec) => div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().text_size(px(15.)).text_color(rgb(MUTED)).child("Hold"))
                .child(keycaps(spec, true))
                .child(div().text_size(px(15.)).text_color(rgb(MUTED)).child("and mouth your words")),
            None => div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(FG))
                .child("Set up silent dictation"),
        };
        let info = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(dot(if ready { GREEN } else { AMBER }, 7.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(if ready { GREEN } else { AMBER }))
                            .child(if ready { "LIPS READY" } else { "LIPS SETUP" }),
                    ),
            )
            .child(headline)
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child("No microphone and no audio. Only small grayscale mouth crops are kept in memory, decoded on this Mac, then dropped."),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(tag(model.map_or("No lip model", |m| m.label), FG, 0xffffff14))
                    .child(tag("Mic off", MUTED, 0xffffff0d))
                    .child(tag("Never saved", MUTED, 0xffffff0d))
                    .child(tag("No polish", MUTED, 0xffffff0d)),
            )
            .child(div().flex().child(toggle_preview));
        let hero = div()
            .flex()
            .items_center()
            .gap(px(24.))
            .rounded(px(16.))
            .border_1()
            .border_color(rgba(GLASS_EDGE))
            .shadow_lg()
            .bg(linear_gradient(
                160.,
                linear_color_stop(rgba(0xffffff12), 0.),
                linear_color_stop(rgba(0xffffff03), 1.),
            ))
            .p(px(24.))
            .child(screen)
            .child(info);

        // ---- checklist
        let flare = models::by_id(lips::FLARE_MODEL_ID);
        let flare_dl = flare.map_or(false, |m| self.is_downloading(m.id));
        let model_action: AnyElement = if model.is_some() {
            div().into_any_element()
        } else if flare_dl {
            let (done, total) = self.progress.get(lips::FLARE_MODEL_ID).copied().unwrap_or((0, 0));
            let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(progress_bar(frac, 120.))
                .child(div().w(px(56.)).text_size(px(12.)).text_color(rgb(MUTED)).child(
                    if total > 0 { format!("{:.0}%", frac * 100.0) } else { "Starting…".to_string() },
                ))
                .child(btn_ghost("Cancel").id("lips-dlc").on_click(cx.listener(|_v, _e, _w, cx| {
                    state::cancel_download(lips::FLARE_MODEL_ID);
                    cx.notify();
                })))
                .into_any_element()
        } else {
            div()
                .flex()
                .gap(px(6.))
                .child(btn_ghost("Browse").id("lips-browse").on_click(cx.listener(|v, _e, _w, cx| {
                    v.page = Page::Models;
                    cx.notify();
                })))
                .child(btn_primary("Get FLARE Large · 430 MB").id("lips-dl").on_click(cx.listener(|_v, _e, _w, cx| {
                    actions::start_download(lips::FLARE_MODEL_ID);
                    cx.notify();
                })))
                .into_any_element()
        };
        let model_ready = model.is_some();
        let shortcut_target = first_model.or(model.map(|m| m.id));
        let shortcut_action: AnyElement = match (&spec, shortcut_target) {
            (None, Some(id)) => btn_primary("Choose shortcut")
                .id("lips-hk")
                .on_click(cx.listener(move |v, _e, w, cx| v.start_capture(id, w, cx)))
                .into_any_element(),
            _ => div().into_any_element(),
        };
        let cam_ok = perm == CAMERA_AUTHORIZED;
        let steps = vec![
            step_row(1, cam_ok && has_cam, has_cam, "Allow camera access",
                if has_cam { "Used only while your shortcut is held in Lips mode." } else { "Connect a camera to use Lips mode." },
                if cam_ok { div().into_any_element() } else { camera_access(perm, cx) }),
            step_row(2, model_ready, true, "Download a lip-reading model",
                "FLARE Large is the most accurate open model: 15.9% word error on LRS3.",
                model_action),
            step_row(3, spec.is_some(), model_ready, "Choose a hold-to-read shortcut",
                "Every shortcut reads lips while Lips mode is on.",
                shortcut_action),
            step_row(4, previewing && face, cam_ok, "Check your framing",
                "Face the camera in good light with your whole mouth visible.",
                div().into_any_element()),
        ];
        let mut out = vec![hero.into_any_element()];
        out.push(group("LIPS SETUP", Some("works best with full sentences, mouthed clearly"), steps).into_any_element());
        out
    }
}

fn polish_toggle(on: bool, ready: bool, cx: &mut Context<ThinSttView>) -> gpui::Stateful<gpui::Div> {
    toggle(on)
        .when(!ready, |d| d.opacity(0.45))
        .id("polish-toggle")
        .on_click(cx.listener(move |v, _e, _w, cx| {
            if ready {
                actions::toggle_polish();
            } else {
                v.page = Page::Models;
                state::status_err("Download the Qwen3 polish model first");
            }
            cx.notify();
        }))
}

impl Render for ThinSttView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_preview(window);
        let body = match self.page {
            Page::Dictate => self.render_dictate(cx),
            Page::Models => self.render_models(cx),
            Page::Settings => self.render_settings(cx),
        };

        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgba(CONTENT_TINT))
            .child(div().h(px(TITLEBAR_H - 12.)).flex_none())
            .child(
                div()
                    .id("scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(760.))
                            .mx_auto()
                            .px(px(36.))
                            .pb(px(36.))
                            .child(body),
                    ),
            );

        div()
            .relative()
            .flex()
            .size_full()
            .text_color(rgb(FG))
            .font_family(".SystemUIFont")
            .child(self.render_sidebar(cx))
            .child(content)
            .when(self.capturing_for.is_some(), |d| d.child(self.render_capture(cx)))
    }
}
