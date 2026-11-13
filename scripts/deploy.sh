#!/usr/bin/env bash
# Install the tulpa binary.  Usage: scripts/deploy.sh [PREFIX]   (default: ~/.local)
set -euo pipefail
cd "$(dirname "$0")/.."
PREFIX="${1:-$HOME/.local}"

[ -x target/release/tulpa ] || cargo build --release
install -d "$PREFIX/bin"
install -m 0755 target/release/tulpa "$PREFIX/bin/tulpa"
install -d "$PREFIX/lib/tulpa" && cp stdlib/*.rav "$PREFIX/lib/tulpa/"
echo "installed $PREFIX/bin/tulpa"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "note: add $PREFIX/bin to your PATH";; esac
