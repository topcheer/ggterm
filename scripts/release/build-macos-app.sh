#!/bin/bash
# build-macos-app.sh — Build macOS .app bundle and .dmg
# Usage: scripts/release/build-macos-app.sh <version>
set -euo pipefail

VERSION="${1:-0.1.0}"
APP_NAME="GGTerm"
BUNDLE_ID="dev.ggterm.app"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIST_DIR="$REPO_ROOT/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"

mkdir -p "$DIST_DIR"
rm -rf "$APP_DIR"

# .app directory structure
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy universal binary
cp "$REPO_ROOT/target/release/ggterm-universal" "$APP_DIR/Contents/MacOS/ggterm"
chmod +x "$APP_DIR/Contents/MacOS/ggterm"

# Copy icon
cp "$REPO_ROOT/assets/icon.icns" "$APP_DIR/Contents/Resources/ggterm.icns" 2>/dev/null || \
  cp "$REPO_ROOT/assets/logo.icns" "$APP_DIR/Contents/Resources/ggterm.icns" 2>/dev/null || true

# Info.plist
cat > "$APP_DIR/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>ggterm</string>
    <key>CFBundleIconFile</key>
    <string>ggterm.icns</string>
    <key>CFBundleCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
PLIST

# Build .dmg
DMG_PATH="$DIST_DIR/GGTerm-$VERSION.dmg"
rm -f "$DMG_PATH"

# Create staging area
DMG_STAGING="$DIST_DIR/dmg-staging"
rm -rf "$DMG_STAGING"
mkdir -p "$DMG_STAGING"
cp -R "$APP_DIR" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"

# Create read-write DMG
TEMP_DMG="$DIST_DIR/temp-$VERSION.dmg"
SIZE=$(du -sk "$DMG_STAGING" | awk '{print $1}')
SIZE=$((SIZE + 10240))

hdiutil create -srcfolder "$DMG_STAGING" -volname "$APP_NAME" -fs HFS+ \
  -format UDRW -size ${SIZE}k "$TEMP_DMG" -quiet

# Convert to compressed read-only
hdiutil convert "$TEMP_DMG" -format UDZO -imagekey zlib-level=9 -o "$DMG_PATH" -quiet
rm -f "$TEMP_DMG"
rm -rf "$DMG_STAGING"

echo "Created: $DMG_PATH"
ls -lh "$DMG_PATH"
