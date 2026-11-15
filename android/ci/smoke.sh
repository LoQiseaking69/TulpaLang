#!/usr/bin/env bash
# Emulator smoke test: install, launch, run a guide snippet and a library example, and capture
# screenshots of each screen. Fails on any crash of the app process.
set -uo pipefail
APK="$1"; OUT="${2:-smoke}"
PKG=dev.hermitech.tulpafield
mkdir -p "$OUT"
adb install -r "$APK"
adb logcat -c

shot() { adb exec-out screencap -p > "$OUT/$1.png"; }
dump() { adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; adb shell cat /sdcard/ui.xml; }
# tap the centre of the first node whose text (or content-desc) matches exactly
cat > /tmp/find_node.py <<'PY'
import re,sys
want=sys.argv[1]; x=sys.stdin.read(); best=None
for m in re.finditer(r'<node [^>]*>', x):
    n=m.group(0)
    t=re.search(r' text="([^"]*)"',n); d=re.search(r'content-desc="([^"]*)"',n)
    if want in ((t.group(1) if t else None),(d.group(1) if d else None)):
        a=list(map(int,re.findall(r'\d+',re.search(r'bounds="([^"]*)"',n).group(1))))
        if best is None or a[1] < best[1]: best=a
if best: print((best[0]+best[2])//2,(best[1]+best[3])//2)
PY
# tap the centre of the top-most node whose text or content-desc is exactly $1
tap() {
  local b; b=$(dump | python3 /tmp/find_node.py "$1")
  if [ -z "$b" ]; then echo "!! no node '$1'"; return 1; fi
  adb shell input tap $b; echo "tapped '$1' at $b"
}
alive() { adb shell pidof $PKG >/dev/null || { echo "!! app process died"; adb logcat -d -b crash; adb logcat -d | grep -E "AndroidRuntime|FATAL|tulpa" | tail -80; exit 1; }; }
has() { dump | grep -q "$1"; }
texts() { dump | grep -o ' text="[^"]\+"' | sed 's/ text=//' | tr '\n' ' ' | cut -c1-1500; echo; }

adb shell am start -W -n $PKG/.MainActivity
sleep 6; alive; shot 01-welcome
has "Welcome to Tulpa Field" && echo "ok: first launch opens the guide" || echo "!! welcome chapter not shown"

# scroll the welcome chapter until the first code block's Run button is on screen
for k in 1 2 3 4 5 6 7 8; do
  y=$(dump | python3 /tmp/find_node.py "Run" | awk '{print $2}')
  [ -n "$y" ] && [ "$y" -lt 2000 ] && break
  adb shell input swipe 540 1600 540 1000 300; sleep 1
done
shot 01b-welcome-scrolled
tap "Run"; sleep 5; alive; shot 02-run-snippet
texts
dump | grep -o 'text="[^"]*km/h[^"]*"' | head -3
has "average speed" && echo "ok: guide snippet ran" || { echo "!! snippet output missing"; FAIL=1; }

tap "Learn"; sleep 2; tap "Back" || adb shell input keyevent 4; sleep 2; tap "Library"; sleep 2; shot 03-library
tap "Hello, World" ; sleep 2; shot 04-example; tap "Open as project"; sleep 4; alive; shot 05-editor
has "main.rav" && echo "ok: editor open" || { echo "!! editor not open"; FAIL=1; }
tap "Run"; sleep 5; alive; shot 06-run-example
has "Hello, World!" && echo "ok: example ran" || { echo "!! example output missing"; FAIL=1; }

tap "REPL"; sleep 2
tap "REPL input"; sleep 1
adb shell "input text '3[ft]%s+%s1[m]'"; adb shell input keyevent 66; sleep 3; alive; shot 07-repl
texts
has "6.2808" && echo "ok: REPL evaluated" || { echo "!! REPL output missing"; FAIL=1; }

tap "Instruments"; sleep 1; tap "Sensors"; sleep 4; alive; shot 08-sensors
adb shell input keyevent 4; sleep 1
tap "Projects"; sleep 2; shot 09-projects
alive
adb logcat -d | grep -A30 "FATAL EXCEPTION" && { echo "!! crash in log"; FAIL=1; }
# small JPEG previews in the log (artifact downloads are not always reachable)
python3 -m pip install -q pillow >/dev/null 2>&1
python3 - "$OUT" <<'PY'
import base64, glob, io, sys
from PIL import Image
for f in sorted(glob.glob(sys.argv[1] + "/*.png")):
    im = Image.open(f).convert("RGB"); im.thumbnail((432, 960))
    b = io.BytesIO(); im.save(b, "JPEG", quality=60)
    print(f"@@shot {f.split('/')[-1]} {base64.b64encode(b.getvalue()).decode()}")
PY
exit ${FAIL:-0}
