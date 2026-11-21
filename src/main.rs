mod actions;
mod audio;
mod config;
mod engine;
mod hotkeys;
mod models;
mod overlay;
mod state;
mod typer;
mod ui;

use std::sync::mpsc::Receiver;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::*;
use objc::{msg_send, sel, sel_impl};
use overlay::OverlayView;
use state::{state, UiEvent};
use ui::ThinSttView;

const OVERLAY_GREEN: u32 = 0x1f8a4c;
const OVERLAY_YELLOW: u32 = 0xb58900;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if std::env::args().any(|a| a == "--selftest") {
        selftest();
        return;
    }

    let app = Application::new();
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(cx);
        }
        cx.activate(true);
    });
    app.run(|cx| {
        let (tx, rx) = std::sync::mpsc::channel::<UiEvent>();
        *state().ui.lock().unwrap() = Some(tx);

        state::reregister_hotkeys();
        install_flags_monitor();
        let swept = actions::cleanup_partial();
        if swept > 0 {
            state::status(format!(
                "cleared {:.0} MB of partial downloads",
                swept as f64 / 1048576.0
            ));
        }
        actions::fetch_remote_sizes();
        setup_tray();
        open_main_window(cx);

        cx.spawn(|cx: &mut AsyncApp| {
            let cx = cx.clone();
            async move { pump(rx, cx).await }
        })
        .detach();

        if std::env::args().any(|a| a == "--demo") {
            // Show the overlay briefly so it can be checked visually.
            state::emit(UiEvent::Overlay {
                text: "● Recording — Whisper Tiny".into(),
                color: OVERLAY_GREEN,
            });
        }
    });
}

fn open_main_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(760.0), px(640.0)), cx);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Thin STT".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                focus: true,
                show: true,
                kind: WindowKind::Normal,
                is_movable: true,
                is_resizable: true,
                is_minimizable: true,
                display_id: None,
                window_background: WindowBackgroundAppearance::Opaque,
                app_id: None,
                window_min_size: None,
                window_decorations: None,
                tabbing_identifier: None,
            },
            |window, cx| cx.new(|cx| ThinSttView::new(window, cx)),
        )
        .expect("open main window");

    // Close button hides the app into the tray instead of quitting.
    let _ = handle.update(cx, |_view, window, cx| {
        window.on_window_should_close(cx, |_window, cx| {
            cx.hide();
            false
        });
    });
    cx.activate(true);
}

fn overlay_position(cx: &App) -> Point<Pixels> {
    let w = px(360.0);
    if let Some(d) = cx.primary_display() {
        let b = d.bounds();
        let x = b.origin.x + (b.size.width - w) / 2.0;
        let y = b.origin.y + px(80.0);
        point(x, y)
    } else {
        point(px(40.0), px(80.0))
    }
}

fn show_overlay(
    cx: &mut App,
    existing: Option<WindowHandle<OverlayView>>,
    text: String,
    color: u32,
) -> Option<WindowHandle<OverlayView>> {
    if let Some(h) = existing {
        let ok = h
            .update(cx, |v, _w, cx| {
                v.text = text.clone().into();
                v.color = color;
                cx.notify();
            })
            .is_ok();
        if ok {
            return Some(h);
        }
    }
    let pos = overlay_position(cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: pos,
                size: size(px(360.0), px(52.0)),
            })),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: None,
            window_min_size: None,
            window_decorations: None,
            tabbing_identifier: None,
        },
        |_window, cx| cx.new(|_| OverlayView::new(text.clone(), color)),
    )
    .ok();
    if let Some(h) = opened {
        let _ = h.update(cx, |_v, window, _cx| {
            // Kill the NSPanel's drop shadow — it renders as a dark box
            // behind the transparent pill window.
            if let Ok(rwh) = raw_window_handle::HasWindowHandle::window_handle(window) {
                if let raw_window_handle::RawWindowHandle::AppKit(kit) = rwh.as_raw() {
                    unsafe {
                        #[allow(deprecated)]
                        let view: cocoa::base::id = kit.ns_view.as_ptr() as cocoa::base::id;
                        let ns: cocoa::base::id = msg_send![view, window];
                        #[allow(deprecated)]
                        cocoa::appkit::NSWindow::setHasShadow_(ns, cocoa::base::NO);
                        let _: () = msg_send![ns, setOpaque: false];
                        let clear: cocoa::base::id =
                            msg_send![objc::class!(NSColor), clearColor];
                        let _: () = msg_send![ns, setBackgroundColor: clear];
                        // NSTitlebarSeparatorStyleNone — the hidden titlebar
                        // otherwise leaves a hairline along the window's top.
                        let _: () = msg_send![ns, setTitlebarSeparatorStyle: 1usize];
                    }
                }
            }
        });
        return Some(h);
    }
    None
}

