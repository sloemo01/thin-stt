# Thin STT

A thin wrapper for local speech-to-text. **Hold** a hotkey in **any app**, speak, release — the transcript is typed where your cursor is.

- **macOS**: native app written in Rust + [gpui](https://gpui.rs), inference via [transcribe-rs](https://github.com/cjpais/transcribe-rs) (whisper.cpp on Metal, Parakeet on ONNX Runtime).
- **Windows**: Python app (`thin_stt/`), inference via [onnx-asr](https://github.com/istupakov/onnx-asr) (ONNX Runtime, CPU — no PyTorch, no cloud).

## Features

- **OpenAI Whisper**: Tiny, Base, Small, Large v3 Turbo
- **NVIDIA Parakeet**: TDT v2 (EN), TDT v3 (multilingual) — int8 ONNX
- Per-model **Download** / **Delete** buttons with progress, sizes fetched from Hugging Face
- **Assignable global hotkeys** per model — click **Set hotkey**, press your combo
- Works from any app: hold hotkey → speak → release → text is typed out as keystrokes (⌘V paste or clipboard-only also available in Settings)
- Settings: microphone choice, insert mode, model cache location, **"Completely delete this app"** (removes models, settings, and the app bundle)
- Minimizes to the system tray; hotkeys stay active
- Floating recording/transcribing overlay

## Install

### macOS

One-liner — paste into Terminal:

```
curl -fsSL https://raw.githubusercontent.com/sloemo01/thin-stt/main/install.sh | sh
```

Downloads the source, builds it with cargo, assembles `Thin STT.app` (ad-hoc signed — no Developer ID needed), and drops it in `~/Applications`. Requires Xcode Command Line Tools (`xcode-select --install`); rustup is installed automatically if cargo is missing.

On first launch macOS will ask for **Microphone** access, and for **Accessibility** access (needed to type text into other apps) — the Settings tab has a button to grant it.

**Manual:**

```
cargo build --release
./target/release/thin-stt
```

### Windows

One-liner — paste into Command Prompt (`cmd`) or Run dialog (`Win+R`):

```
curl -L https://raw.githubusercontent.com/sloemo01/thin-stt/main/install.ps1 -o %TEMP%\thinstt.ps1 && powershell -ExecutionPolicy Bypass -File %TEMP%\thinstt.ps1
```

In PowerShell use `curl.exe` and `;` instead of `curl` and `&&`. Downloads the repo, creates a venv, adds a Desktop shortcut, and launches the app. Needs Python 3.10–3.12 on PATH.

**Manual:**

```
pip install -r requirements.txt
python -m thin_stt
```

or double-click `run.bat`.

## Usage

1. Click **Download** on a model (Whisper Tiny is ~77 MB, Parakeet ~640 MB).
2. Click **Set hotkey** on that row and press a combo (e.g. `Ctrl+Alt+9` / `⌃⌥9`).
3. Focus any app (Notes, browser, IDE), **hold** the combo, speak, release.
4. The transcript is typed into the focused app.

Requires a working microphone.

## Layout

| Path | Purpose |
|---|---|
| `src/` | macOS app (Rust + gpui) |
| `src/models.rs` | model registry + HF download/delete |
| `src/engine.rs` | transcribe-rs engine wrapper (lazy load) |
| `src/audio.rs` | cpal mic capture, resample to 16 kHz |
| `src/typer.rs` | keystroke / ⌘V / clipboard insertion |
| `thin_stt/` | Windows app (Python + PySide6) |
| `install.sh` | macOS curl installer |
| `install.ps1` | Windows curl installer |
