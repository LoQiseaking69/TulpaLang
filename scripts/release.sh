#!/usr/bin/env bash
# Build release packages for one target:
#   scripts/release.sh                                   # host target
#   scripts/release.sh --target aarch64-unknown-linux-gnu --cross   # uses `cross`
#   scripts/release.sh --no-vsix --no-deb
# Output goes to dist/: tulpa-<ver>-<target>.tar.gz, tulpa_<ver>_<arch>.deb, tulpa-lang-<ver>.vsix,
# and SHA256SUMS.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
USE_CROSS=0; VSIX=1; DEB=1
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET="$2"; shift ;;
    --cross) USE_CROSS=1 ;;
    --no-vsix) VSIX=0 ;;
    --no-deb) DEB=0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
DIST=dist
mkdir -p "$DIST"

echo "==> building tulpa $VERSION for $TARGET"
if [ "$USE_CROSS" = 1 ]; then
  cross build --release --locked --target "$TARGET"
else
  cargo build --release --locked --target "$TARGET"
fi
BIN="target/$TARGET/release/tulpa"
[ -x "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

NAME="tulpa-$VERSION-$TARGET"
STAGE="$DIST/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/share/tulpa" "$STAGE/share/doc/tulpa"
cp "$BIN" "$STAGE/bin/"
cp -r examples editors "$STAGE/share/tulpa/"
rm -rf "$STAGE/share/tulpa/editors/vscode/node_modules" "$STAGE/share/tulpa/editors/vscode/out" "$STAGE"/share/tulpa/editors/vscode/*.vsix
cp README.md LICENSE "$STAGE/share/doc/tulpa/"
cp docs/*.md "$STAGE/share/doc/tulpa/"
cat > "$STAGE/install.sh" <<'INSTALL'
#!/usr/bin/env sh
# Installs tulpa. Usage: ./install.sh [PREFIX]   (default: /usr/local as root, ~/.local otherwise)
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
if [ -n "$1" ]; then PREFIX="$1"; elif [ "$(id -u)" = 0 ]; then PREFIX=/usr/local; else PREFIX="$HOME/.local"; fi
install -d "$PREFIX/bin" "$PREFIX/share/tulpa" "$PREFIX/share/doc/tulpa"
install -m 0755 "$HERE/bin/tulpa" "$PREFIX/bin/tulpa"
cp -r "$HERE/share/tulpa/." "$PREFIX/share/tulpa/"
cp -r "$HERE/share/doc/tulpa/." "$PREFIX/share/doc/tulpa/"
echo "installed $PREFIX/bin/tulpa ($("$PREFIX/bin/tulpa" -V))"
echo "examples: $PREFIX/share/tulpa/examples   docs: $PREFIX/share/doc/tulpa"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "note: add $PREFIX/bin to your PATH";; esac
if command -v code >/dev/null 2>&1 && ls "$HERE"/tulpa-lang-*.vsix >/dev/null 2>&1; then
  code --install-extension "$HERE"/tulpa-lang-*.vsix >/dev/null && echo "installed the VS Code extension"
fi
INSTALL
chmod +x "$STAGE/install.sh"

if [ "$VSIX" = 1 ] && command -v npm >/dev/null 2>&1; then
  echo "==> packaging VS Code extension"
  (cd editors/vscode && npm ci --no-audit --no-fund >/dev/null 2>&1 || npm install --no-audit --no-fund >/dev/null; rm -f ./*.vsix; npm run package >/dev/null)
  cp editors/vscode/tulpa-lang-"$VERSION".vsix "$DIST/"
  cp editors/vscode/tulpa-lang-"$VERSION".vsix "$STAGE/"
fi

echo "==> tarball"
tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"

if [ "$DEB" = 1 ] && command -v dpkg-deb >/dev/null 2>&1; then
  case "$TARGET" in
    x86_64-*linux*) ARCH=amd64 ;;
    aarch64-*linux*) ARCH=arm64 ;;
    armv7-*linux*|arm-*linux*) ARCH=armhf ;;
    *) ARCH="" ;;
  esac
  if [ -n "$ARCH" ]; then
    echo "==> .deb ($ARCH)"
    DEBROOT="$DIST/deb-$ARCH"
    rm -rf "$DEBROOT"
    mkdir -p "$DEBROOT/DEBIAN" "$DEBROOT/usr/bin" "$DEBROOT/usr/share/tulpa" "$DEBROOT/usr/share/doc/tulpa"
    install -m 0755 "$BIN" "$DEBROOT/usr/bin/tulpa"
    cp -r "$STAGE/share/tulpa/." "$DEBROOT/usr/share/tulpa/"
    cp -r "$STAGE/share/doc/tulpa/." "$DEBROOT/usr/share/doc/tulpa/"
    SIZE=$(du -sk "$DEBROOT/usr" | cut -f1)
    # declare the real glibc floor of this build so apt refuses it cleanly on older systems
    GLIBC_MIN=$(objdump -T "$BIN" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1 | cut -d_ -f2 || true)
    LIBC_DEP="libc6${GLIBC_MIN:+ (>= $GLIBC_MIN)}"
    cat > "$DEBROOT/DEBIAN/control" <<CONTROL
Package: tulpa
Version: $VERSION
Section: devel
Priority: optional
Architecture: $ARCH
Maintainer: HermiTech <noreply@hermitech.dev>
Installed-Size: $SIZE
Depends: $LIBC_DEP
Recommends: clang
Homepage: https://github.com/LoQiseaking69/TulpaLang
Description: Tulpa, a language for robots and hardware
 Python-flavoured scripting with units of measure, contracts, state machines,
 reactive safety rules, GPIO/I2C/SPI/UART/PWM/CAN/BLE devices with a simulator,
 record/replay of hardware sessions, a bytecode VM, LLVM native code
 generation and a language server.
CONTROL
    dpkg-deb --root-owner-group --build "$DEBROOT" "$DIST/tulpa_${VERSION}_${ARCH}.deb" >/dev/null
    rm -rf "$DEBROOT"
  fi
fi

rm -rf "$STAGE"
(cd "$DIST" && sha256sum ./*.tar.gz ./*.deb ./*.vsix 2>/dev/null | sed 's# \./# #' > SHA256SUMS || true)
echo "==> done:"; ls -1 "$DIST"
