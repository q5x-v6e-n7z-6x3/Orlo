#!/bin/sh
# Wrap the release binary in a minimal macOS app bundle at target/Orlo.app.
set -eu
cd "$(dirname "$0")/.."
# --locked: ship exactly the versions Cargo.lock pins, the ones CI tests and runs cargo audit on
# Build paths (the home folder, Cargo's registry, this checkout) end up in panic locations;
# remap the home folder so a shipped binary does not carry the builder's user name.
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=~" cargo build --locked -p orlo-app --release
APP=target/Orlo.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# only the system's libraries: a signed app refuses others (and other Macs lack them)
FOREIGN=$(otool -L target/release/Orlo | tail -n +2 | awk '{print $1}' | grep -v -E '^/(usr/lib|System)/' || true)
if [ -n "$FOREIGN" ]; then
  echo "Orlo links libraries outside macOS, which the app cannot load:" >&2
  echo "$FOREIGN" >&2
  exit 1
fi
cp target/release/Orlo "$APP/Contents/MacOS/Orlo"
# AppIcon.icns from assets/icon.png, one size per iconset slot
SET=target/AppIcon.iconset
rm -rf "$SET" && mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$SET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) assets/icon.png --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$SET"
# the bundle's version is the app crate's
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' crates/app/Cargo.toml | head -n 1)
[ -n "$VERSION" ] || { echo "no version in crates/app/Cargo.toml" >&2; exit 1; }
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Orlo</string>
  <key>CFBundleDisplayName</key><string>Orlo</string>
  <key>CFBundleIdentifier</key><string>dev.orlo.editor</string>
  <key>CFBundleExecutable</key><string>Orlo</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# Sign the bundle, not just the binary: the linker's ad-hoc signature leaves Info.plist and
# Resources unsealed, which Gatekeeper rejects. Notarization requires the hardened runtime.
# ORLO_SIGN_IDENTITY="Developer ID Application: …" signs for distribution; the default "-"
# (ad-hoc) runs on the Mac that built it but is not accepted by Gatekeeper elsewhere.
# The one entitlement lets wasm grammars run: without it the first wasm parse is killed.
IDENTITY=${ORLO_SIGN_IDENTITY:--}
if [ "$IDENTITY" = - ]; then TIMESTAMP=--timestamp=none; else TIMESTAMP=--timestamp; fi
codesign --force --options runtime --entitlements scripts/Orlo.entitlements "$TIMESTAMP" --sign "$IDENTITY" "$APP"
codesign --verify --strict --verbose=2 "$APP"
# ORLO_NOTARY_PROFILE names a `xcrun notarytool store-credentials` keychain profile.
if [ -n "${ORLO_NOTARY_PROFILE:-}" ]; then
  [ "$IDENTITY" != - ] || { echo "notarizing needs ORLO_SIGN_IDENTITY" >&2; exit 1; }
  ZIP=target/Orlo.zip
  rm -f "$ZIP" && ditto -c -k --keepParent "$APP" "$ZIP"
  xcrun notarytool submit "$ZIP" --keychain-profile "$ORLO_NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  # the download carries the stapled ticket, so first launch works offline
  rm -f "$ZIP" && ditto -c -k --keepParent "$APP" "$ZIP"
  spctl --assess --type execute --verbose=2 "$APP"
  echo "notarized $ZIP"
fi
echo "built $APP"
