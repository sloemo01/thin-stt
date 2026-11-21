use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::{config, models, state};

pub fn fetch_remote_sizes() {
    std::thread::spawn(|| {
        for m in models::MODELS {
            let wanted: Vec<&str> = m.files.iter().map(|(r, _)| *r).collect();
            if let Some(bytes) = models::remote_size_bytes(m.repo, &wanted) {
                let s = state::state();
                s.cfg
                    .lock()
                    .unwrap()
                    .remote_sizes
                    .insert(m.id.to_string(), bytes);
            }
        }
        let _ = config::save(&state::state().cfg.lock().unwrap());
        state::emit(state::UiEvent::StateChanged);
    });
}

pub fn start_download(model_id: &str) {
    let s = state::state();
    let info = match models::by_id(model_id) {
        Some(i) => i,
        None => return,
    };
    if models::is_downloaded(info) || s.downloads.lock().unwrap().contains_key(model_id) {
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    s.downloads
        .lock()
        .unwrap()
        .insert(model_id.to_string(), cancel.clone());
    let model_id = model_id.to_string();

    std::thread::spawn(move || {
        let id = model_id;
        let result = download_all(info, &cancel);
        state::state().downloads.lock().unwrap().remove(&id);
        match result {
            Ok(()) => {
                state::status(format!("{} ready", info.label));
            }
            Err(e) => {
                state::status_err(format!("{} download failed: {}", info.label, e));
            }
        }
        state::emit(state::UiEvent::StateChanged);
    });
}

fn download_all(info: &'static models::ModelInfo, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let dir = models::model_dir(info.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // total bytes across all files (HEAD each)
    let mut total: u64 = 0;
    for (remote, _) in info.files {
        let url = models::download_url(info, remote);
        if let Ok(resp) = ureq::head(&url).call() {
            if let Some(len) = resp
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
            {
                total += len;
            }
        }
    }

    let mut done: u64 = 0;
    let mut last_emit = Instant::now();
    for (remote, local) in info.files {
        if cancel.load(Ordering::SeqCst) {
            cleanup(&dir);
            return Err("cancelled".into());
        }
        let url = models::download_url(info, remote);
        let dest = dir.join(local);
        let part = dir.join(format!("{}.part", local));
        let mut resp = ureq::get(&url).call().map_err(|e| e.to_string())?;
        let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut reader = resp.body_mut().as_reader();
        let mut buf = [0u8; 256 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(e.to_string());
                }
            };
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&part);
                cleanup(&dir);
                return Err("cancelled".into());
            }
            if last_emit.elapsed().as_millis() > 80 {
                last_emit = Instant::now();
                state::emit(state::UiEvent::DownloadProgress {
                    id: info.id.to_string(),
                    done,
                    total,
                });
            }
        }
        drop(file);
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    }
    state::emit(state::UiEvent::DownloadProgress {
        id: info.id.to_string(),
        done: total,
        total,
    });
    Ok(())
}

fn cleanup(dir: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().ends_with(".part") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

pub fn delete_model(model_id: &str) {
    let s = state::state();
    state::cancel_download(model_id);
    s.engine.lock().unwrap().drop_model(model_id);
    let dir = models::model_dir(model_id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    // also clear any hotkey
    {
        let mut cfg = s.cfg.lock().unwrap();
        if cfg.hotkeys.remove(model_id).is_some() {
            let _ = config::save(&cfg);
        }
    }
    state::reregister_hotkeys();
    state::emit(state::UiEvent::StateChanged);
    state::status("model deleted");
}

pub fn set_hotkey(model_id: &str, spec: &str) {
    let s = state::state();
    {
        let mut cfg = s.cfg.lock().unwrap();
        cfg.hotkeys.insert(model_id.to_string(), spec.to_string());
        let _ = config::save(&cfg);
    }
    state::reregister_hotkeys();
    state::emit(state::UiEvent::StateChanged);
}

pub fn clear_hotkey(model_id: &str) {
    let s = state::state();
    {
        let mut cfg = s.cfg.lock().unwrap();
        cfg.hotkeys.remove(model_id);
        let _ = config::save(&cfg);
    }
    state::reregister_hotkeys();
    state::emit(state::UiEvent::StateChanged);
}

pub fn set_mic(name: Option<String>) {
    let s = state::state();
    {
        let mut cfg = s.cfg.lock().unwrap();
        cfg.mic_device = name;
        let _ = config::save(&cfg);
    }
    state::emit(state::UiEvent::StateChanged);
}

pub fn set_paste_mode(mode: &str) {
    let s = state::state();
    {
        let mut cfg = s.cfg.lock().unwrap();
        cfg.paste_mode = Some(mode.to_string());
        let _ = config::save(&cfg);
    }
    state::emit(state::UiEvent::StateChanged);
}

/// Spawn a detached cleaner that removes app data (and the .app bundle) once
/// this process exits. Caller should quit the app right after.
pub fn uninstall() {
    let pid = std::process::id();
    let data = config::app_data_dir();
    let exe = std::env::current_exe().unwrap_or_default();
    let app_bundle = exe
        .ancestors()
        .find(|p| p.extension().map(|e| e == "app").unwrap_or(false))
        .map(|p| p.to_path_buf());

    let mut script = format!(
        "while kill -0 {} 2>/dev/null; do sleep 1; done; rm -rf '{}'",
        pid,
        data.display()
    );
    if let Some(app) = app_bundle {
        script.push_str(&format!(" '{}'", app.display()));
    }
    let _ = std::process::Command::new("sh").arg("-c").arg(script).spawn();
}
