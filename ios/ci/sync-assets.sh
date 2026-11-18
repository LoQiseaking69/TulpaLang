#!/usr/bin/env bash
# Copies the bundled content the app ships with into ios/build/TulpaAssets (same layout as the
# Android build's syncTulpaAssets task): examples, the guide + language reference, stdlib, MuJoCo
# models, and the example catalogue.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
SRC="$REPO/android/app/src/main"
OUT="$HERE/build/TulpaAssets"
rm -rf "$OUT"; mkdir -p "$OUT"/{examples,guide/reference,guide/field,stdlib,models}
cp -R "$REPO/examples/." "$OUT/examples/"
cp -R "$SRC/tulpa/examples/." "$OUT/examples/"
cp -R "$REPO/docs/." "$OUT/guide/reference/"
cp -R "$SRC/tulpa/guide/." "$OUT/guide/field/"
cp -R "$REPO/stdlib/." "$OUT/stdlib/"
cp -R "$SRC/tulpa/models/." "$OUT/models/"
cp "$SRC/assets/catalog.json" "$SRC/assets/guide.json" "$OUT/"
find "$OUT" -name .DS_Store -delete
echo "TulpaAssets: $(find "$OUT" -type f | wc -l | tr -d ' ') files, $(du -sh "$OUT" | cut -f1)"
