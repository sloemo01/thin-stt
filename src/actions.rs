use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    // Show the progress row right away, before any network round trip.
    state::emit(state::UiEvent::DownloadProgress { id: model_id.to_string(), done: 0, total: 0 });
    let model_id = model_id.to_string();

    std::thread::spawn(move || {
        let id = model_id;
        let result = download_all(info, &cancel);
        {
            // Cancel already dropped our entry; never remove a newer retry's.
            let mut d = state::state().downloads.lock().unwrap();
            if d.get(&id).map_or(false, |c| Arc::ptr_eq(c, &cancel)) {
                d.remove(&id);
            }
        }
        match result {
            Ok(()) => state::status(format!("{} ready", info.label)),
            Err(_) if cancel.load(Ordering::SeqCst) => {}
            Err(e) => state::status_err(format!("{} download failed: {}", info.label, e)),
        }
        state::emit(state::UiEvent::StateChanged);
    });
}

/// Downloads must fail fast on a dead connection instead of hanging at
/// "Starting…": connect, headers and each socket read are all bounded.
fn download_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .build()
        .new_agent()
}

fn download_all(info: &'static models::ModelInfo, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let dir = models::model_dir(info.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Sizes are only for the progress bar: look them up beside the
    // download instead of making the first byte wait on them.
    let total = Arc::new(AtomicU64::new(0));
    {
        let total = total.clone();
        std::thread::spawn(move || {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(15)))
                .build()
                .new_agent();
            std::thread::scope(|sc| {
                for (remote, _) in info.files {
                    let (agent, total) = (&agent, &total);
                    sc.spawn(move || {
                        let len = agent
                            .head(&models::download_url(info, remote))
                            .call()
                            .ok()
                            .and_then(|r| r.headers().get("content-length")?.to_str().ok()?.parse::<u64>().ok());
                        if let Some(n) = len {
                            total.fetch_add(n, Ordering::Relaxed);
                        }
                    });
                }
            });
        });
    }
    let agent = download_agent();
    let nonce = std::process::id() as u128 * 1_000_000_000
        + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());

    let mut done: u64 = 0;
    let mut last_emit = Instant::now();
    for (remote, local) in info.files {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let url = models::download_url(info, remote);
        let dest = dir.join(local);
        if let Ok(m) = std::fs::metadata(&dest) {
            done += m.len();
            continue;
        }
        let part = dir.join(format!("{}.{:x}.part", local, nonce));
        let mut resp = agent.get(&url).call().map_err(|e| e.to_string())?;
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
                return Err("cancelled".into());
            }
            if last_emit.elapsed().as_millis() > 80 {
                last_emit = Instant::now();
                state::emit(state::UiEvent::DownloadProgress {
                    id: info.id.to_string(),
                    done,
                    total: total.load(Ordering::Relaxed),
                });
            }
        }
        drop(file);
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    }
    state::emit(state::UiEvent::DownloadProgress { id: info.id.to_string(), done, total: done });
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

/// Check GitHub for a newer release; report via status line and, when an
/// update exists, a macOS notification.
/// This build's stamp — dist.sh writes the unix build time into
/// src/buildstamp.txt before compiling and ships it as a `build.txt`
/// release asset, so the check compares builds, not just the semver.
fn build_stamp() -> u64 {
    include_str!("buildstamp.txt").trim().parse().unwrap_or(0)
}

