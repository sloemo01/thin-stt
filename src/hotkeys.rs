use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// Modifier bit flags used for modifier-only hotkeys (no regular key).
pub const BIT_CTRL: u8 = 1;
pub const BIT_ALT: u8 = 2;
pub const BIT_SHIFT: u8 = 4;
pub const BIT_CMD: u8 = 8;

/// A hotkey trigger: either a global-hotkey HotKey (key + optional mods) or a
/// modifier-only combo watched via an NSEvent flagsChanged monitor.
pub enum Trigger {
    Keyed(HotKey),
    Mods(u8),
}

/// Stored form: "ctrl+alt+9", "cmd+shift+space", or modifier-only "alt",
/// "ctrl+alt". Also accepts the legacy canonical form
/// ("Control+Alt+Digit9", "CmdOrControl+Alt+KeyA").
pub fn parse(spec: &str) -> Option<Trigger> {
    let mut mods = Modifiers::empty();
    let mut bits: u8 = 0;
    let mut key: Option<Code> = None;
    for tok in spec.split('+') {
        match normalize(tok).as_str() {
            "ctrl" => {
                mods |= Modifiers::CONTROL;
                bits |= BIT_CTRL;
            }
            "alt" => {
                mods |= Modifiers::ALT;
                bits |= BIT_ALT;
            }
            "shift" => {
                mods |= Modifiers::SHIFT;
                bits |= BIT_SHIFT;
            }
            "cmd" => {
                mods |= Modifiers::SUPER;
                bits |= BIT_CMD;
            }
            k => key = code_for(k),
        }
    }
    match key {
        Some(code) => Some(Trigger::Keyed(HotKey::new(
            if mods.is_empty() { None } else { Some(mods) },
            code,
        ))),
        None if bits != 0 => Some(Trigger::Mods(bits)),
        None => None,
    }
}

/// Normalize one token to the short form: modifiers -> ctrl/alt/shift/cmd,
/// "Digit9"->"9", "KeyA"->"a", "ArrowUp"->"up".
fn normalize(tok: &str) -> String {
    let t = tok.to_lowercase();
    match t.as_str() {
        "ctrl" | "control" => return "ctrl".into(),
        "alt" | "option" => return "alt".into(),
        "shift" => return "shift".into(),
        "cmd" | "meta" | "super" | "command" | "cmdorcontrol" | "win" => return "cmd".into(),
        _ => {}
    }
    if let Some(rest) = t.strip_prefix("digit") {
        return rest.to_string();
    }
    if let Some(rest) = t.strip_prefix("key") {
        if rest.len() == 1 {
            return rest.to_string();
        }
        return t;
    }
    if let Some(rest) = t.strip_prefix("arrow") {
        return rest.to_string();
    }
    t
}

/// Build the stored spec from a gpui keystroke. Returns None for lone modifiers.
pub fn from_gpui(ks: &gpui::Keystroke) -> Option<String> {
    let key = ks.key.to_lowercase();
    if matches!(
        key.as_str(),
        "meta" | "control" | "ctrl" | "alt" | "shift" | "fn" | "capslock"
    ) {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    push_mods(&mut parts, &ks.modifiers);
    parts.push(key);
    Some(parts.join("+"))
}

/// Build a modifier-only spec ("alt", "ctrl+alt", …) from gpui modifiers.
/// Used to capture bare modifier hotkeys, which never produce key events.
pub fn from_modifiers(m: &gpui::Modifiers) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    push_mods(&mut parts, m);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("+"))
    }
}

fn push_mods(parts: &mut Vec<String>, m: &gpui::Modifiers) {
    if m.control {
        parts.push("ctrl".into());
    }
    if m.alt {
        parts.push("alt".into());
    }
    if m.shift {
        parts.push("shift".into());
    }
    if m.platform {
        parts.push("cmd".into());
    }
}

/// Map NSEvent modifierFlags to our bit set (ctrl=1, alt=2, shift=4, cmd=8).
pub fn ns_flags_to_bits(flags: u64) -> u8 {
    let mut bits = 0u8;
    if flags & (1 << 18) != 0 {
        bits |= BIT_CTRL;
    }
    if flags & (1 << 19) != 0 {
        bits |= BIT_ALT;
    }
    if flags & (1 << 17) != 0 {
        bits |= BIT_SHIFT;
    }
    if flags & (1 << 20) != 0 {
        bits |= BIT_CMD;
    }
    bits
}

/// "ctrl+alt+9" -> "⌃⌥9"; "alt" -> "⌥"; "ctrl+alt" -> "⌃⌥"
pub fn display(spec: &str) -> String {
    let mut out = String::new();
    for t in spec.split('+').map(normalize) {
        match t.as_str() {
            "ctrl" => out.push_str("⌃"),
            "alt" => out.push_str("⌥"),
            "shift" => out.push_str("⇧"),
            "cmd" => out.push_str("⌘"),
            k => {
                let s: String = match k {
                    "space" => "Space".into(),
                    "enter" => "↩".into(),
                    "tab" => "⇥".into(),
                    "escape" => "⎋".into(),
                    "backspace" => "⌫".into(),
                    "delete" => "⌦".into(),
                    "up" => "↑".into(),
                    "down" => "↓".into(),
                    "left" => "←".into(),
                    "right" => "→".into(),
                    k if k.len() == 1 => k.to_uppercase(),
                    k => k.to_string(),
                };
                out.push_str(&s);
            }
        }
    }
    out
}

fn code_for(name: &str) -> Option<Code> {
    use Code::*;
    Some(match name {
        "a" => KeyA, "b" => KeyB, "c" => KeyC, "d" => KeyD, "e" => KeyE,
        "f" => KeyF, "g" => KeyG, "h" => KeyH, "i" => KeyI, "j" => KeyJ,
        "k" => KeyK, "l" => KeyL, "m" => KeyM, "n" => KeyN, "o" => KeyO,
        "p" => KeyP, "q" => KeyQ, "r" => KeyR, "s" => KeyS, "t" => KeyT,
        "u" => KeyU, "v" => KeyV, "w" => KeyW, "x" => KeyX, "y" => KeyY,
        "z" => KeyZ,
        "0" => Digit0, "1" => Digit1, "2" => Digit2, "3" => Digit3,
        "4" => Digit4, "5" => Digit5, "6" => Digit6, "7" => Digit7,
        "8" => Digit8, "9" => Digit9,
        "space" => Space,
        "enter" | "return" => Enter,
        "tab" => Tab,
        "escape" | "esc" => Escape,
        "backspace" => Backspace,
        "delete" => Delete,
        "insert" => Insert,
        "home" => Home,
        "end" => End,
        "pageup" => PageUp,
        "pagedown" => PageDown,
        "up" | "arrowup" => ArrowUp,
        "down" | "arrowdown" => ArrowDown,
        "left" | "arrowleft" => ArrowLeft,
        "right" | "arrowright" => ArrowRight,
        "f1" => F1, "f2" => F2, "f3" => F3, "f4" => F4, "f5" => F5,
        "f6" => F6, "f7" => F7, "f8" => F8, "f9" => F9, "f10" => F10,
        "f11" => F11, "f12" => F12,
        "minus" => Minus,
        "equal" => Equal,
        "bracketleft" => BracketLeft,
        "bracketright" => BracketRight,
        "backslash" => Backslash,
        "semicolon" => Semicolon,
        "quote" => Quote,
        "backquote" => Backquote,
        "comma" => Comma,
        "period" => Period,
        "slash" => Slash,
        _ => return None,
    })
}
