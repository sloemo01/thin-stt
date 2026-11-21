use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

/// Insert transcribed text into the currently focused application.
///
/// mode: "type" (synthesized keystrokes, enigo unicode),
///       "paste" (clipboard + Cmd+V — fast for long text),
///       "clipboard" (leave it on the clipboard only).
pub fn insert_text(text: &str, mode: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    // Always seed the clipboard (so "paste" works and "type" has a fallback).
    {
        let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
        cb.set_text(text.to_string()).map_err(|e| e.to_string())?;
    }

    match mode {
        "clipboard" => Ok(()),
        "paste" => {
            let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
            thread::sleep(Duration::from_millis(150)); // clipboard settle
            enigo
                .key(Key::Meta, Direction::Press)
                .map_err(|e| e.to_string())?;
            enigo
                .key(Key::Unicode('v'), Direction::Click)
                .map_err(|e| e.to_string())?;
            enigo
                .key(Key::Meta, Direction::Release)
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        _ => {
            // "type" — synthesize keystrokes
            let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
            enigo.text(text).map_err(|e| e.to_string())
        }
    }
}

pub fn accessibility_trusted() -> bool {
    macos_accessibility_client::accessibility::application_is_trusted()
}

/// Prompt the user to grant Accessibility access (shows the macOS dialog).
pub fn request_accessibility() {
    macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
}