pub fn check_for_updates() {
    state::status("checking for updates…");
    std::thread::spawn(|| {
        // The release carries a build.txt companion asset holding the
        // unix timestamp dist.sh stamped the build with. Same stamp =
        // same build; newer = update. No API call, no rate limit.
        let build_url =
            "https://github.com/sloemo01/thin-stt/releases/latest/download/build.txt";
        let remote_stamp = ureq::get(build_url)
            .call()
            .ok()
            .and_then(|mut r| r.body_mut().read_to_string().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        if let Some(remote) = remote_stamp {
            let local = build_stamp();
            if remote > local {
                let msg = format!(
                    "Update available (build {}) — you have build {}",
                    remote, local
                );
                *state::state().update_available.lock().unwrap() = Some(msg.clone());
                notify_macos("Thin STT", &msg);
                state::status(msg);
                state::emit(state::UiEvent::StateChanged);
            } else {
                state::status(format!("up to date (build {})", local));
            }
            return;
        }
        // Older releases have no build.txt — fall back to comparing the
        // tag version pulled from the DMG redirect Location.
        let url = "https://github.com/sloemo01/thin-stt/releases/latest/download/Thin-STT.dmg";
        let agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .new_agent();
        let tag = agent
            .head(url)
            .call()
            .ok()
            .and_then(|r| {
                let loc = r.headers().get("location")?.to_str().ok()?;
                // .../releases/download/<tag>/Thin-STT.dmg
                let mut parts = loc.split('/');
                let pos = parts.clone().position(|p| p == "download")?;
                parts.nth(pos + 1).map(String::from)
            });
        match tag {
            Some(tag) => {
                let latest = semver_of(&tag);
                let current = semver_of(env!("CARGO_PKG_VERSION"));
                if latest > current {
                    let msg = format!("Update available: {} (you have v{})", tag, env!("CARGO_PKG_VERSION"));
                    *state::state().update_available.lock().unwrap() = Some(msg.clone());
                    notify_macos("Thin STT", &msg);
                    state::status(msg);
                    state::emit(state::UiEvent::StateChanged);
                } else {
                    state::status(format!("up to date (v{})", env!("CARGO_PKG_VERSION")));
                }
            }
            None => state::status_err("update check failed"),
        }
    });
}

/// Download the latest release DMG, replace /Applications/Thin STT.app
/// and relaunch. Runs the same flow install.sh does, in-app.
pub fn apply_update() {
    state::status("downloading update…");
    std::thread::spawn(|| match apply_update_inner() {
        Ok(()) => {
            state::status("updated — relaunching");
            std::thread::sleep(std::time::Duration::from_millis(600));
            let _ = std::process::Command::new("open")
                .arg("-n")
                .arg("/Applications/Thin STT.app")
                .spawn();
            state::emit(state::UiEvent::Quit);
        }
        Err(e) => state::status_err(format!("update failed: {}", e)),
    });
}

fn apply_update_inner() -> Result<(), String> {
    let dmg_url =
        "https://github.com/sloemo01/thin-stt/releases/latest/download/Thin-STT.dmg";
    let dmg = std::env::temp_dir().join("thin-stt-update.dmg");
    {
        let mut resp = ureq::get(dmg_url).call().map_err(|e| e.to_string())?;
        let mut file = std::fs::File::create(&dmg).map_err(|e| e.to_string())?;
        std::io::copy(&mut resp.body_mut().as_reader(), &mut file)
            .map_err(|e| e.to_string())?;
    }
    let cleanup = |dmg: &std::path::Path| {
        let _ = std::fs::remove_file(dmg);
    };

    // Mount the image; transient I/O flakes mean retrying a fresh mount.
    let mut mount: Option<String> = None;
    for _ in 0..5 {
        let out = std::process::Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-readonly", dmg.to_str().unwrap()])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        mount = stdout
            .lines()
            .rev()
            .find_map(|l| l.find("/Volumes/").map(|i| l[i..].trim().to_string()));
        if mount.is_some() {
            break;
        }
    }
    let mount = match mount {
        Some(m) => m,
        None => {
            cleanup(&dmg);
            return Err("could not mount update image".into());
        }
    };
    let src = format!("{}/Thin STT.app", mount);
    let dst = "/Applications/Thin STT.app";

    // Move the running app aside, copy the new one in, then relaunch.
    let stale = format!("{}.old-{}", dst, std::process::id());
    let _ = std::fs::rename(dst, &stale);
    let copy = std::process::Command::new("ditto")
        .args([&src, dst])
        .status()
        .map_err(|e| e.to_string());
    let _ = std::process::Command::new("hdiutil")
        .args(["detach", "-quiet", &mount])
        .status();
    cleanup(&dmg);
    match copy {
        Ok(st) if st.success() => {
            let _ = std::fs::remove_dir_all(&stale);
            Ok(())
        }
        _ => {
            // roll back
            let _ = std::fs::remove_dir_all(dst);
            let _ = std::fs::rename(&stale, dst);
            Err("copy failed".into())
        }
    }
}

/// Strip tag decorations like "v0.1.0-macos" down to (major, minor, patch).
fn semver_of(s: &str) -> (u64, u64, u64) {
    let core = s.trim_start_matches('v').split('-').next().unwrap_or("");
    let mut it = core.split('.');
    (
        it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
        it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
        it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
    )
}

fn notify_macos(title: &str, body: &str) {
    let script = format!(
        "display notification {:?} with title {:?}",
        body.replace('"', "\\\""),
        title.replace('"', "\\\"")
    );
    // output() waits for osascript to exit — spawn() would leave a zombie.
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output();
}

/// Clear the cache only: partial downloads, stray files, and model dirs
/// that never finished. Fully-downloaded models are kept.
pub fn clear_model_cache() {
    let s = state::state();
    let dir = config::models_dir();
    let mut freed: u64 = 0;
    let mut removed_any = false;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if path.is_dir() && models::by_id(&name).map_or(false, models::is_downloaded) {
                // complete model — purge stray .part files only, keep the model
                if let Ok(inner) = std::fs::read_dir(&path) {
                    for ie in inner.flatten() {
                        if ie.file_name().to_string_lossy().ends_with(".part") {
                            if let Ok(md) = ie.metadata() {
                                freed += md.len();
                            }
                            removed_any = true;
                            let _ = std::fs::remove_file(ie.path());
                        }
                    }
                }
                continue;
            }
            let remove = if path.is_file() {
                name.ends_with(".part")
            } else if let Some(info) = models::by_id(&name) {
                // incomplete download — the whole dir is cache
                !models::is_downloaded(info)
            } else {
                true // stray folder not tied to a model
            };
            if !remove {
                continue;
            }
            // Don't pull a dir out from under a live download — ours is in
            // the downloads map; another process's shows recent writes.
            // A cancelled download is in neither, so it clears now.
            let in_flight = models::by_id(&name)
                .map(|i| s.downloads.lock().unwrap().contains_key(i.id))
                .unwrap_or(false);
            if in_flight || (path.is_dir() && models::by_id(&name).is_none() && is_fresh(&path)) {
                continue;
            }
            if let Some(info) = models::by_id(&name) {
                state::cancel_download(info.id);
            }
            freed += dir_size(&path);
            removed_any = true;
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    state::emit(state::UiEvent::StateChanged);
    if !removed_any {
        state::status("cache already clean");
    } else {
        state::status(format!("cache cleared — freed {:.0} MB", freed as f64 / 1048576.0));
    }
}

fn dir_size(dir: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let md = e.metadata().ok();
            match (e.file_type().ok(), md) {
                (Some(t), Some(_)) if t.is_dir() => total += dir_size(&e.path()),
                (Some(t), Some(m)) if t.is_file() => total += m.len(),
                _ => {}
            }
        }
    }
    total
}

