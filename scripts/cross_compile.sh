#!/usr/bin/env bash
# Cross-compile for single-board computers.
#   scripts/cross_compile.sh            -> Raspberry Pi 3/4/5 (64-bit), aarch64-unknown-linux-gnu
#   scripts/cross_compile.sh armv7      -> 32-bit Pi OS, armv7-unknown-linux-gnueabihf
# Uses `cross` (https://github.com/cross-rs/cross) when installed, otherwise cargo + rustup target.
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-aarch64}" in
  aarch64) TARGET=aarch64-unknown-linux-gnu ;;
  armv7)   TARGET=armv7-unknown-linux-gnueabihf ;;
  *)       TARGET="$1" ;;
esac
if command -v cross >/dev/null 2>&1; then
  cross build --release --target "$TARGET"
else
  rustup target add "$TARGET"
  cargo build --release --target "$TARGET"
fi
echo "binary: target/$TARGET/release/tulpa"
