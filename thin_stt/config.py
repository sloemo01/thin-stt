"""App config persisted to %APPDATA%\\ThinSTT\\config.json."""

import json
import os
from pathlib import Path

APP_NAME = "ThinSTT"


def app_data_dir() -> Path:
    base = os.environ.get("APPDATA") or str(Path.home() / "AppData" / "Roaming")
    d = Path(base) / APP_NAME
    d.mkdir(parents=True, exist_ok=True)
    return d


def recordings_dir() -> Path:
    d = app_data_dir() / "recordings"
    d.mkdir(parents=True, exist_ok=True)
    return d


DEFAULTS = {
    "hotkeys": {},        # model_id -> hotkey string e.g. "ctrl+alt+1"
    "mic_device": None,   # sounddevice device index or None for default
    "paste_mode": "type",  # "type" (keystrokes) | "paste" (ctrl+v) | "clipboard"
    "remote_sizes": {},   # model_id -> bytes, cached remote size lookups
}


def load() -> dict:
    cfg = dict(DEFAULTS)
    cfg["hotkeys"] = {}
    cfg["remote_sizes"] = {}
    path = app_data_dir() / "config.json"
    try:
        stored = json.loads(path.read_text(encoding="utf-8"))
        for k, v in stored.items():
            if k in cfg:
                cfg[k] = v
    except Exception:
        pass
    return cfg


def save(cfg: dict) -> None:
    path = app_data_dir() / "config.json"
    path.write_text(json.dumps(cfg, indent=2), encoding="utf-8")
