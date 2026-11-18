#!/usr/bin/env bash
# Builds a Release archive for devices without a signing identity, then packages an .ipa that is
# ad-hoc ("-") signed. No Apple Developer account is needed: install it with AltStore, SideStore or
# Sideloadly, which re-sign it on your machine with your own (free) Apple ID; TrollStore installs it
# as-is. Output: dist/TulpaField-<version>.ipa
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
VERSION="${TULPA_FIELD_VERSION:-$(tr -d ' \n' < "$REPO/android/VERSION")}"
DIST="$REPO/dist"; mkdir -p "$DIST"
cd "$HERE"
rm -rf build/TulpaField.xcarchive build/Payload
set +e
xcodebuild archive \
  -project TulpaField.xcodeproj -scheme TulpaField -configuration Release \
  -sdk iphoneos -destination "generic/platform=iOS" \
  -archivePath build/TulpaField.xcarchive -derivedDataPath build/DerivedData-dev \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY="" \
  COMPILER_INDEX_STORE_ENABLE=NO > build/archive.log 2>&1
rc=$?
set -e
grep -E "error:|\*\* " build/archive.log || true
[ "$rc" -eq 0 ] || { echo "::error title=iOS archive::xcodebuild archive failed"; grep -E "error:" -A3 build/archive.log | head -80; tail -40 build/archive.log; exit 1; }
APP="build/TulpaField.xcarchive/Products/Applications/TulpaField.app"
[ -d "$APP" ] || { echo "archive has no TulpaField.app"; exit 1; }
mkdir -p build/Payload
cp -R "$APP" build/Payload/
APP="build/Payload/TulpaField.app"
# ad-hoc signature: gives the bundle a valid code directory so sideloading tools can re-sign it
cat > build/adhoc.entitlements <<'PL'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>application-identifier</key><string>dev.hermitech.tulpafield</string>
  <key>get-task-allow</key><false/>
</dict></plist>
PL
find "$APP" -name "*.dylib" -o -name "*.framework" | while read -r f; do codesign --force --sign - "$f"; done
codesign --force --sign - --entitlements build/adhoc.entitlements --timestamp=none "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" -c "Print :CFBundleVersion" -c "Print :MinimumOSVersion" "$APP/Info.plist"
lipo -info "$APP/TulpaField"
IPA="$DIST/TulpaField-$VERSION.ipa"
rm -f "$IPA"
(cd build && ditto -c -k --norsrc --keepParent Payload "$IPA")
unzip -l "$IPA" | tail -1
size=$(stat -f %z "$IPA")
echo "IPA: $IPA ($((size / 1000000)) MB)"
[ "$size" -gt 1000000 ] || { echo "::error::IPA is suspiciously small"; exit 1; }
shasum -a 256 "$IPA" | tee "$IPA.sha256"