/// Older builds persisted every dictation as rec-*.wav under Application
/// Support — audio now never touches disk. Sweep the leftovers once at
/// launch so the cache dir doesn't grow forever.
pub fn sweep_recordings() {
    let dir = config::recordings_dir();
    if dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let _ = std::fs::remove_dir(&dir);
    }
}

/// Startup sweep: delete partial downloads, orphan folders, and incomplete
/// model dirs left by crashes/interruptions. Returns bytes freed.
pub fn cleanup_partial() -> u64 {
    let dir = config::models_dir();
    if !dir.is_dir() {
        return 0;
    }
    let mut freed = 0u64;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let known = models::by_id(&name);
            let remove = if path.is_file() {
                name.ends_with(".part")
            } else if let Some(info) = known {
                // incomplete download or stray .part inside
                !models::is_downloaded(info) || has_part(&path)
            } else if models::is_polish_dir(&name) {
                // a legacy polish model dir — not in MODELS but still serving
                // as the polish fallback; only sweep an interrupted download
                has_part(&path)
            } else {
                true // unknown folder
            };
            // A second process may be mid-download into one of these dirs —
            // skip anything touched in the last 10 minutes.
            if remove && is_fresh(&path) {
                continue;
            }
            if remove {
                freed += dir_size(&path);
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    freed
}

/// True when the file (or anything inside the dir) was modified in the last
/// 10 minutes — signals another process is still writing it.
fn is_fresh(path: &std::path::Path) -> bool {
    let cutoff = std::time::Duration::from_secs(600);
    let now = std::time::SystemTime::now();
    fn newest(path: &std::path::Path) -> Option<std::time::SystemTime> {
        let mut n = std::fs::metadata(path).ok()?.modified().ok();
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for e in entries.flatten() {
                    let en = newest(&e.path());
                    if en > n {
                        n = en;
                    }
                }
            }
        }
        n
    }
    newest(path)
        .and_then(|t| now.duration_since(t).ok())
        .map(|age| age < cutoff)
        .unwrap_or(false)
}

fn has_part(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|es| es.flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part")))
        .unwrap_or(false)
}

pub fn delete_model(model_id: &str) {
    let s = state::state();
    state::cancel_download(model_id);
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

pub fn toggle_polish() {
    let s = state::state();
    {
        let mut cfg = s.cfg.lock().unwrap();
        cfg.polish = !cfg.polish;
        let _ = config::save(&cfg);
    }
    state::emit(state::UiEvent::StateChanged);
}

pub fn set_input_source(source: &str) {
    let s = state::state();
    {
        let mut cfg = s.cfg.lock().unwrap();
        cfg.input_source = Some(source.to_string());
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
