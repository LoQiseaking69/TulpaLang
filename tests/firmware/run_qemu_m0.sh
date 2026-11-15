#!/usr/bin/env bash
# Builds a Tulpa program for Cortex-M0 (the RP2040's core family) and runs it on an emulated
# BBC micro:bit, comparing its console output with the interpreter's (`--virtual-time`).
#   tests/firmware/run_qemu_m0.sh program.rav [tulpa]
# Needs: clang, arm-none-eabi-gcc + newlib, qemu-system-arm.
set -euo pipefail
src=$1
tulpa=${2:-tulpa}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
"$tulpa" build "$src" --target cortex-m0 --sim -o "$work/app.o" 2>"$work/build.log" || { cat "$work/build.log"; exit 1; }
"$tulpa" runtime "$work/rt" --target sim >/dev/null
arm-none-eabi-gcc -mcpu=cortex-m0 -mthumb -Os -std=c11 --specs=rdimon.specs -nostartfiles \
    -DTULPA_MAX_XFER=64 -DSIM_I2C=4 -DSIM_PINS=64 -I "$work/rt" -I "$work" \
    -T "$here/cortex_m0.ld" -Wl,--gc-sections -ffunction-sections -fdata-sections \
    "$here/start_m0.c" "$work/rt/tulpa_rt.c" "$work/rt/hal_sim.c" "$work/app.o" -lm -o "$work/app.elf"
arm-none-eabi-size "$work/app.elf" >&2
timeout 60 qemu-system-arm -M microbit -nographic -monitor none -serial none \
    -semihosting-config enable=on,target=native -kernel "$work/app.elf" >"$work/native.txt" 2>&1 || true
"$tulpa" run --virtual-time "$src" >"$work/interp.txt" 2>/dev/null || true
if diff -u "$work/interp.txt" "$work/native.txt"; then
    echo "cortex-m0 (qemu microbit): output matches the interpreter ($(wc -l <"$work/native.txt") lines)"
else
    echo "cortex-m0 (qemu microbit): output differs" >&2
    exit 1
fi
