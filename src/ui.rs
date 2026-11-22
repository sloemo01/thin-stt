use std::collections::HashMap;

use gpui::{
    div, prelude::*, px, rgb, Context, FocusHandle, KeyDownEvent, SharedString, Window,
};

use crate::{actions, audio, hotkeys, models, state, typer};

const BG: u32 = 0x141419;
const CARD: u32 = 0x1e1e26;
const CARD2: u32 = 0x2a2a35;
const BORDER: u32 = 0x2e2e38;
const FG: u32 = 0xe8e8ee;
const MUTED: u32 = 0x9a9aa8;
const ACCENT: u32 = 0x4f8cff;
const GREEN: u32 = 0x3fb950;
const DANGER: u32 = 0xe5484d;
const DANGER_BG: u32 = 0x2a1d20;

pub struct ThinSttView {
    status: String,
    status_error: bool,
    capturing_for: Option<String>,
    // Largest modifier set held so far during capture — committed when
    // everything is released, so Ctrl+Option assigns as one combo.
    capture_mods: Option<(u8, String)>,
    progress: HashMap<String, (u64, u64)>,
    mic_open: bool,
    confirm_delete: bool,
    focus: FocusHandle,
}

impl ThinSttView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        ThinSttView {
            status: "ready".into(),
            status_error: false,
            capturing_for: None,
            capture_mods: None,
            progress: HashMap::new(),
            mic_open: false,
            confirm_delete: false,
            focus: cx.focus_handle(),
        }
    }

    pub fn set_status(&mut self, text: String, error: bool) {
        self.status = text;
        self.status_error = error;
    }

    pub fn set_progress(&mut self, id: String, done: u64, total: u64) {
        self.progress.insert(id, (done, total));
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn dot(color: u32) -> gpui::Div {
    div()
        .w(px(7.))
        .h(px(7.))
        .rounded_full()
        .bg(rgb(color))
        .flex_none()
}

fn chip(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .px_2()
        .py(px(2.))
        .rounded_md()
        .bg(rgb(CARD2))
        .text_xs()
        .text_color(rgb(MUTED))
        .child(label.into())
}

fn status_pill(text: impl Into<SharedString>, color: u32) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded_full()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(BORDER))
        .child(dot(color))
        .child(div().text_xs().text_color(rgb(MUTED)).child(text.into()))
}

fn section_label(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(label.into())
}

fn row<E: IntoElement>(label: impl Into<SharedString>, control: E) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_sm().child(label.into()))
        .child(control)
}

fn card(children: gpui::Div) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(BORDER))
        .child(children)
}

fn pill_btn(label: impl Into<SharedString>, bg: u32) -> gpui::Div {
    div()
        .px_3()
        .py_1()
        .rounded_md()
        .bg(rgb(bg))
        .text_sm()
        .text_color(rgb(0xffffff))
        .cursor_pointer()
        .child(label.into())
}

fn ghost_btn(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .text_sm()
        .text_color(rgb(MUTED))
        .cursor_pointer()
        .child(label.into())
}

