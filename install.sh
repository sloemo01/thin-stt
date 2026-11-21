#!/bin/sh
# Thin STT for macOS — curl installer. Run via:
#   curl -fsSL https://raw.githubusercontent.com/sloemo01/thin-stt/main/install.sh | sh
# Builds from source, assembles "Thin STT.app", ad-hoc signs it, installs to ~/Applications.
set -eu

APP_NAME="Thin STT"
APP_DIR="$HOME/Applications"
APP="$APP_DIR/$APP_NAME.app"
REPO="https://github.com/sloemo01/thin-stt"
BRANCH="main"

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

# gpui compiles Metal shaders — needs the `metal` tool, which lives in
# full Xcode (not the Command Line Tools alone). If Xcode is installed
# but xcode-select still points at the CLT, use it via DEVELOPER_DIR.
if ! xcrun -f metal >/dev/null 2>&1; then
    if [ -d /Applications/Xcode.app/Contents/Developer ]; then
        export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
    else
        echo "Full Xcode is required to build (gpui compiles Metal shaders)."
        echo "Install Xcode from the App Store, then re-run the installer."
        exit 1
    fi
fi

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

# Sign with a local self-signed certificate (created once, stored in your
# login keychain). Unlike ad-hoc signing this gives the app a stable
# identity, so macOS keeps Microphone/Accessibility grants across updates.
CERT_NAME="Thin STT Local Signing"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
if ! security find-certificate -c "$CERT_NAME" "$KEYCHAIN" >/dev/null 2>&1; then
    echo "Creating local signing certificate (one-time)..."
    if openssl req -x509 -newkey rsa:2048 -nodes \
        -keyout "$WORK/thinstt-key.pem" -out "$WORK/thinstt-cert.pem" \
        -days 3650 -subj "/CN=$CERT_NAME/O=Thin STT" \
        -addext "keyUsage=critical,digitalSignature" \
        -addext "extendedKeyUsage=critical,codeSigning" >/dev/null 2>&1 \
    && { openssl pkcs12 -export -legacy -out "$WORK/thinstt.p12" \
        -inkey "$WORK/thinstt-key.pem" -in "$WORK/thinstt-cert.pem" \
        -passout pass: >/dev/null 2>&1 \
      || openssl pkcs12 -export -out "$WORK/thinstt.p12" \
        -inkey "$WORK/thinstt-key.pem" -in "$WORK/thinstt-cert.pem" \
        -passout pass: >/dev/null 2>&1; } \
    && security import "$WORK/thinstt.p12" -k "$KEYCHAIN" \
        -T /usr/bin/codesign >/dev/null; then
        # Let codesign use the key without prompting on every rebuild
        security set-key-partition-list -S apple-tool:,apple:,codesign: \
            -s -k "" "$KEYCHAIN" >/dev/null 2>&1 || true
    else
        echo "Certificate creation failed — falling back to ad-hoc signing."
    fi
fi

if security find-certificate -c "$CERT_NAME" "$KEYCHAIN" >/dev/null 2>&1; then
    codesign --force --deep --sign "$CERT_NAME" "$APP" 2>/dev/null || {
        echo "Note: if macOS asks to allow key access, click 'Always Allow'."
        codesign --force --deep --sign "$CERT_NAME" "$APP"
    }
else
    # Fall back to ad-hoc signing if certificate creation failed
    codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
fi
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true

echo "Installed: $APP"
echo "On first launch macOS will ask for Microphone + Accessibility permission."
open "$APP"
