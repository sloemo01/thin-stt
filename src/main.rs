#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod actions;
mod audio;
mod camera;
mod config;
mod engine;
mod hotkeys;
mod lips;
mod llm;
mod models;
mod overlay;
mod state;
mod typer;
mod ui;

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use gpui::*;
use objc::{msg_send, sel, sel_impl};
use overlay::OverlayView;
use state::{state, UiEvent};
use ui::ThinSttView;

const OVERLAY_RECORDING: u32 = 0xff5a5f;
const OVERLAY_BUSY: u32 = 0xf0b54a;
const OVERLAY_W: f32 = 300.0;
const OVERLAY_H: f32 = 52.0;

fn main() {
    tune_allocator();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Some(t) = std::env::args()
        .nth(1)
        .filter(|a| a == "--polish")
        .and_then(|_| std::env::args().nth(2))
    {
        selftest_polish(&t);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--lipfile") {
        selftest_lipfile();
        return;
    }
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
        actions::sweep_recordings();
        setup_tray();
        unsafe { dark_appearance() };
        open_main_window(cx);

        cx.spawn(|cx: &mut AsyncApp| {
            let cx = cx.clone();
            async move { pump(rx, cx).await }
        })
        .detach();

        if std::env::args().any(|a| a == "--demo") {
            // Show the overlay briefly so it can be checked visually.
            state::emit(UiEvent::Overlay {
                title: "Listening".into(),
                detail: "Whisper Tiny · release to insert".into(),
                color: OVERLAY_RECORDING,
                live: true,
            });
        }
    });
}

/// Liquid Glass (macOS 26 `NSGlassEffectView`) inserted under the GPUI view.
/// `frame` is in the content view's coordinates; no-op on older macOS.
#[allow(deprecated)]
unsafe fn add_glass(gpui_view: cocoa::base::id, frame: cocoa::foundation::NSRect, radius: f64, mask: u64) {
    use cocoa::base::{id, nil};
    let Some(cls) = objc::runtime::Class::get("NSGlassEffectView") else {
        return;
    };
    let parent: id = msg_send![gpui_view, superview];
    if parent == nil {
        return;
    }
    let glass: id = msg_send![cls, alloc];
    let glass: id = msg_send![glass, initWithFrame: frame];
    let _: () = msg_send![glass, setCornerRadius: radius];
    let _: () = msg_send![glass, setAutoresizingMask: mask];
    let tint: id = msg_send![objc::class!(NSColor), colorWithSRGBRed: 0.03f64 green: 0.03f64 blue: 0.04f64 alpha: 0.35f64];
    let _: () = msg_send![glass, setTintColor: tint];
    // NSWindowBelow = -1
    let _: () = msg_send![parent, addSubview: glass positioned: -1isize relativeTo: gpui_view];
}

#[allow(deprecated)]
/// Force dark Aqua app-wide so the window blur and glass use their dark
/// variants. Must run before any window exists: changing a live window's
/// appearance re-enters GPUI while it is borrowed.
unsafe fn dark_appearance() {
    use cocoa::foundation::NSString;
    let name = NSString::alloc(cocoa::base::nil).init_str("NSAppearanceNameDarkAqua");
    let look: cocoa::base::id = msg_send![objc::class!(NSAppearance), appearanceNamed: name];
    let app: cocoa::base::id = msg_send![objc::class!(NSApplication), sharedApplication];
    let _: () = msg_send![app, setAppearance: look];
}

fn open_main_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(920.0), px(660.0)), cx);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Thin STT".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(20.0), px(18.0))),
                }),
                focus: true,
                show: true,
                kind: WindowKind::Normal,
                is_movable: true,
                is_resizable: true,
                is_minimizable: true,
                display_id: None,
                window_background: WindowBackgroundAppearance::Blurred,
                app_id: None,
                window_min_size: Some(size(px(780.0), px(540.0))),
                window_decorations: None,
                tabbing_identifier: None,
            },
            |window, cx| cx.new(|cx| ThinSttView::new(window, cx)),
        )
        .expect("open main window");
    // Grab the NSView, then touch AppKit outside the GPUI borrow: appearance
    // and subview changes call back into GPUI synchronously.
    let view_ptr = handle
        .update(cx, |_v, window, _cx| {
            match raw_window_handle::HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                Ok(raw_window_handle::RawWindowHandle::AppKit(kit)) => kit.ns_view.as_ptr() as usize,
                _ => 0,
            }
        })
        .unwrap_or(0);
    if view_ptr != 0 {
        unsafe {
            #[allow(deprecated)]
            let view = view_ptr as cocoa::base::id;
            // Glass sidebar: the full sidebar column, flush with the window
            // edges (the window's rounded mask clips its corners).
            let parent: cocoa::base::id = msg_send![view, superview];
            let bounds: cocoa::foundation::NSRect = msg_send![parent, bounds];
            let frame = cocoa::foundation::NSRect::new(
                cocoa::foundation::NSPoint::new(0.0, 0.0),
                cocoa::foundation::NSSize::new(ui::SIDEBAR_WIDTH as f64, bounds.size.height),
            );
            // NSViewHeightSizable(16) | NSViewMaxXMargin(4)
            add_glass(view, frame, 0.0, 16 | 4);
        }
    }

    // Let the close button actually destroy the window: the app lives on in
    // the tray, and the GPU surfaces + Metal state get freed instead of kept.
    cx.activate(true);
}

