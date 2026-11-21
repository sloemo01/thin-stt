# Thin STT

A thin Windows wrapper for local speech-to-text. Press a hotkey in **any app**, speak, press again — the transcript is typed (or pasted) where your cursor is.

Models run fully local via [onnx-asr](https://github.com/istupakov/onnx-asr) (ONNX Runtime, CPU — no PyTorch, no cloud).

## Features

- **OpenAI Whisper** (ONNX): Tiny, Base, Small, Large v3 Turbo
- **NVIDIA Parakeet** (ONNX): CTC, RNN-T, TDT v2 (EN), TDT v3 (multilingual)
- Per-model **Download** / **Delete** buttons (models live in the Hugging Face cache)
- **Assignable global hotkeys** per model — click the shortcut field, press your combo
- Works from any app: hotkey → speak → hotkey → text inserted via Ctrl+V, keystrokes, or clipboard
- Settings: microphone choice, insert mode, model cache location, **"Completely delete this app"** (removes models, settings, and the app folder)
- Minimizes to the system tray; hotkeys stay active

## Install

**One-liner** (downloads, creates a venv, adds a Desktop shortcut, launches):

```
curl -L https://raw.githubusercontent.com/sloemo01/thin-stt/master/install.ps1 -o %TEMP%\thinstt.ps1 && powershell -ExecutionPolicy Bypass -File %TEMP%\thinstt.ps1
```

Needs Python 3.10–3.12 on PATH.

**Manual:**

```
pip install -r requirements.txt
python -m thin_stt
```

or double-click `run.bat`.

## Usage

1. **Models tab** — click **Download** on a model (small ones like Whisper Tiny are ~150 MB, Parakeet ~2.4 GB).
2. Click the shortcut field on that row and press a combo (e.g. `Ctrl+Alt+1`).
3. Focus any app (Notepad, browser, IDE), press the combo, speak, press it again.
4. The transcript is pasted/typed into the focused app.

Requires a working microphone (Windows audio service must be running).

## Layout

| File | Purpose |
|---|---|
| `thin_stt/models.py` | model registry + HF download/delete |
| `thin_stt/engines.py` | onnx-asr wrapper (lazy load) |
| `thin_stt/audio.py` | 16 kHz mic capture to WAV |
| `thin_stt/hotkeys.py` | global hotkeys (`keyboard`) |
| `thin_stt/typer.py` | text insertion into focused app |
| `thin_stt/ui.py` | PySide6 window, overlay, tray, uninstall |
| `thin_stt/config.py` | `%APPDATA%\ThinSTT\config.json` |

## Uninstall

Settings → **Completely delete this app**, or manually remove:

- `%APPDATA%\ThinSTT` (settings + recordings)
- `~/.cache/huggingface/hub/models--*` entries for the models
- this folder
