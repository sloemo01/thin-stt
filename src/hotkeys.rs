use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// Stored form: "ctrl+alt+9", "cmd+shift+space". Also accepts the legacy
/// canonical form ("Control+Alt+Digit9", "CmdOrControl+Alt+KeyA").
pub fn parse(spec: &str) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;
    for tok in spec.split('+') {
        match normalize(tok).as_str() {
            "ctrl" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "cmd" => mods |= Modifiers::SUPER,
            k => key = code_for(k),
        }
    }
    let m = if mods.is_empty() { None } else { Some(mods) };
    Some(HotKey::new(m, key?))
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
    let m = &ks.modifiers;
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
    parts.push(key);
    Some(parts.join("+"))
}

/// "ctrl+alt+9" -> "⌃⌥9" (legacy "Control+Alt+Digit9" -> same)
pub fn display(spec: &str) -> String {
    let mut out = String::new();
    let mut tokens: Vec<String> = spec.split('+').map(normalize).collect();
    let key = tokens.pop().unwrap_or_default();
    let key = key.as_str();
    for t in tokens {
        out.push_str(match t.as_str() {
            "ctrl" => "⌃",
            "alt" => "⌥",
            "shift" => "⇧",
            "cmd" => "⌘",
            _ => t.as_str(),
        });
    }
    let upper = key.to_uppercase();
    out.push_str(match key {
        "space" => "Space",
        "enter" => "↩",
        "tab" => "⇥",
        "escape" => "⎋",
        "backspace" => "⌫",
        "delete" => "⌦",
        "up" | "arrowup" => "↑",
        "down" | "arrowdown" => "↓",
        "left" | "arrowleft" => "←",
        "right" | "arrowright" => "→",
        k if k.len() == 1 => upper.as_str(),
        k => k,
    });
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