fn overlay_position(cx: &App) -> Point<Pixels> {
    let w = px(OVERLAY_W);
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
    title: String,
    detail: String,
    color: u32,
    live: bool,
) -> Option<WindowHandle<OverlayView>> {
    if let Some(h) = existing {
        let ok = h
            .update(cx, |v, _w, cx| {
                v.title = title.clone().into();
                v.detail = detail.clone().into();
                v.color = color;
                v.live = live;
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
                size: size(px(OVERLAY_W), px(OVERLAY_H)),
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
        |_window, cx| cx.new(|_| OverlayView::new(title.clone(), detail.clone(), color, live)),
    )
    .ok();
    if let Some(h) = opened {
        let vp = h.update(cx, |_v, window, _cx| {
            match raw_window_handle::HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                Ok(raw_window_handle::RawWindowHandle::AppKit(kit)) => kit.ns_view.as_ptr() as usize,
                _ => 0,
            }
        });
        if let Some(vp) = vp.ok().filter(|v| *v != 0) {
            unsafe {
                #[allow(deprecated)]
                let view = vp as cocoa::base::id;
                let frame = cocoa::foundation::NSRect::new(
                    cocoa::foundation::NSPoint::new(0.0, 0.0),
                    cocoa::foundation::NSSize::new(OVERLAY_W as f64, OVERLAY_H as f64),
                );
                add_glass(view, frame, OVERLAY_H as f64 / 2.0, 2 | 16);
            }
        }
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

fn hotkey_pressed(id: u32, recorder: &mut Option<audio::Recorder>) {
    let s = state();
    if recorder.is_some() {
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
    if info.engine == "llm" {
        return; // polish model — not a transcription engine
    }
    if s.cfg.lock().unwrap().lips() {
        lips_pressed();
        return;
    }
    if !models::is_downloaded(info) {
        state::status_err(format!("{} is not downloaded", info.label));
        return;
    }
    let mic = s.cfg.lock().unwrap().mic_device.clone();
    match audio::Recorder::start(mic.as_deref()) {
        Ok(r) => {
            *recorder = Some(r);
            state::emit(UiEvent::Overlay {
                title: "Listening".into(),
                detail: format!("{} · release to insert", info.label),
                color: OVERLAY_RECORDING,
                live: true,
            });
            state::status(format!("Hold hotkey… release to transcribe ({})", info.label));
        }
        Err(e) => state::status_err(format!("mic: {}", e)),
    }
}

fn hotkey_released(id: u32, recorder: &mut Option<audio::Recorder>) {
    let s = state();
    let model_id = match s.hotkeys.lock().unwrap().get(&id) {
        Some(m) => m.clone(),
        None => return,
    };
    if lips_active() {
        let mode = s.cfg.lock().unwrap().paste_mode.clone().unwrap_or_else(|| "type".into());
        lips_released(mode);
        return;
    }
    let rec = match recorder.take() {
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
        title: "Transcribing".into(),
        detail: models::by_id(&model_id).map(|m| m.label).unwrap_or("").into(),
        color: OVERLAY_BUSY,
        live: false,
    });

    // stop() pauses the CoreAudio stream — the Recorder is !Send, so it
    // stays on the pump thread; only the samples cross to the worker.
    let samples = rec.stop();
    if samples.len() < 16000 / 5 {
        state::emit(UiEvent::OverlayHide);
        state::status_err("recording too short");
        return;
    }
    std::thread::spawn(move || {
        // Silence/fan-noise gate: below ~-40 dBFS RMS there is no speech to
        // keep, so skip inference instead of inviting a hallucination.
        let rms = (samples.iter().map(|x| x * x).sum::<f32>()
            / samples.len().max(1) as f32)
            .sqrt();
        if rms < 0.01 {
            state::emit(UiEvent::OverlayHide);
            state::status("no speech detected");
            return;
        }
        let res = state().engine.lock().unwrap().transcribe(&model_id, &samples);
        match res {
            Ok(text) if !text.is_empty() => {
                // Optional local-LLM polish: the model loads for this pass
                // and drops before we type, matching the STT engines.
                let text = maybe_polish(text);
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
        trim_memory();
        state::emit(UiEvent::StateChanged);
    });
}

static LIP_CAM: std::sync::Mutex<Option<camera::Camera>> = std::sync::Mutex::new(None);

fn lips_pressed() {
    let Some(model) = lips::best_model() else {
        state::status_err("Lips mode: download a lip model on the Models page");
        return;
    };
    if camera::permission() != 3 {
        camera::request_permission(|_| state::emit(UiEvent::StateChanged));
        state::status_err("Lips mode needs camera access");
        return;
    }
    {
        let mut live = camera::LIVE.lock().unwrap();
        let l = live.get_or_insert_with(Default::default);
        l.front = Some(lips::Frontend::default());
        l.started = None;
        l.frames = 0;
    }
    let mut cam = LIP_CAM.lock().unwrap();
    if cam.is_none() {
        match camera::Camera::start() {
            Ok(c) => *cam = Some(c),
            Err(e) => {
                camera::LIVE.lock().unwrap().take();
                state::status_err(format!("camera: {}", e));
                return;
            }
        }
    }
    state::emit(UiEvent::Overlay {
        title: "Reading lips".into(),
        detail: format!("{} · no audio · release to insert", model.label),
        color: OVERLAY_RECORDING,
        live: true,
    });
    state::status("Mouth your words… release to insert");
}

fn lips_active() -> bool {
    camera::LIVE.lock().unwrap().as_ref().map_or(false, |l| l.front.is_some())
}

/// Stop capture, decode the mouth crops on a worker, insert without polish.
fn lips_released(mode: String) {
    let (front, previewing) = {
        let mut live = camera::LIVE.lock().unwrap();
        let front = live.as_mut().and_then(|l| l.front.take());
        let previewing = live.as_ref().map_or(false, |l| l.previewing);
        if !previewing {
            *live = None;
        }
        (front, previewing)
    };
    if !previewing {
        // Never drop the session while holding LIVE: stopRunning waits
        // for the capture queue, whose callback takes that lock.
        let cam = LIP_CAM.lock().unwrap().take();
        drop(cam);
    }
    let (Some(front), Some(model)) = (front, lips::best_model()) else { return };
    state::emit(UiEvent::Overlay {
        title: "Decoding lips".into(),
        detail: model.label.into(),
        color: OVERLAY_BUSY,
        live: false,
    });
    std::thread::spawn(move || {
        let faces = front.faces;
        let crops = front.finish();
        let frames = crops.len() / (lips::CROP * lips::CROP);
        let res = if faces == 0 {
            Err("no face in view".to_string())
        } else if frames < lips::MIN_FRAMES {
            Err("clip too short".to_string())
        } else {
            lips::transcribe(model.id, &crops)
        };
        drop(crops);
        state::emit(UiEvent::OverlayHide);
        match res {
            Ok(text) if !text.is_empty() => {
                let shown: String = text.chars().take(40).collect();
                match typer::insert_text(&text, &mode) {
                    Ok(()) => state::status(format!("Inserted \"{}\"", shown)),
                    Err(e) => state::status_err(format!("typed to clipboard (insert failed: {})", e)),
                }
            }
            Ok(_) => state::status_err("couldn't read any words"),
            Err(e) => state::status_err(format!("lips: {}", e)),
        }
        trim_memory();
        state::emit(UiEvent::StateChanged);
    });
}

// mimalloc v3 option ids (c_src mimalloc.h enum order). Set once at startup
// so freed memory purges to the OS at once rather than lingering in caches.
const MI_OPTION_PURGE_DELAY: i32 = 15; // ms of delay before purge; 0 = immediate
const MI_OPTION_ARENA_PURGE_MULT: i32 = 24; // arena purge delay = N * purge_delay
const MI_OPTION_GENERIC_COLLECT: i32 = 34; // collect heaps every N generic allocs
const MI_OPTION_PAGE_FULL_RETAIN: i32 = 36; // full small pages kept per class
const MI_OPTION_PAGE_COMMIT_ON_DEMAND: i32 = 40; // commit pages only when touched

fn tune_allocator() {
    unsafe {
        libmimalloc_sys::mi_option_set(MI_OPTION_PURGE_DELAY, 0);
        libmimalloc_sys::mi_option_set(MI_OPTION_ARENA_PURGE_MULT, 1);
        libmimalloc_sys::mi_option_set(MI_OPTION_GENERIC_COLLECT, 1000);
        libmimalloc_sys::mi_option_set(MI_OPTION_PAGE_FULL_RETAIN, 0);
        libmimalloc_sys::mi_option_set(MI_OPTION_PAGE_COMMIT_ON_DEMAND, 1);
    }
}

/// Return freed inference buffers to the OS. Rust's allocator keeps
/// freed mallocs cached, so RSS stays high after a dictation without this.
fn trim_memory() {
    unsafe {
        // mimalloc's own purge — covers every Rust-side allocation
        libmimalloc_sys::mi_collect(true);
        extern "C" {
            fn malloc_zone_pressure_relief(
                zone: *mut std::ffi::c_void,
                goal: usize,
            ) -> usize;
        }
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}

fn maybe_polish(text: String) -> String {
    let s = state();
    if !s.cfg.lock().unwrap().polish {
        return text;
    }
    // A legacy polish model that's still on disk keeps working after
    // an update swaps the default — better than silently unpolished text.
    let Some(path) = models::polish_model_path() else {
        return text;
    };
    state::emit(UiEvent::Overlay {
        title: "Polishing".into(),
        detail: "Qwen3 cleanup pass".into(),
        color: OVERLAY_BUSY,
        live: false,
    });
    match llm::polish(&path, &text) {
        Ok(cleaned) if !cleaned.is_empty() => cleaned,
        _ => text, // polish failure must never eat the transcript
    }
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
        .with_icon_as_template(true)
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
    // The active mic capture — !Send, owned by the pump on the main thread.
    let mut recorder: Option<audio::Recorder> = None;
    let mut last_trim = Instant::now();
    loop {
        while let Ok(ev) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
            match ev.state {
                global_hotkey::HotKeyState::Pressed => hotkey_pressed(ev.id, &mut recorder),
                global_hotkey::HotKeyState::Released => hotkey_released(ev.id, &mut recorder),
            }
        }
        while let Some((pid, pressed)) =
            state().mod_events.lock().unwrap().pop_front()
        {
            if pressed {
                hotkey_pressed(pid, &mut recorder);
            } else {
                hotkey_released(pid, &mut recorder);
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
                UiEvent::Overlay { title, detail, color, live } => {
                    let _ = cx.update(|cx| {
                        overlay = show_overlay(cx, overlay.take(), title, detail, color, live);
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
                UiEvent::Quit => {
                    let _ = cx.update(|cx| cx.quit());
                }
            }
        }
        // Live mic meter for the overlay while the hotkey is held.
        if let (Some(rec), Some(h)) = (recorder.as_ref(), overlay.as_ref()) {
            let level = rec.level();
            let _ = cx.update(|cx| {
                let _ = h.update(cx, |v, _w, cx| {
                    if v.live {
                        v.push_level(level);
                        cx.notify();
                    }
                });
            });
        }
        // Idle memory sweep: while nothing is recording, hand cached
        // allocator pages back to the OS roughly every 15s.
        if recorder.is_none()
            && last_trim.elapsed() >= Duration::from_secs(15)
        {
            trim_memory();
            last_trim = Instant::now();
        }
        cx.background_executor().timer(Duration::from_millis(30)).await;
    }
}

fn selftest_polish(text: &str) {
    let Some(path) = models::polish_model_path() else {
        println!("polishtest: model not downloaded");
        std::process::exit(1);
    };
    match llm::polish(&path, text) {
        Ok(c) => println!("polishtest OK: {:?}", c),
        Err(e) => {
            println!("polishtest FAILED: {}", e);
            std::process::exit(1);
        }
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

/// `--lipfile <gray.raw> <w> <h> [crops.out]`: run 25 fps raw grayscale
/// frames through Vision + the mouth-crop frontend, then the lip model.
fn selftest_lipfile() {
    let a: Vec<String> = std::env::args().collect();
    let (w, h): (usize, usize) = (a[3].parse().unwrap(), a[4].parse().unwrap());
    let raw = std::fs::read(&a[2]).expect("read frames");
    let mut front = lips::Frontend::default();
    for (i, f) in raw.chunks_exact(w * h).enumerate() {
        let t = i as f64 * 0.04;
        if !front.wants(t) {
            continue;
        }
        let m = camera::detect_gray(f, w, h);
        front.push(t, f.to_vec(), w, h, m.map(|x| x.0), m.map(|x| x.1).unwrap_or(0.0));
    }
    let faces = front.faces;
    let crops = front.finish();
    println!("frames {} faces {} crops {}", raw.len() / (w * h), faces, crops.len() / (lips::CROP * lips::CROP));
    if let Some(out) = a.get(5) {
        std::fs::write(out, &crops).unwrap();
    }
    for id in [lips::FAST_MODEL_ID] {
        if models::by_id(id).map_or(false, models::is_downloaded) {
            let t = Instant::now();
            println!("{}: {:?} ({:?})", id, lips::transcribe(id, &crops), t.elapsed());
        }
    }
}
