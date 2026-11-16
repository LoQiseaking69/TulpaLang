#!/usr/bin/env bash
# Runs a Tulpa program on the dynamic tier (the Tulpa VM for firmware, tulpa_dyn.c) under QEMU
# and compares its console output with the interpreter's.
#   tests/firmware/run_qemu_dyn.sh <m0|rv32> program.rav [tulpa]
# m0:   Cortex-M0 code with the RP2040's memory budget (256 KB RAM) on an MPS2-AN385 board
# rv32: RV32IMC (the ESP32-C3 ISA) with 384 KB RAM on the QEMU virt board
set -euo pipefail
arch=$1
src=$2
tulpa=${3:-tulpa}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
target=cortex-m0
[ "$arch" = rv32 ] && target=esp32c3
"$tulpa" build "$src" --target $target --sim --dynamic -o "$work/app.o" 2>"$work/build.log" || { cat "$work/build.log"; exit 1; }
"$tulpa" runtime "$work/rt" --target sim >/dev/null
if [ "$arch" = m0 ]; then
    arm-none-eabi-gcc -mcpu=cortex-m0 -mthumb -Os -std=c11 --specs=rdimon.specs -nostartfiles \
        -DTULPA_MAX_XFER=64 -DSIM_I2C=4 -DSIM_PINS=64 -I "$work/rt" -I "$work" \
        -T "$here/rp2040_like.ld" -Wl,--gc-sections -ffunction-sections -fdata-sections \
        "$here/start_m0.c" "$work/rt/tulpa_rt.c" "$work/rt/tulpa_dyn.c" "$work/rt/hal_sim.c" "$work/app.o" -lm -o "$work/app.elf"
    arm-none-eabi-size "$work/app.elf" >&2
    timeout 120 qemu-system-arm -M mps2-an385 -nographic -monitor none -serial none \
        -semihosting-config enable=on,target=native -kernel "$work/app.elf" >"$work/native.txt" 2>&1 || true
else
    cat >"$work/start.c" <<'C'
#include "tulpa_hal.h"
int main(void) { tulpa_entry(); return 0; }
C
    riscv64-unknown-elf-gcc -march=rv32imc_zicsr_zifencei -mabi=ilp32 -Os -std=c11 \
        --specs=picolibc.specs --oslib=semihost --crt0=semihost \
        -Wl,--defsym=__flash=0x80000000 -Wl,--defsym=__flash_size=0x200000 \
        -Wl,--defsym=__ram=0x80200000 -Wl,--defsym=__ram_size=0x60000 -Wl,--defsym=__stack_size=0x8000 \
        -Wl,--gc-sections -ffunction-sections -fdata-sections \
        -I "$work/rt" -I "$work" "$work/start.c" "$work/rt/tulpa_rt.c" "$work/rt/tulpa_dyn.c" "$work/rt/hal_sim.c" "$work/app.o" -lm \
        -o "$work/app.elf"
    riscv64-unknown-elf-size "$work/app.elf" >&2
    timeout 120 qemu-system-riscv32 -M virt -cpu rv32 -bios none -nographic -monitor none -serial none \
        -semihosting-config enable=on,target=native -kernel "$work/app.elf" >"$work/native.txt" 2>&1 || true
fi
"$tulpa" run --virtual-time "$src" >"$work/interp.txt" 2>/dev/null || true
if diff -u "$work/interp.txt" "$work/native.txt"; then
    echo "dynamic tier on $arch (qemu): output matches the interpreter ($(wc -l <"$work/native.txt") lines)"
else
    echo "dynamic tier on $arch (qemu): output differs" >&2
    exit 1
fi
