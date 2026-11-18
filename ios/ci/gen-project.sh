#!/usr/bin/env bash
# Generates ios/TulpaField.xcodeproj with XcodeGen. The version is shared with Android
# (android/VERSION, or $TULPA_FIELD_VERSION); the build number is the commit count.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
export TF_VERSION="${TULPA_FIELD_VERSION:-$(tr -d ' \n' < "$REPO/android/VERSION")}"
export TF_BUILD="${TF_BUILD:-$(git -C "$REPO" rev-list --count HEAD 2>/dev/null || echo 1)}"
[ -d "$HERE/build/TulpaAssets" ] || "$HERE/ci/sync-assets.sh"
cd "$HERE"
xcodegen generate --spec project.yml
echo "TulpaField $TF_VERSION ($TF_BUILD)"
