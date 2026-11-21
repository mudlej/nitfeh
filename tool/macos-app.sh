#!/usr/bin/env bash
# Joins the two macOS builds into one universal executable, wraps it in Nitfeh.app (so a double-click opens the
# window instead of a Terminal), and puts it in a disk image: <output-dir>/Nitfeh-macos.dmg. Runs on a Mac.
# Usage: tool/macos-app.sh <arm64 executable> <x86_64 executable> <output-dir>
set -euo pipefail

ROOT="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
USAGE="usage: macos-app.sh <arm64 executable> <x86_64 executable> <output-dir>"
ARM="${1:?$USAGE}"
INTEL="${2:?$USAGE}"
OUT="${3:?$USAGE}"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
APP="$OUT/Nitfeh.app"

DMG="$OUT/Nitfeh-macos.dmg"

rm -rf "$APP" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/nitfeh" "$ARM" "$INTEL"
chmod +x "$APP/Contents/MacOS/nitfeh"
cp "$ROOT/assets/macos/nitfeh.icns" "$APP/Contents/Resources/nitfeh.icns"
cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Nitfeh</string>
  <key>CFBundleDisplayName</key><string>Nitfeh</string>
  <key>CFBundleIdentifier</key><string>com.mudlej.Nitfeh</string>
  <key>CFBundleExecutable</key><string>nitfeh</string>
  <key>CFBundleIconFile</key><string>nitfeh</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Apple silicon runs only signed code. This is the free "ad hoc" signature, not a developer identity.
codesign --force --deep --sign - "$APP"
lipo -info "$APP/Contents/MacOS/nitfeh"

# The image shows the app beside a shortcut to Applications, to drag one onto the other.
STAGE="$(mktemp -d)"
trap 'rm -rf -- "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
# hdiutil sometimes reports "Resource busy" on CI runners; it works on a second try.
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname Nitfeh -srcfolder "$STAGE" -format UDZO -ov "$DMG"; then
    break
  fi
  [[ "$attempt" -lt 5 ]] || { echo "macos-app: hdiutil could not create $DMG" >&2; exit 1; }
  sleep 3
done
echo "$DMG"
