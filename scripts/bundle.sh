#!/bin/sh
# Builds "Lampster Remote.app" from the release binary and the Swift helpers,
# puts it into ~/Applications and (re)starts it.
#
#   ./scripts/bundle.sh
#   ./scripts/bundle.sh --build-only   # only "target/Lampster Remote.app"
#
# Signing: by default the app is signed locally ("ad hoc"). macOS then sees
# every new build as a new program and asks for Bluetooth access again.
# The access stays with a certificate of your own:
#   Keychain Access → Keychain Access (menu) → Certificate Assistant
#   → Create a Certificate … → name "Lampster Remote Dev", identity type
#   "Self-Signed Root", certificate type "Code Signing".
# A certificate "Lampster Remote Dev" in the keychain is used
# automatically; pick another one with
#   LAMPSTER_SIGN_ID="Name" ./scripts/bundle.sh
set -eu
cd "$(dirname "$0")/.."

case "${1:-}" in
    "") INSTALL=yes ;;
    --build-only) INSTALL=no ;;
    *) echo "usage: $0 [--build-only]" >&2; exit 2 ;;
esac

cargo build --release --locked

# Version from Cargo.toml (the first `version` line, the package's).
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)

APP="target/Lampster Remote.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/lampster-remote "$APP/Contents/MacOS/LampsterRemote"
# The oldest macOS the app runs on (like LSMinimumSystemVersion below);
# without a fixed target the building macOS would count.
TARGET="arm64-apple-macos14.0"
# Swift helper: Bluetooth (CoreBluetooth).
xcrun swiftc -O -target "$TARGET" -o "$APP/Contents/MacOS/lampster-ble" helpers/ble/*.swift
# Swift helper: menu bar icon and panel (SwiftUI).
xcrun swiftc -O -target "$TARGET" -parse-as-library -swift-version 5 -o "$APP/Contents/MacOS/lampster-ui" helpers/ui/*.swift

# The app's icon, drawn by the panel's helper from the menu bar's robot.
ICONSET="target/AppIcon.iconset"
rm -rf "$ICONSET"
"$APP/Contents/MacOS/lampster-ui" --icon "$ICONSET"
mkdir -p "$APP/Contents/Resources"
iconutil -c icns -o "$APP/Contents/Resources/AppIcon.icns" "$ICONSET"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Lampster Remote</string>
    <key>CFBundleDisplayName</key>
    <string>Lampster Remote</string>
    <key>CFBundleIdentifier</key>
    <string>local.lampster-remote</string>
    <key>CFBundleExecutable</key>
    <string>LampsterRemote</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSBluetoothAlwaysUsageDescription</key>
    <string>Lampster Remote switches your Lampster lamp and sets its light over Bluetooth.</string>
</dict>
</plist>
PLIST

if [ -z "${LAMPSTER_SIGN_ID:-}" ]; then
    if security find-identity -p codesigning | grep -q '"Lampster Remote Dev"'; then
        LAMPSTER_SIGN_ID="Lampster Remote Dev"
    fi
fi
SIGN_ID="${LAMPSTER_SIGN_ID:--}"
codesign --force --sign "$SIGN_ID" --identifier local.lampster-remote.ble "$APP/Contents/MacOS/lampster-ble"
codesign --force --sign "$SIGN_ID" --identifier local.lampster-remote.ui "$APP/Contents/MacOS/lampster-ui"
codesign --force --sign "$SIGN_ID" --identifier local.lampster-remote "$APP"

if [ "$INSTALL" = no ]; then
    echo "Built: $APP (signature: $SIGN_ID)"
    exit 0
fi

# Quit a running Lampster Remote: only the supervisor gets the signal (its
# command line ends in the executable's name, the child's in "--child");
# the child notices through the pipe and stops its helpers. Whatever from
# the bundle still runs after 10 s is killed.
SUPERVISOR='Lampster Remote\.app/Contents/MacOS/LampsterRemote$'
BUNDLE='Lampster Remote\.app/Contents/MacOS/'
if pkill -f "$SUPERVISOR"; then
    waited=0
    while pgrep -f "$BUNDLE" >/dev/null; do
        if [ "$waited" -ge 50 ]; then
            pkill -KILL -f "$BUNDLE" || true
            break
        fi
        waited=$((waited + 1))
        sleep 0.2
    done
fi

mkdir -p "$HOME/Applications"
rm -rf "$HOME/Applications/Lampster Remote.app"
cp -R "$APP" "$HOME/Applications/"
# A new date tells Finder and Spotlight to read the icon again.
touch "$HOME/Applications/Lampster Remote.app"
open "$HOME/Applications/Lampster Remote.app"
echo "Built and started: ~/Applications/Lampster Remote.app (signature: $SIGN_ID)"
