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
- Optional **LLM polish**: a ~380 MB local Qwen3 model (GGUF via llama.cpp) cleans each transcript — punctuation, casing, filler removal — loading only while polishing and dropping right after
- Settings: microphone choice, insert mode, LLM polish toggle, model cache location, **"Completely delete this app"** (removes models, settings, and the app bundle)
- Minimizes to the system tray; hotkeys stay active
- Floating recording/transcribing overlay

## Install

### macOS

**Download the prebuilt app (recommended):**

```
curl -fsSL -o Thin-STT.dmg https://github.com/sloemo01/thin-stt/releases/latest/download/Thin-STT.dmg
hdiutil attach Thin-STT.dmg
```

Mount, drag `Thin STT.app` to Applications. Downloading through curl sets no quarantine flag, so Gatekeeper does not prompt (browser/AirDrop downloads would). Apple Silicon only; signed with the maintainer's local `Thin STT Local Signing` certificate.

**Build from source** — paste into Terminal:

```
curl -fsSL https://raw.githubusercontent.com/sloemo01/thin-stt/main/install.sh | sh
```

Downloads the latest signed release, mounts the DMG, and installs `Thin STT.app` to `/Applications` — no Xcode or Rust needed. If you'd rather build from source: `sh install.sh --source` (requires full Xcode — gpui compiles Metal shaders; rustup is installed automatically if cargo is missing). Source builds are signed with a **local self-signed certificate**, giving the app a stable signing identity so macOS remembers your Microphone/Accessibility grants across rebuilds.

On first launch macOS will ask for **Microphone** access, and for **Accessibility** access (needed to type text into other apps) — the Settings tab has a button to grant it.

**Manual:**

```
cargo build --release
./target/release/thin-stt
```

**Share the app:** run `sh dist.sh` — it builds, signs `Thin STT.app` with your local `Thin STT Local Signing` certificate, and writes `dist/Thin-STT.dmg` (`--zip` for a zip). If the recipient fetches it with curl the app is not quarantined and opens directly; over browser/AirDrop they'll need right-click → Open once (self-signed apps show "unidentified developer" there).

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
