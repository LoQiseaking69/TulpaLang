#!/usr/bin/env bash
# Builds Tulpa Field for the iOS Simulator and runs the XCUITest smoke suite on an iPhone simulator.
# Screenshots land in $1 (default ios/smoke); the result bundle in ios/build/smoke.xcresult.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$(mkdir -p "${1:-$HERE/smoke}" && cd "${1:-$HERE/smoke}" && pwd)"
cd "$HERE"
# newest available iPhone simulator
UDID="$(xcrun simctl list devices available -j | python3 -c '
import json, re, sys
d = json.load(sys.stdin)["devices"]
best = None
for rt, devs in d.items():
    m = re.search(r"iOS-(\d+)-(\d+)", rt)
    if not m: continue
    v = (int(m.group(1)), int(m.group(2)))
    for x in devs:
        if x["name"].startswith("iPhone") and x.get("isAvailable", True):
            k = (v, "Pro" in x["name"], x["name"])
            if best is None or k > best[0]: best = (k, x["udid"])
print(best[1] if best else "")')"
[ -n "$UDID" ] || { echo "no iPhone simulator available"; xcrun simctl list devices; exit 1; }
xcrun simctl list devices | grep "$UDID"
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b > /dev/null
rm -rf build/smoke.xcresult
set +e
TEST_RUNNER_TF_SHOT_DIR="$OUT" xcodebuild test \
  -project TulpaField.xcodeproj -scheme TulpaField -configuration Debug \
  -destination "id=$UDID" -derivedDataPath build/DerivedData-sim \
  -resultBundlePath build/smoke.xcresult \
  CODE_SIGN_IDENTITY="-" CODE_SIGNING_REQUIRED=NO DEVELOPMENT_TEAM="" \
  COMPILER_INDEX_STORE_ENABLE=NO 2>&1 | tee build/smoke.log | grep -E "error:|warning: unre|Test Case|Executed|\*\* |SmokeTests" 
rc=${PIPESTATUS[0]}
set -e
xcrun simctl io "$UDID" screenshot "$OUT/99-final.png" > /dev/null 2>&1 || true
# app logs for diagnosis
xcrun simctl spawn "$UDID" log show --last 10m --style compact --predicate 'process == "TulpaField"' > "$OUT/app.log" 2>/dev/null || true
ls -la "$OUT"
if [ "$rc" -ne 0 ]; then
  echo "::error title=iOS smoke test::xcodebuild test failed ($rc)"
  grep -E "error:|failed|Fatal|crash" build/smoke.log | head -60 || true
fi
exit "$rc"
