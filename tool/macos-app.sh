#!/usr/bin/env bash
# Wraps the macOS executable in Istma.app, so a double-click opens the window instead of a Terminal.
# Usage: tool/macos-app.sh <executable> <output-dir>
set -euo pipefail

ROOT="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${1:?usage: macos-app.sh <executable> <output-dir>}"
OUT="${2:?usage: macos-app.sh <executable> <output-dir>}"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)"
APP="$OUT/Istma.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/istma"
chmod +x "$APP/Contents/MacOS/istma"
cp "$ROOT/assets/macos/istma.icns" "$APP/Contents/Resources/istma.icns"
cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Istma</string>
  <key>CFBundleDisplayName</key><string>Istma</string>
  <key>CFBundleIdentifier</key><string>com.mudlej.istma</string>
  <key>CFBundleExecutable</key><string>istma</string>
  <key>CFBundleIconFile</key><string>istma</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
echo "$APP"