fn hide_overlay(cx: &mut App, overlay: &mut Option<WindowHandle<OverlayView>>) {
    if let Some(h) = overlay.take() {
        let _ = h.update(cx, |_v, window, _cx| window.remove_window());
    }
}

fn notify_main(cx: &mut App) {
    for w in cx.windows() {
        if let Some(h) = w.downcast::<ThinSttView>() {
            let _ = h.update(cx, |_v, _w, cx| cx.notify());
        }
    }
}

fn main_view(cx: &mut App, f: impl FnOnce(&mut ThinSttView, &mut Context<ThinSttView>)) {
    let mut f = Some(f);
    for w in cx.windows() {
        let Some(f) = f.take() else { return };
        if let Some(h) = w.downcast::<ThinSttView>() {
            let _ = h.update(cx, |v, _w, cx| f(v, cx));
            return;
        }
    }
}

fn hotkey_pressed(id: u32) {
    let s = state();
    if s.recorder.lock().unwrap().is_some() {
        return;
    }
    let model_id = match s.hotkeys.lock().unwrap().get(&id) {
        Some(m) => m.clone(),
        None => return,
    };
    let info = match models::by_id(&model_id) {
        Some(i) => i,
        None => return,
    };
    if !models::is_downloaded(info) {
        state::status_err(format!("{} is not downloaded", info.label));
        return;
    }
    let mic = s.cfg.lock().unwrap().mic_device.clone();
    match audio::Recorder::start(mic.as_deref()) {
        Ok(r) => {
            *s.recorder.lock().unwrap() = Some(r);
            state::emit(UiEvent::Overlay {
                text: format!("● Recording — {}", info.label),
                color: OVERLAY_GREEN,
            });
            state::status(format!("Hold hotkey… release to transcribe ({})", info.label));
        }
        Err(e) => state::status_err(format!("mic: {}", e)),
    }
}

fn hotkey_released(id: u32) {
    let s = state();
    let model_id = match s.hotkeys.lock().unwrap().get(&id) {
        Some(m) => m.clone(),
        None => return,
    };
    let rec = s.recorder.lock().unwrap().take();
    let rec = match rec {
        Some(r) => r,
        None => return,
    };
    let mode = s
        .cfg
        .lock()
        .unwrap()
        .paste_mode
        .clone()
        .unwrap_or_else(|| "type".into());
    state::emit(UiEvent::Overlay {
        text: "Transcribing…".into(),
        color: OVERLAY_YELLOW,
    });

    std::thread::spawn(move || {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let wav = config::recordings_dir().join(format!("rec-{}.wav", ts));
        let samples = rec.stop(&wav);
        if samples.len() < 16000 / 5 {
            state::emit(UiEvent::OverlayHide);
            state::status_err("recording too short");
            return;
        }
        let res = state().engine.lock().unwrap().transcribe(&model_id, &samples);
        match res {
            Ok(text) if !text.is_empty() => {
                state::emit(UiEvent::OverlayHide);
                let shown: String = text.chars().take(40).collect();
                match typer::insert_text(&text, &mode) {
                    Ok(()) => state::status(format!("Inserted \"{}\"", shown)),
                    Err(e) => state::status_err(format!("typed to clipboard (insert failed: {})", e)),
                }
            }
            Ok(_) => {
                state::emit(UiEvent::OverlayHide);
                state::status_err("no speech detected");
            }
            Err(e) => {
                state::emit(UiEvent::OverlayHide);
                state::status_err(format!("transcribe: {}", e));
            }
        }
        state::emit(UiEvent::StateChanged);
    });
}

