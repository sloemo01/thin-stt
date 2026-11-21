#!/bin/sh
# Thin STT for macOS — curl installer. Run via:
#   curl -fsSL https://raw.githubusercontent.com/sloemo01/thin-stt/master/install.sh | sh
# Builds from source, assembles "Thin STT.app", ad-hoc signs it, installs to ~/Applications.
set -eu

APP_NAME="Thin STT"
APP_DIR="$HOME/Applications"
APP="$APP_DIR/$APP_NAME.app"
REPO="https://github.com/sloemo01/thin-stt"
BRANCH="master"

echo "== Thin STT for macOS =="

# Xcode Command Line Tools (clang, git, iconutil are needed for the build)
if ! xcode-select -p >/dev/null 2>&1; then
    echo "Xcode Command Line Tools are required. Run this, then re-run the installer:"
    echo "    xcode-select --install"
    exit 1
fi

# Rust toolchain
if ! command -v cargo >/dev/null 2>&1; then
    echo "Installing Rust..."
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --quiet
    # shellcheck disable=SC1090
    . "$HOME/.cargo/env"
fi

WORK="$(mktemp -d /tmp/thinstt.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

echo "Downloading source..."
mkdir -p "$WORK/src"
if command -v git >/dev/null 2>&1; then
    git clone --depth 1 --branch "$BRANCH" "$REPO" "$WORK/src" >/dev/null 2>&1 || true
fi
if [ ! -f "$WORK/src/Cargo.toml" ]; then
    curl -fsSL "$REPO/archive/refs/heads/$BRANCH.tar.gz" | tar -xz -C "$WORK/src" --strip-components=1
fi

cd "$WORK/src"
echo "Building (first run compiles ~600 crates — a few minutes)..."
cargo build --release --quiet

# -- icon.icns
ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z "$s" "$s" assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
done
cp assets/icon.png "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$WORK/AppIcon.icns"

# -- .app bundle
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/thin-stt "$APP/Contents/MacOS/thin-stt"
cp "$WORK/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>thin-stt</string>
    <key>CFBundleIdentifier</key><string>app.thinstt</string>
    <key>CFBundleName</key><string>Thin STT</string>
    <key>CFBundleDisplayName</key><string>Thin STT</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>Thin STT records your microphone while you hold the transcription hotkey.</string>
</dict>
</plist>
PLIST

# Ad-hoc sign (no Developer ID needed) so macOS keeps TCC grants stable
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true

echo "Installed: $APP"
echo "On first launch macOS will ask for Microphone + Accessibility permission."
open "$APP"
