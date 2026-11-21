"""Global hotkeys via the `keyboard` package (works from any app).

Hold-to-talk: the press handler fires when the combo goes down, the release
handler when it comes up — record while held, transcribe on release.
"""

import keyboard


def qt_seq_to_hotkey(seq: str) -> str:
    """'Ctrl+Alt+Q' -> 'ctrl+alt+q' (keyboard package format)."""
    parts = [p.strip().lower() for p in seq.split("+") if p.strip()]
    out = []
    for p in parts:
        out.append({
            "control": "ctrl", "meta": "win", "return": "enter",
            "escape": "esc", " ": "space",
        }.get(p, p))
    return "+".join(out)


class HotkeyManager:
    """model_id -> hotkey, with separate press and release callbacks."""

    def __init__(self, on_press, on_release):
        self._on_press = on_press
        self._on_release = on_release
        self._handles: dict[str, tuple] = {}

    def set(self, model_id: str, hotkey: str) -> None:
        self.clear(model_id)
        if not hotkey:
            return
        press = keyboard.add_hotkey(
            hotkey, lambda mid=model_id: self._on_press(mid))
        release = keyboard.add_hotkey(
            hotkey, lambda mid=model_id: self._on_release(mid),
            trigger_on_release=True)
        self._handles[model_id] = (press, release)

    def clear(self, model_id: str) -> None:
        for h in self._handles.pop(model_id, ()):
            try:
                keyboard.remove_hotkey(h)
            except Exception:
                pass

    def clear_all(self) -> None:
        for mid in list(self._handles):
            self.clear(mid)
