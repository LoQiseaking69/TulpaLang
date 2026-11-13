#!/usr/bin/env bash
# Build, test and package Tulpa.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> building (release)"
cargo build --release

echo "==> running tests"
cargo test --release

echo "==> checking examples parse"
./target/release/tulpa check examples/*/main.rav

echo "==> packaging"
pkg=target/release-package
rm -rf "$pkg"
mkdir -p "$pkg/bin"
cp target/release/tulpa "$pkg/bin/"
cp -r examples docs README.md LICENSE "$pkg/"
echo "done: $pkg"
