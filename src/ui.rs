use std::collections::HashMap;

use gpui::{
    div, prelude::*, px, rgb, Context, FocusHandle, KeyDownEvent, SharedString, Window,
};

use crate::{actions, audio, hotkeys, models, state, typer};

const BG: u32 = 0x141419;
const CARD: u32 = 0x1f1f27;
const CARD2: u32 = 0x262630;
const BORDER: u32 = 0x33333f;
const FG: u32 = 0xe8e8ee;
const MUTED: u32 = 0x9a9aa8;
const ACCENT: u32 = 0x4f8cff;
const GREEN: u32 = 0x3fb950;
const DANGER: u32 = 0xe5484d;

pub struct ThinSttView {
    status: String,
    status_error: bool,
    capturing_for: Option<String>,
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

        // right-hand controls
        let mut controls = div().flex().items_center().gap_2();

        // hotkey chip + assign button
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
                    rgb(FG)
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

        if downloading {
            let (done, total) = self.progress.get(m.id).copied().unwrap_or((0, 0));
            let frac = if total > 0 {
                (done as f32 / total as f32).min(1.0)
            } else {
                0.0
            };
            let mid = m.id;
            controls = controls.child(
                div().flex().flex_col().gap_1().w(px(160.)).child(
                    div()
                        .w_full()
                        .h_2()
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
            );
            controls = controls.child(
                ghost_btn("Cancel").id(SharedString::from(format!("dlc-{}", m.id))).on_click(
                    cx.listener(move |_v, _e, _w, cx| {
                        state::cancel_download(mid);
                        cx.notify();
                    }),
                ),
            );
        } else if downloaded {
            controls = controls.child(
                div()
                    .text_xs()
                    .text_color(rgb(GREEN))
                    .child(format!("ready • {:.0} MB", disk)),
            );
            let mid = m.id;
            controls = controls.child(
                pill_btn("Delete", DANGER).id(SharedString::from(format!("del-{}", m.id))).on_click(
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
            .items_center()
            .justify_between()
            .p_3()
            .rounded_lg()
            .bg(rgb(CARD))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(m.label))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(m.badge)),
            )
            .child(controls)
    }

    fn render_capture(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let m = self.capturing_for.clone().unwrap_or_default();
        div()
            .id("capture")
            .track_focus(&self.focus)
            .focusable()
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, _w, cx| {
                let key = e.keystroke.key.to_lowercase();
                if key == "escape" || key == "esc" {
                    v.capturing_for = None;
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
                    .text_xl()
                    .child(format!("Assign hotkey — {}", m)),
            )
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child("Press the key combination (Esc to cancel)"),
            )
    }

    fn render_settings(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let s = state::state();
        let cfg = s.cfg.lock().unwrap().clone();
        let mode = cfg.paste_mode.clone().unwrap_or_else(|| "type".into());
        let mic = cfg.mic_device.clone();
        let trusted = typer::accessibility_trusted();

        let mut card = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_lg()
            .bg(rgb(CARD))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Settings"),
            );

        // microphone
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().child("Microphone"))
                .child(
                    ghost_btn(mic.clone().unwrap_or_else(|| "System default".into()))
                        .id("mic-toggle")
                        .on_click(cx.listener(|v, _e, _w, cx| {
                            v.mic_open = !v.mic_open;
                            cx.notify();
                        })),
                ),
        );
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
            card = card.child(list);
        }

        // insert mode
        let mut modes = div().flex().gap_2();
        for (id, label) in [
            ("type", "Type"),
            ("paste", "Paste (⌘V)"),
            ("clipboard", "Clipboard only"),
        ] {
            let active = mode == id;
            let b = if active {
                pill_btn(label, ACCENT)
            } else {
                ghost_btn(label)
            };
            modes = modes.child(
                b.id(SharedString::from(format!("mode-{}", id)))
                    .on_click(cx.listener(move |_v, _e, _w, cx| {
                        actions::set_paste_mode(id);
                        cx.notify();
                    })),
            );
        }
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().child("Insert mode"))
                .child(modes),
        );

        // updates
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().child("Updates"))
                .child(
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
                ),
        );

        // models dir + clear cache
        let cache_mb: u64 = models::MODELS
            .iter()
            .map(|m| models::disk_size_mb(m.id) as u64)
            .sum();
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().child("Model cache"))
                .child(
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
                ),
        );

        // accessibility
        let acc_btn: gpui::Div = if trusted {
            ghost_btn("Accessibility ✓")
        } else {
            pill_btn("Enable typing access", ACCENT)
        };
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().child("Permissions"))
                .child(
                    acc_btn.id("acc").on_click(cx.listener(|_v, _e, _w, cx| {
                        typer::request_accessibility();
                        cx.notify();
                    })),
                ),
        );

        // danger zone
        if self.confirm_delete {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(DANGER))
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
                    ),
            );
        } else {
            card = card.child(
                ghost_btn("Completely delete this app")
                    .id("uninstall")
                    .on_click(cx.listener(|v, _e, _w, cx| {
                        v.confirm_delete = true;
                        cx.notify();
                    })),
            );
        }

        card
    }
}

impl Render for ThinSttView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.capturing_for.is_some() {
            return self.render_capture(cx);
        }

        let mut models_list = div().flex().flex_col().gap_2();
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
            .gap_4()
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
                    .child(
                        div()
                            .text_sm()
                            .text_color(if self.status_error {
                                rgb(DANGER)
                            } else {
                                rgb(MUTED)
                            })
                            .child(self.status.clone()),
                    ),
            )
            .child(models_list)
            .child(self.render_settings(cx))
    }
}
