#!/usr/bin/env bash
# Builds a Tulpa program for the ESP32-C3's ISA (RV32IMC) and runs it on QEMU's RISC-V virt
# board, comparing its console output with the interpreter's (`--virtual-time`).
#   tests/firmware/run_qemu_riscv32.sh program.rav [tulpa]
# Needs: clang, riscv64-unknown-elf-gcc + picolibc, qemu-system-riscv32.
set -euo pipefail
src=$1
tulpa=${2:-tulpa}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
"$tulpa" build "$src" --target esp32c3 --sim -o "$work/app.o" 2>"$work/build.log" || { cat "$work/build.log"; exit 1; }
"$tulpa" runtime "$work/rt" --target sim >/dev/null
cat >"$work/start.c" <<'C'
#include "tulpa_hal.h"
int main(void) { tulpa_entry(); return 0; }
C
riscv64-unknown-elf-gcc -march=rv32imc_zicsr_zifencei -mabi=ilp32 -Os -std=c11 \
    --specs=picolibc.specs --oslib=semihost --crt0=semihost \
    -Wl,--defsym=__flash=0x80000000 -Wl,--defsym=__flash_size=0x100000 \
    -Wl,--defsym=__ram=0x80100000 -Wl,--defsym=__ram_size=0x100000 \
    -I "$work/rt" -I "$work" "$work/start.c" "$work/rt/tulpa_rt.c" "$work/rt/hal_sim.c" "$work/app.o" -lm \
    -o "$work/app.elf"
riscv64-unknown-elf-size "$work/app.elf" >&2
timeout 60 qemu-system-riscv32 -M virt -cpu rv32 -bios none -nographic -monitor none -serial none \
    -semihosting-config enable=on,target=native -kernel "$work/app.elf" >"$work/native.txt" 2>&1 || true
"$tulpa" run --virtual-time "$src" >"$work/interp.txt" 2>/dev/null || true
if diff -u "$work/interp.txt" "$work/native.txt"; then
    echo "rv32imc (qemu riscv32 virt): output matches the interpreter ($(wc -l <"$work/native.txt") lines)"
else
    echo "rv32imc (qemu riscv32 virt): output differs" >&2
    exit 1
fi
