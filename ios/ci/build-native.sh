#!/usr/bin/env bash
# Builds the Rust bridge (Tulpa runtime + phone:: + MuJoCo shim) as static libraries:
#   ios/build/lib/iphoneos/libtulpa_ios.a         (aarch64-apple-ios)
#   ios/build/lib/iphonesimulator/libtulpa_ios.a  (aarch64-apple-ios-sim)
# Run ios/ci/build-mujoco.sh first. SDKS limits the slices (default: both).
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
SDKS="${SDKS:-iphoneos iphonesimulator}"
export IPHONEOS_DEPLOYMENT_TARGET="${IOS_DEPLOYMENT_TARGET:-16.0}"
cd "$HERE/native"
for sdk in $SDKS; do
  case "$sdk" in
    iphoneos) triple=aarch64-apple-ios ;;
    iphonesimulator) triple=aarch64-apple-ios-sim ;;
    *) echo "unknown sdk $sdk"; exit 1 ;;
  esac
  echo "::group::tulpa_ios for $sdk ($triple)"
  MUJOCO_DIR="$HERE/mujoco/$sdk" \
    cargo build --release --locked --target "$triple"
  mkdir -p "$HERE/build/lib/$sdk"
  cp "target/$triple/release/libtulpa_ios.a" "$HERE/build/lib/$sdk/"
  echo "::endgroup::"
  ls -la "$HERE/build/lib/$sdk"
done