/// Watch global modifier changes (flagsChanged) so bare modifier hotkeys —
/// "alt", "ctrl", "ctrl+alt", "cmd" — work. global-hotkey can't register
/// modifier-only combos, so we track them ourselves.
fn install_flags_monitor() {
    unsafe {
        // NSFlagsChangedMask
        let mask: u64 = 1 << 12;
        let block = block::ConcreteBlock::new(|event: cocoa::base::id| {
            let flags: u64 = msg_send![event, modifierFlags];
            state::flags_changed(flags);
        })
        .copy();
        let observer: cocoa::base::id = msg_send![
            objc::class!(NSEvent),
            addGlobalMonitorForEventsMatchingMask: mask
            handler: &*block
        ];
        if observer.is_null() {
            state::status_err(
                "modifier hotkeys need Accessibility permission (Settings → enable typing access)",
            );
        }
        std::mem::forget(block);
        std::mem::forget(observer);
    }
}

fn setup_tray() {
    let menu = muda::Menu::new();
    let show = muda::MenuItem::with_id("show", "Show Thin STT", true, None);
    let quit = muda::MenuItem::with_id("quit", "Quit Thin STT", true, None);
    let _ = menu.append_items(&[&show, &quit]);

    let icon = load_tray_icon();
    let tray = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Thin STT")
        .with_icon(icon)
        .build();
    match tray {
        Ok(t) => std::mem::forget(t), // live for the app lifetime
        Err(e) => log::error!("tray icon failed: {}", e),
    }
}

fn load_tray_icon() -> tray_icon::Icon {
    let img = image::load_from_memory(include_bytes!("../assets/tray.png"))
        .expect("tray.png")
        .to_rgba8();
    let (w, h) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).expect("tray icon")
}

async fn pump(rx: Receiver<UiEvent>, cx: AsyncApp) {
    let mut overlay: Option<WindowHandle<OverlayView>> = None;
    loop {
        while let Ok(ev) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
            match ev.state {
                global_hotkey::HotKeyState::Pressed => hotkey_pressed(ev.id),
                global_hotkey::HotKeyState::Released => hotkey_released(ev.id),
            }
        }
        while let Some((pid, pressed)) =
            state().mod_events.lock().unwrap().pop_front()
        {
            if pressed {
                hotkey_pressed(pid);
            } else {
                hotkey_released(pid);
            }
        }
        while let Ok(ev) = muda::MenuEvent::receiver().try_recv() {
            match ev.id.0.as_str() {
                "show" => {
                    let _ = cx.update(|cx| {
                        if cx.windows().is_empty() {
                            open_main_window(cx);
                        } else {
                            cx.activate(true);
                        }
                    });
                }
                "quit" => {
                    let _ = cx.update(|cx| cx.quit());
                }
                _ => {}
            }
        }
        while let Ok(ev) = rx.try_recv() {
            match ev {
                UiEvent::Overlay { text, color } => {
                    let _ = cx.update(|cx| {
                        overlay = show_overlay(cx, overlay.take(), text, color);
                    });
                }
                UiEvent::OverlayHide => {
                    let _ = cx.update(|cx| hide_overlay(cx, &mut overlay));
                }
                UiEvent::StateChanged => {
                    let _ = cx.update(|cx| notify_main(cx));
                }
                UiEvent::Status { text, error } => {
                    let _ = cx.update(|cx| {
                        main_view(cx, |v, cx| {
                            v.set_status(text.clone(), error);
                            cx.notify();
                        });
                    });
                }
                UiEvent::DownloadProgress { id, done, total } => {
                    let _ = cx.update(|cx| {
                        main_view(cx, |v, cx| {
                            v.set_progress(id.clone(), done, total);
                            cx.notify();
                        });
                    });
                }
            }
        }
        cx.background_executor().timer(Duration::from_millis(30)).await;
    }
}

fn selftest() {
    let id = "whisper-tiny";
    println!("selftest: loading {}", id);
    // 3 seconds of quiet noise — exercises load + full inference path.
    let mut samples = Vec::with_capacity(48000);
    for i in 0..48000u32 {
        let t = i as f32 / 16000.0;
        samples.push((t * 220.0 * 6.28).sin() * 0.02);
    }
    let res = state()
        .engine
        .lock()
        .unwrap()
        .transcribe(id, &samples);
    match res {
        Ok(text) => println!("selftest OK: {:?}", text),
        Err(e) => {
            println!("selftest FAILED: {}", e);
            std::process::exit(1);
        }
    }
}