impl ThinSttView {
    fn render_model(&mut self, m: &'static models::ModelInfo, cx: &mut Context<Self>) -> gpui::Div {
        let s = state::state();
        let cfg = s.cfg.lock().unwrap().clone();
        let downloaded = models::is_downloaded(m);
        let downloading = self.progress.contains_key(m.id)
            && s.downloads.lock().unwrap().contains_key(m.id);
        let hotkey = cfg.hotkeys.get(m.id).cloned();
        let remote_mb = cfg.remote_sizes.get(m.id).copied();
        let disk = models::disk_size_mb(m.id);

        // ---- line 1: name + status
        let (state_txt, state_color) = if downloading {
            let (done, total) = self.progress.get(m.id).copied().unwrap_or((0, 0));
            let label = if total > 0 {
                format!("downloading {:.0}%", 100.0 * done as f32 / total as f32)
            } else {
                "downloading".to_string()
            };
            (label, ACCENT)
        } else if downloaded {
            ("ready".to_string(), GREEN)
        } else {
            ("not installed".to_string(), MUTED)
        };
        let mut meta = div()
            .flex()
            .items_center()
            .gap_2()
            .child(chip(m.badge))
            .child(div().text_xs().text_color(rgb(MUTED)).child(m.desc));
        meta = meta.child(
            div()
                .flex_1()
                .flex()
                .justify_end()
                .flex()
                .items_center()
                .gap_1()
                .child(dot(state_color))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(state_color))
                        .child(state_txt),
                ),
        );

        // ---- line 2: controls
        let mut controls = div().flex().items_center().gap_2();

        let hk_label = hotkey
            .as_deref()
            .map(|h| hotkeys::display(h))
            .unwrap_or_else(|| "no hotkey".into());
        controls = controls.child(
            div()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(rgb(CARD2))
                .text_xs()
                .text_color(if hotkey.is_some() {
                    rgb(ACCENT)
                } else {
                    rgb(MUTED)
                })
                .child(hk_label),
        );
        let mid: SharedString = m.id.into();
        if downloaded {
            controls = controls.child(
                ghost_btn("Set hotkey").id(SharedString::from(format!("hk-{}", mid))).on_click(
                    cx.listener(move |v, _e, window, cx| {
                        v.capturing_for = Some(mid.to_string());
                        v.capture_mods = None;
                        window.focus(&v.focus);
                        cx.notify();
                    }),
                ),
            );
            if hotkey.is_some() {
                let mid = m.id;
                controls = controls.child(
                    ghost_btn("✕").id(SharedString::from(format!("hkx-{}", m.id))).on_click(
                        cx.listener(move |_v, _e, _w, cx| {
                            actions::clear_hotkey(mid);
                            cx.notify();
                        }),
                    ),
                );
            }
        }
        controls = controls.child(div().flex_1());

        if downloading {
            let (done, total) = self.progress.get(m.id).copied().unwrap_or((0, 0));
            let frac = if total > 0 {
                (done as f32 / total as f32).min(1.0)
            } else {
                0.0
            };
            let mid = m.id;
            controls = controls.child(
                div().flex().items_center().gap_2().child(
                    div().flex().flex_col().gap_1().w(px(140.)).child(
                        div()
                            .w_full()
                            .h(px(6.))
                            .rounded_full()
                            .bg(rgb(CARD2))
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(rgb(ACCENT))
                                    .w(gpui::relative(frac)),
                            ),
                    ).child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(if total > 0 {
                                format!("{} / {}", mb(done), mb(total))
                            } else {
                                format!("{} downloaded", mb(done))
                            }),
                    ),
                ).child(
                    ghost_btn("Cancel").id(SharedString::from(format!("dlc-{}", m.id))).on_click(
                        cx.listener(move |_v, _e, _w, cx| {
                            state::cancel_download(mid);
                            cx.notify();
                        }),
                    ),
                ),
            );
        } else if downloaded {
            controls = controls.child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(format!("{:.0} MB on disk", disk)),
            );
            let mid = m.id;
            controls = controls.child(
                ghost_btn("Delete").id(SharedString::from(format!("del-{}", m.id))).on_click(
                    cx.listener(move |_v, _e, _w, cx| {
                        actions::delete_model(mid);
                        cx.notify();
                    }),
                ),
            );
        } else {
            let size_lbl = remote_mb.map(|b| mb(b)).unwrap_or_else(|| "? MB".into());
            controls = controls.child(div().text_xs().text_color(rgb(MUTED)).child(size_lbl));
            let mid = m.id;
            controls = controls.child(
                pill_btn("Download", ACCENT)
                    .id(SharedString::from(format!("dl-{}", m.id)))
                    .on_click(cx.listener(move |_v, _e, _w, cx| {
                        actions::start_download(mid);
                        cx.notify();
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded_lg()
            .bg(rgb(CARD))
            .border_1()
            // accent ring marks the model your hotkey drives
            .border_color(rgb(if hotkey.is_some() { ACCENT } else { BORDER }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(m.label),
                    ),
            )
            .child(meta)
            .child(controls)
    }

    fn render_capture(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let m = self.capturing_for.clone().unwrap_or_default();
        let live = self
            .capture_mods
            .as_ref()
            .map(|(_, spec)| hotkeys::display(spec));

        // Bare modifiers produce no key events — watch flags changes and
        // assign whatever modifier set is currently held.
        let view = cx.entity();
        div()
            .id("capture")
            .track_focus(&self.focus)
            .focusable()
            .on_modifiers_changed(move |ev, _w, cx| {
                let m = &ev.modifiers;
                let bits = (m.control as u8) + (m.alt as u8) + (m.shift as u8) + (m.platform as u8);
                let _ = view.update(cx, |v, vc| {
                    if v.capturing_for.is_none() {
                        return;
                    }
                    if bits == 0 {
                        if let Some((_, spec)) = v.capture_mods.take() {
                            if let Some(id) = v.capturing_for.take() {
                                actions::set_hotkey(&id, &spec);
                            }
                        }
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
                if key == "escape" || key == "esc" {
                    v.capturing_for = None;
                    v.capture_mods = None;
                    cx.notify();
                    return;
                }
                if let Some(spec) = hotkeys::from_gpui(&e.keystroke) {
                    if let Some(id) = v.capturing_for.take() {
                        actions::set_hotkey(&id, &spec);
                    }
                }
                cx.notify();
            }))
            .flex()
            .flex_col()
            .size_full()
            .justify_center()
            .items_center()
            .gap_4()
            .bg(rgb(BG))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!("Assign hotkey — {}", m)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(160.))
                    .h(px(56.))
                    .rounded_lg()
                    .bg(rgb(CARD))
                    .border_1()
                    .border_color(rgb(ACCENT))
                    .child(
                        div()
                            .text_2xl()
                            .text_color(if live.is_some() { rgb(ACCENT) } else { rgb(MUTED) })
                            .child(live.unwrap_or_else(|| "…".into())),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("Press the key combination — release to assign · Esc cancels"),
            )
    }

    fn render_settings(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let s = state::state();
        let cfg = s.cfg.lock().unwrap().clone();
        let mode = cfg.paste_mode.clone().unwrap_or_else(|| "type".into());
        let mic = cfg.mic_device.clone();
        let trusted = typer::accessibility_trusted();

        let mut wrap = div().flex().flex_col().gap_3();

        // ---------- Preferences
        wrap = wrap.child(section_label("Preferences"));
        let mut prefs = div().flex().flex_col().gap_3();

        // microphone
        prefs = prefs.child(row(
            "Microphone",
            ghost_btn(mic.clone().unwrap_or_else(|| "System default".into()))
                .id("mic-toggle")
                .on_click(cx.listener(|v, _e, _w, cx| {
                    v.mic_open = !v.mic_open;
                    cx.notify();
                })),
        ));
        if self.mic_open {
            let mut list = div().flex().flex_col().gap_1();
            let mut names = vec!["System default".to_string()];
            names.extend(audio::input_device_names());
            for name in names {
                let chosen = if name == "System default" {
                    None
                } else {
                    Some(name.clone())
                };
                list = list.child(
                    div()
                        .id(SharedString::from(format!("mic-{}", name)))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(CARD2))
                        .text_sm()
                        .cursor_pointer()
                        .child(name)
                        .on_click(cx.listener(move |v, _e, _w, cx| {
                            actions::set_mic(chosen.clone());
                            v.mic_open = false;
                            cx.notify();
                        })),
                );
            }
            prefs = prefs.child(list);
        }

        // insert mode — segmented control
        let mut segs = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(2.))
            .rounded_md()
            .bg(rgb(CARD2));
        for (id, label) in [
            ("type", "Type"),
            ("paste", "Paste"),
            ("clipboard", "Clipboard"),
        ] {
            let active = mode == id;
            segs = segs.child(
                div()
                    .id(SharedString::from(format!("mode-{}", id)))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |d| d.bg(rgb(ACCENT)).text_color(rgb(0xffffff)))
                    .when(!active, |d| d.text_color(rgb(MUTED)))
                    .child(label)
                    .on_click(cx.listener(move |_v, _e, _w, cx| {
                        actions::set_paste_mode(id);
                        cx.notify();
                    })),
            );
        }
        prefs = prefs.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(row("Insert mode", segs))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("how dictated text reaches the focused app"),
                ),
        );

        // permissions
        let acc_btn: gpui::Div = if trusted {
            status_pill("Accessibility granted", GREEN)
        } else {
            pill_btn("Enable typing access", ACCENT)
        };
        prefs = prefs.child(row(
            "Permissions",
            acc_btn.id("acc").on_click(cx.listener(|_v, _e, _w, cx| {
                typer::request_accessibility();
                cx.notify();
            })),
        ));
        wrap = wrap.child(card(prefs));

        // ---------- Maintenance
        wrap = wrap.child(section_label("Maintenance"));
        let mut maint = div().flex().flex_col().gap_3();
        maint = maint.child(row(
            "Updates",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                )
                .child(
                    ghost_btn("Check for updates")
                        .id("check-update")
                        .on_click(cx.listener(|_v, _e, _w, cx| {
                            actions::check_for_updates();
                            cx.notify();
                        })),
                ),
        ));
        let cache_mb: u64 = models::MODELS
            .iter()
            .map(|m| models::disk_size_mb(m.id) as u64)
            .sum();
        maint = maint.child(row(
            "Model cache",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(format!("{} MB used", cache_mb)),
                )
                .child(
                    ghost_btn("Clear cache")
                        .id("clear-cache")
                        .on_click(cx.listener(|_v, _e, _w, cx| {
                            actions::clear_model_cache();
                            cx.notify();
                        })),
                ),
        ));
        wrap = wrap.child(card(maint));

        // ---------- Danger zone
        if self.confirm_delete {
            let confirm = div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(DANGER))
                        .child("Delete all data + the app?"),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            pill_btn("Delete everything", DANGER)
                                .id("uninstall-yes")
                                .on_click(cx.listener(|_v, _e, _w, cx| {
                                    actions::uninstall();
                                    cx.quit();
                                })),
                        )
                        .child(
                            ghost_btn("Cancel").id("uninstall-no").on_click(
                                cx.listener(|v, _e, _w, cx| {
                                    v.confirm_delete = false;
                                    cx.notify();
                                }),
                            ),
                        ),
                );
            wrap = wrap.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgb(DANGER_BG))
                    .border_1()
                    .border_color(rgb(DANGER))
                    .child(confirm),
            );
        } else {
            wrap = wrap.child(
                div()
                    .flex()
                    .justify_center()
                    .child(
                        ghost_btn("Completely delete this app")
                            .id("uninstall")
                            .on_click(cx.listener(|v, _e, _w, cx| {
                                v.confirm_delete = true;
                                cx.notify();
                            })),
                    ),
            );
        }

        wrap
    }
}

impl Render for ThinSttView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.capturing_for.is_some() {
            return self.render_capture(window, cx);
        }

        let cfg = state::state().cfg.lock().unwrap().clone();
        let any_hotkey = cfg.hotkeys.values().next().cloned();
        let any_downloaded = models::MODELS.iter().any(models::is_downloaded);
        let hint = if let Some(hk) = any_hotkey {
            format!("Hold {} to dictate — release to type at the cursor", hotkeys::display(&hk))
        } else if any_downloaded {
            "Set a hotkey on a model below to start dictating".to_string()
        } else {
            "Download a model below to start dictating".to_string()
        };

        let mut models_list = div().flex().flex_col().gap_3();
        for m in models::MODELS {
            models_list = models_list.child(self.render_model(m, cx));
        }

        div()
            .id("main")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(BG))
            .text_color(rgb(FG))
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Thin STT"),
                    )
                    .child(status_pill(
                        self.status.clone(),
                        if self.status_error { DANGER } else { GREEN },
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(hint),
            )
            .child(models_list)
            .child(self.render_settings(cx))
    }
}
