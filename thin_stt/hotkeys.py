"""Global hotkeys via the `keyboard` package (works from any app)."""

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
    """model_id -> hotkey. Pressing toggles record/transcribe for that model."""

    def __init__(self, on_trigger):
        self._on_trigger = on_trigger
        self._handles: dict[str, object] = {}

    def set(self, model_id: str, hotkey: str) -> None:
        self.clear(model_id)
        if not hotkey:
            return
        self._handles[model_id] = keyboard.add_hotkey(
            hotkey, lambda mid=model_id: self._on_trigger(mid))

    def clear(self, model_id: str) -> None:
        h = self._handles.pop(model_id, None)
        if h is not None:
            try:
                keyboard.remove_hotkey(h)
            except Exception:
                pass

    def clear_all(self) -> None:
        for mid in list(self._handles):
            self.clear(mid)
