#!/usr/bin/env bash
# Writes an AltStore / SideStore source (https://faq.altstore.io/developers/make-a-source) that
# points at the release's .ipa, so the app can be added (and updated) from inside AltStore/SideStore.
# Usage: altstore-source.sh <ipa> <tag> <out.json>
set -euo pipefail
IPA="$1"; TAG="$2"; OUT="$3"
REPO="${GITHUB_REPOSITORY:-LoQiseaking69/TulpaLang}"
VERSION="${TAG#field-v}"
SIZE=$(stat -f %z "$IPA" 2>/dev/null || stat -c %s "$IPA")
BUILD="${TF_BUILD:-1}"
NAME="$(basename "$IPA")"
python3 - "$OUT" <<PY
import json, sys, datetime
raw = "https://raw.githubusercontent.com/$REPO/main/ios"
src = {
  "name": "Tulpa Field",
  "identifier": "dev.hermitech.tulpafield.source",
  "subtitle": "The Tulpa IDE for iPhone and iPad",
  "description": "Write, run and verify Tulpa programs on your phone, with sensors, a REPL and a MuJoCo 3D simulator.",
  "iconURL": raw + "/TulpaField/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png",
  "website": "https://github.com/$REPO",
  "tintColor": "#7C5CFF",
  "apps": [{
    "name": "Tulpa Field",
    "bundleIdentifier": "dev.hermitech.tulpafield",
    "developerName": "HermiTech",
    "subtitle": "Tulpa IDE with sensors, REPL and 3D simulation",
    "localizedDescription": "Tulpa Field is the Tulpa IDE: a syntax-highlighting editor with live diagnostics, the Tulpa runtime with unit-checked arithmetic, phone sensors through phone::, a REPL, contract verification, state-machine views, and MuJoCo co-simulation rendered in 3D.",
    "iconURL": raw + "/TulpaField/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png",
    "tintColor": "#7C5CFF",
    "category": "developer",
    "screenshots": [],
    "versions": [{
      "version": "$VERSION",
      "buildVersion": "$BUILD",
      "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
      "localizedDescription": "Tulpa Field $VERSION for iOS.",
      "downloadURL": "https://github.com/$REPO/releases/download/$TAG/$NAME",
      "size": $SIZE,
      "minOSVersion": "16.0"
    }],
    "appPermissions": {
      "entitlements": [],
      "privacy": {
        "NSMotionUsageDescription": "Tulpa programs read the accelerometer, gyroscope, magnetometer and barometer through phone::sensor.",
        "NSLocationWhenInUseUsageDescription": "Tulpa programs can read GPS position and speed through phone::sensor(\"gps\").",
        "NSCameraUsageDescription": "Tulpa programs can switch the flashlight on and off through phone::torch.",
        "NSLocalNetworkUsageDescription": "Co-simulation and Tulpa Link talk to programs and devices on your local network."
      }
    }
  }],
  "news": []
}
json.dump(src, open(sys.argv[1], "w"), indent=2)
PY
echo "AltStore source: $OUT"
