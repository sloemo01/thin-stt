#!/bin/sh
# Build Thin STT.app, sign it with your local "Thin STT Local Signing"
# certificate, and package a distributable DMG for sharing.
#   sh dist.sh            # build + sign + dist/Thin-STT.dmg
#   sh dist.sh --zip      # also produce a .zip instead of a .dmg
set -eu

APP_NAME="Thin STT"
CERT_NAME="Thin STT Local Signing"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
DIST_DIR="$(pwd)/dist"
APP="$DIST_DIR/$APP_NAME.app"
FORMAT="dmg"
[ "${1:-}" = "--zip" ] && FORMAT="zip"

echo "== Thin STT distributor =="

command -v cargo >/dev/null 2>&1 || { . "$HOME/.cargo/env"; }
command -v cargo >/dev/null 2>&1 || { echo "cargo not found — run install.sh first or install rustup."; exit 1; }

if ! xcrun -f metal >/dev/null 2>&1; then
    if [ -d /Applications/Xcode.app/Contents/Developer ]; then
        export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
    else
        echo "Full Xcode is required to build (gpui compiles Metal shaders)."; exit 1
    fi
fi

# Stamp this build — the app embeds it and dist/build.txt carries it
# to the release so Check for Updates compares builds, not just semver.
BUILD_STAMP="$(date +%s)"
echo "$BUILD_STAMP" > src/buildstamp.txt
cargo build --release --quiet
echo 0 > src/buildstamp.txt   # keep the tree clean; stamp lives in the binary

WORK="$(mktemp -d /tmp/thinstt-dist.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
rm -rf "$APP"
mkdir -p "$DIST_DIR" "$APP/Contents/MacOS" "$APP/Contents/Resources"

# -- icon.icns
ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z "$s" "$s" assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
done
cp assets/icon.png "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cp target/release/thin-stt "$APP/Contents/MacOS/thin-stt"
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
    <key>CFBundleShortVersionString</key><string>0.3.2</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>Thin STT records your microphone while you hold the transcription hotkey.</string>
    <key>NSCameraUsageDescription</key>
    <string>Thin STT reads your lips from the camera only while Lips mode is on and the hotkey is held. Frames are never saved.</string>
</dict>
</plist>
PLIST

# -- your certificate (created once on your Mac, stored in your keychain)
if ! security find-certificate -c "$CERT_NAME" "$KEYCHAIN" >/dev/null 2>&1; then
    echo "Creating your signing certificate (one-time)..."
    if openssl req -x509 -newkey rsa:2048 -nodes \
        -keyout "$WORK/key.pem" -out "$WORK/cert.pem" \
        -days 3650 -subj "/CN=$CERT_NAME/O=Thin STT" \
        -addext "keyUsage=critical,digitalSignature" \
        -addext "extendedKeyUsage=critical,codeSigning" >/dev/null 2>&1 \
    && { openssl pkcs12 -export -legacy -out "$WORK/thinstt.p12" \
        -inkey "$WORK/key.pem" -in "$WORK/cert.pem" -passout pass:thinstt-local >/dev/null 2>&1 \
      || openssl pkcs12 -export -out "$WORK/thinstt.p12" \
        -inkey "$WORK/key.pem" -in "$WORK/cert.pem" -passout pass:thinstt-local >/dev/null 2>&1; } \
    && security import "$WORK/thinstt.p12" -k "$KEYCHAIN" \
        -T /usr/bin/codesign -P thinstt-local >/dev/null; then
        security set-key-partition-list -S apple-tool:,apple:,codesign: \
            -s -k "" "$KEYCHAIN" >/dev/null 2>&1 || true
        echo "If macOS asks to allow key access, click 'Always Allow'."
    else
        echo "Could not create the signing certificate."; exit 1
    fi
fi

codesign --force --deep --sign "$CERT_NAME" "$APP"
codesign --verify --verbose=2 "$APP" 2>&1 | tail -2
codesign -dv "$APP" 2>&1 | grep -E "Identifier|Signature" | head -3

# -- package
OUT="$DIST_DIR/Thin-STT.$FORMAT"
rm -f "$OUT"
if [ "$FORMAT" = "dmg" ]; then
    hdiutil create -volname "$APP_NAME" -srcfolder "$APP" -ov -format UDZO "$OUT" >/dev/null
else
    (cd "$DIST_DIR" && zip -qry "Thin-STT.zip" "$APP_NAME.app")
fi
echo "$BUILD_STAMP" > "$DIST_DIR/build.txt"
echo ""
echo "Done: $OUT"
echo "Signed by: $CERT_NAME (your local certificate)"
echo ""
echo "Share the file. On your friend's Mac:"
echo "  1. Mount the DMG / unzip, drag 'Thin STT.app' to Applications."
echo "  2. First launch: right-click the app -> Open -> Open"
echo "     (Gatekeeper shows 'unidentified developer' once for"
echo "      self-signed apps — that prompt is expected.)"
echo "     Terminal alternative: xattr -dr com.apple.quarantine '/Applications/Thin STT.app'"
