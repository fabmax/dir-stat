#!/usr/bin/env bash
# Builds target/release/dir-stat.app: Info.plist with the privacy purpose strings, the icon,
# and a code signature (design §9, FS §5.2).
#
#   scripts/bundle-macos.sh                      # ad-hoc signature
#   DIRSTAT_SIGN_IDENTITY="Developer ID Application: …" scripts/bundle-macos.sh
#
# Privacy grants (Full Disk Access, Desktop/Documents/Downloads) are tied to the signature.
# An ad-hoc signature changes with every build, so macOS forgets the grants after a rebuild;
# sign with a stable identity (a Developer ID, or a self-signed certificate) to keep them.
set -euo pipefail

cd "$(dirname "$0")/.."
BUNDLE_ID="com.example.dir-stat" # keep in sync with app::BUNDLE_ID
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
APP="target/release/dir-stat.app"

cargo build --release -p dir-stat

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/dir-stat "$APP/Contents/MacOS/dir-stat"
cp crates/app/assets/icon/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

why="dir-stat measures folder sizes to show what uses your disk. It only reads sizes and names; it never opens your files."
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>dir-stat</string>
  <key>CFBundleDisplayName</key><string>dir-stat</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>dir-stat</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSDesktopFolderUsageDescription</key><string>${why}</string>
  <key>NSDocumentsFolderUsageDescription</key><string>${why}</string>
  <key>NSDownloadsFolderUsageDescription</key><string>${why}</string>
  <key>NSNetworkVolumesUsageDescription</key><string>${why}</string>
  <key>NSRemovableVolumesUsageDescription</key><string>${why}</string>
  <key>NSFileProviderDomainUsageDescription</key><string>${why}</string>
  <key>NSAppDataUsageDescription</key><string>${why}</string>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist" >/dev/null

if [[ -n "${DIRSTAT_SIGN_IDENTITY:-}" ]]; then
  codesign --force --options runtime --identifier "$BUNDLE_ID" \
    --sign "$DIRSTAT_SIGN_IDENTITY" "$APP"
else
  echo "warning: no DIRSTAT_SIGN_IDENTITY; signing ad-hoc. macOS forgets privacy grants" \
    "(Full Disk Access, folder access) after every rebuild." >&2
  codesign --force --identifier "$BUNDLE_ID" --sign - "$APP"
fi
codesign --verify --strict "$APP"
echo "Built $APP ($VERSION, $BUNDLE_ID)"
