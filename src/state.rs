use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

use crate::{config, config::Config, engine::Engine};

/// Events posted from background threads to the UI pump on the main thread.
pub enum UiEvent {
    /// Rerun render: config/disk state changed.
    StateChanged,
    /// Status line for the main window.
    Status { text: String, error: bool },
    /// Show/update the floating overlay pill.
    Overlay { title: String, detail: String, color: u32, live: bool },
    /// Hide the overlay.
    OverlayHide,
    /// Download progress for one model (bytes done/total).
    DownloadProgress { id: String, done: u64, total: u64 },
    /// Ask the app to quit (after an update is staged).
    Quit,
}

pub struct AppState {
    pub cfg: Mutex<Config>,

    pub engine: Mutex<Engine>,
    /// global-hotkey shortcut id -> model_id
    pub hotkeys: Mutex<HashMap<u32, String>>,
    /// model_id -> registered HotKey (to unregister)
    pub registered: Mutex<HashMap<String, global_hotkey::hotkey::HotKey>>,
    /// model_id -> cancel flag while a download runs
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub ui: Mutex<Option<mpsc::Sender<UiEvent>>>,
    pub hotkey_manager: Mutex<global_hotkey::GlobalHotKeyManager>,
    /// pseudo_id -> required modifier bits for modifier-only hotkeys
    pub mod_watch: Mutex<Vec<(u32, u8)>>,
    /// pseudo_id -> currently held (to emit transitions)
    pub mod_held: Mutex<HashMap<u32, bool>>,
    /// press/release events synthesized by the flagsChanged monitor
    pub mod_events: Mutex<VecDeque<(u32, bool)>>,
    /// Set when Check for updates found a newer build — the message shown
    /// next to the "Update now" button.
    pub update_available: Mutex<Option<String>>,
}

pub fn state() -> &'static Arc<AppState> {
    static S: OnceLock<Arc<AppState>> = OnceLock::new();
    S.get_or_init(|| {
        Arc::new(AppState {
            cfg: Mutex::new(config::load()),
            engine: Mutex::new(Engine::new()),
            hotkeys: Mutex::new(HashMap::new()),
            registered: Mutex::new(HashMap::new()),
            downloads: Mutex::new(HashMap::new()),
            ui: Mutex::new(None),
            hotkey_manager: Mutex::new(global_hotkey::GlobalHotKeyManager::new().unwrap()),
            mod_watch: Mutex::new(Vec::new()),
            mod_held: Mutex::new(HashMap::new()),
            mod_events: Mutex::new(VecDeque::new()),
            update_available: Mutex::new(None),
        })
    })
}

pub fn emit(ev: UiEvent) {
    let tx = state().ui.lock().unwrap().clone();
    if let Some(tx) = tx {
        let _ = tx.send(ev);
    }
}

pub fn status(text: impl Into<String>) {
    emit(UiEvent::Status {
        text: text.into(),
        error: false,
    });
}

pub fn status_err(text: impl Into<String>) {
    emit(UiEvent::Status {
        text: text.into(),
        error: true,
    });
}

/// (Re)register all global hotkeys from config.
pub fn reregister_hotkeys() {
    let s = state();
    let specs: Vec<(String, String)> = {
        let cfg = s.cfg.lock().unwrap();
        cfg.hotkeys.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };
    let mgr = s.hotkey_manager.lock().unwrap();
    let mut registered = s.registered.lock().unwrap();
    let mut ids = s.hotkeys.lock().unwrap();

    // clear all
    let stale: Vec<global_hotkey::hotkey::HotKey> = registered.values().cloned().collect();
    if !stale.is_empty() {
        let _ = mgr.unregister_all(&stale);
    }
    registered.clear();
    ids.clear();

    s.mod_watch.lock().unwrap().clear();
    s.mod_held.lock().unwrap().clear();

    let mut next_pseudo: u32 = 0xF000_0000;
    for (model_id, spec) in specs {
        match crate::hotkeys::parse(&spec) {
            Some(crate::hotkeys::Trigger::Keyed(hk)) => {
                let _ = mgr.register(hk);
                ids.insert(hk.id(), model_id.clone());
                registered.insert(model_id, hk);
            }
            Some(crate::hotkeys::Trigger::Mods(bits)) => {
                let pid = next_pseudo;
                next_pseudo += 1;
                ids.insert(pid, model_id.clone());
                s.mod_watch.lock().unwrap().push((pid, bits));
            }
            None => {}
        }
    }
}

/// Called by the NSEvent flagsChanged monitor: emit press/release transitions
/// for every watched modifier-only combo.
pub fn flags_changed(ns_flags: u64) {
    let bits = crate::hotkeys::ns_flags_to_bits(ns_flags);
    let s = state();
    let watch = s.mod_watch.lock().unwrap().clone();
    let mut held = s.mod_held.lock().unwrap();
    let mut events = s.mod_events.lock().unwrap();
    for (pid, needed) in watch {
        let now = bits & needed == needed;
        let was = held.get(&pid).copied().unwrap_or(false);
        if now != was {
            held.insert(pid, now);
            events.push_back((pid, now));
        }
    }
}

pub fn cancel_download(model_id: &str) {
    let s = state();
    if let Some(flag) = s.downloads.lock().unwrap().get(model_id) {
        flag.store(true, Ordering::SeqCst);
    }
}
