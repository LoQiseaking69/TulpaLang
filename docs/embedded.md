# Native and embedded targets

`tulpa build` compiles a whole Tulpa program to native code through LLVM. Since 3.3 that covers
a lot of the language:

- structs (with invariants);
- devices;
- state machines;
- `always` monitors;
- `every!` loops;
- top-level code.

The output can be a standalone executable for Linux or the simulator, or an object file for
firmware on a microcontroller.

```
tulpa targets                                   # list the targets
tulpa build app.rav --target sim --exe -o app   # host program on the simulated bus
tulpa build app.rav --target linux --exe -o app # host program on real hardware (Pi, SBCs)
tulpa build app.rav --target rp2040 -o app.o    # object + app.h for the Pico SDK
tulpa new blinky --board esp32c3                # ready-to-build firmware project
tulpa runtime rt/ --target cortex-m4f           # C runtime + HAL template for your own build
tulpa bundle app.rav -o app                     # standalone executable with the whole language
tulpa deploy                                    # prove, check, build and flash a project
```

## Targets

| Target | Chip / platform | Toolchain for the final link |
|---|---|---|
| `sim` | host, simulated devices, virtual clock | clang (built in) |
| `linux` | host, gpiochip v2, i2c-dev, spidev, termios, sysfs PWM | clang (built in) |
| `rp2040` | Raspberry Pi Pico (Cortex-M0+) | Pico SDK 2.x |
| `esp32c3`, `esp32c6` | ESP32-C3/C6 (RISC-V) | ESP-IDF 5.x |
| `esp32`, `esp32s2`, `esp32s3` | ESP32 / S2 / S3 (Xtensa) | ESP-IDF 5.x, plus Espressif's clang |
| `cortex-m0`, `cortex-m4f`, `riscv32` | generic | your own startup code and HAL |

For the ESP32, S2 and S3, Tulpa needs Espressif's clang, because upstream LLVM has no
`-mcpu=esp32`. Install it with `python $IDF_PATH/tools/idf_tools.py install esp-clang`. `tulpa`
looks for it in `~/.espressif/tools/esp-clang` and `$IDF_TOOLS_PATH`, or you can set
`TULPA_CLANG`.

Embedded builds default to `-Os -ffunction-sections`. Pass `-O0` to `-O3` to override that.

## What compiles natively

**Values**
- `int`, `float`, `bool`, and quantities with units (stored as SI doubles; dimensions are
  checked at build time)
- `str` literals and parameters
- fixed-size arrays (bounds-checked, with negative indices counted from the end)

**Structs**
- fields
- defaults that can refer to earlier fields
- methods and static methods
- invariants, checked after construction, method calls and field writes
- `struct S impl Interface`
- parameters typed as an interface

**Devices**
- `GPIO`, `I2C`, `SPI`, `UART`, `PWM`, `Servo`, with the same methods and error messages as
  the interpreter

**State machines**
- states, events with parameters, guards, global transitions
- `m.state`, `m.send("event")`, `reset()`

**Control**
- `if` / `while` / `for` / `loop`
- `match` on numbers, ranges, alternatives (`1 | 2`), bindings and guards
- `error("...")`
- `every!(period[, count], { ... })` (with `break`), `repeat!`
- `require` / `ensure`
- `always cond [else { ... }]`, re-checked after every device call, field write, global write
  and transition

**Calls**
- default arguments, keyword arguments, calls between functions and methods
- `time::sleep/millis/micros`, math builtins, `implements()`

**Output**
- `print`, f-strings, `format(x, ".3f")`
- `q.to("unit")` while printing

**Modules**
- `from "std/drivers/mpu6050" import Mpu6050` and module aliases are flattened into one
  compilation unit

**Outside the static subset**
- dicts
- closures and lambdas
- `try` / `catch`
- tasks and signals
- `when` rules
- `match` with list patterns
- string building beyond printing

Every function that is skipped is listed with the reason. A program whose top level uses these
still deploys everywhere:

- on a microcontroller target, `tulpa build` puts the whole program on the
  [Tulpa VM for firmware](#the-whole-language-on-a-microcontroller);
- on hosted targets (`sim`, `linux`, or no `--target`), `--exe` falls back to a
  [bundle](#the-whole-language-in-one-executable).

## The whole language on a microcontroller

```
tulpa build examples/firmware_vm/main.rav --target rp2040 -o app.o
  <program> is outside the static native subset (...)
  dynamic tier: 3 module(s), 4197 nodes, 98089 bytes of T-code; runs on the Tulpa VM (tulpa_dyn.c)
```

When a firmware build's top-level program does not fit the static compiler, Tulpa compiles it
to T-code, a compact tree form of the program, and the object file carries that instead of
machine code. `tulpa_dyn.c`, the Tulpa VM for firmware, runs it. The VM is portable C11 with no
dependencies beyond the C library; `tulpa new` and `tulpa runtime` write it next to
`tulpa_rt.c`, the project templates already compile it, and the linker drops it from firmware
that does not need it (a static blinker for the Pico stays at 48 KB).

Everything in the language runs there: dicts, lists and strings (UTF-8), closures, `try` /
`catch`, `match` with every pattern, structs with invariants and operator overloading,
interfaces, state machines, signals, `when` / `always` / `after … within` rules, tasks,
macros, contracts, units of measure (including units built from strings at run time),
modules, the devices, `pack::` and `json::`, and the Tulpa-source standard library:
`std/plan`, `std/link`, `std/frame`, `std/driver` and its drivers, `std/world`,
`std/verify`, `std/lipm`, `std/topics`, `std/rtl`, and `std/emu` in builds for the simulated
board.

**Same behaviour, checked.** The VM is tested against the interpreter: the programs in
`tests/dyn/` and the examples must print the same output from both, on the host and under QEMU
as Cortex-M0 code with the RP2040's 256 KB of RAM and as RV32IMC (the ESP32-C3's instruction
set). `tulpa build --target sim --exe --dynamic app.rav -o app` builds any program onto the VM
on your computer, so you can run exactly what the board will run.

**Cost.**

| | Cortex-M0, `-Os` |
|---|---|
| VM code (`tulpa_dyn.c`) | 107 KB of flash |
| Whole firmware with the C library's float formatting and maths | about 200 KB of flash plus the program |
| Program | about 23 bytes of flash per syntax node (`std/plan` is 64 KB, `std/link` 28 KB); it is read in place and never copied to RAM |
| Fixed RAM | 38 KB (a 2048-slot value stack; `-DTULPA_DYN_STACK=N` changes it) |
| Heap | what the program's live objects need; garbage is collected at calls and loop iterations |

A Pico (2 MB flash, 264 KB RAM) runs a program that imports `std/plan` and `std/link` with room
to spare. It executes at roughly the speed of the desktop bytecode VM relative to the same
CPU, which is far slower than static native code: put a control loop that has to run at
kilohertz rates in a program that fits the static subset.

**Tuning** (compile definitions for `tulpa_dyn.c`):

| Definition | Default on a microcontroller | Meaning |
|---|---|---|
| `TULPA_DYN_STACK` | 2048 | value-stack slots, 16 bytes each |
| `TULPA_DYN_C_STACK` | 24576 (44000 in the Pico and ESP-IDF templates) | bytes of C stack the VM may use; past it a call fails with a catchable `stack overflow` error instead of corrupting memory. A nested Tulpa call costs about 600 bytes, so 24 KB allows about 40 levels. Give the thread a little more than this (the templates set `PICO_STACK_SIZE` and `CONFIG_ESP_MAIN_TASK_STACK_SIZE` to 48 KB) |
| `TULPA_DYN_GC_BYTES` | 12288 | allocation between garbage collections, at least |
| `TULPA_DYN_DEPTH` | 2500 | nested Tulpa calls (on a microcontroller the C stack runs out first) |

**What stays on the host.** A build stops with an error naming the line when a program uses:

- `nn::`, `cosim::`, `phone::`, the fieldbus modules (`BLE::`, `CAN::`, Modbus) and their
  `sim::` counterparts, `json::load` / `json::save`;
- `sim::` hooks in a build for real hardware. `sim::gpio_input`, `gpio_level`, `i2c_set`,
  `i2c_get`, `spi_respond`, `uart_feed`, `advance` and `backend` exist on the simulated board.

These fail at run time with a catchable error: `input()`, `read_file()` / `write_file()` /
`append_file()`, and `check()` (contract fuzzing belongs to `tulpa test` and `tulpa deploy`,
which run before the firmware is built). `define_test!` blocks are skipped, `dbg!` is silent,
and `tlm::` calls do nothing.

Other differences from the interpreter:

- A program is either compiled statically or run on the VM, as a whole. Mixing the two in one
  firmware image is not supported.
- The clock counts whole microseconds, where the interpreter's virtual clock is a float, so a
  timestamp printed after hundreds of 1 ms sleeps can differ by a millisecond.
- An uncaught error prints `tulpa: runtime error at line N: …` and calls `tulpa_hal_panic`.
- Runaway recursion ends in `stack overflow` or `value stack overflow` at a depth set by the
  board's memory, not at the interpreter's 2500 calls.
- `I2C::scan()` returns an empty list on the simulated board.

**Tulpa Link.** The VM contains a full Tulpa Link node (discovery, typed topics, reliable and
latched delivery, services, parameters), so `import "std/link"` works in firmware. See
[Tulpa Link on a microcontroller](link.md#on-a-microcontroller).

## The whole language in one executable

```
tulpa bundle app.rav -o app                      # this machine
tulpa bundle app.rav -o app --hw                 # use real hardware (Linux backend) by default
tulpa bundle app.rav -o app-pi --runtime tulpa-aarch64   # cross: a Tulpa binary built for the target
```

A bundle is the Tulpa runtime with your program and every module it imports (including `std/`)
packed inside one file. It needs nothing installed and runs every language feature: dicts,
closures, tasks, `std/plan`, `std/link`, `nn::`. Arguments after the executable's name go to
the program as `ARGS`.

Which path to use:

| Target | Program fits the native subset | Program uses dynamic features |
|---|---|---|
| Desktop, Raspberry Pi, other Linux | `tulpa build --exe` (static native code) | `tulpa bundle`, or `tulpa build --exe`, which falls back to a bundle |
| RP2040, ESP32, Cortex-M, RISC-V | `tulpa build --target <board>` (static native code) | the same command: the program runs on the [Tulpa VM for firmware](#the-whole-language-on-a-microcontroller) |

`tulpa deploy` runs the whole path for a `tulpa new` project: prove, check on the simulator,
build with the board SDK, flash. See [Simulation and verification](simulation.md).

**Display units.** The interpreter remembers each quantity's display unit, so `1[deg]` prints
as `1 deg`. Native code stores SI values, so it prints `0.0174533 rad` unless you write
`.to("deg")`. Values, comparisons and contract checks are identical either way.

## Firmware projects

`tulpa new <dir> --board <board>` writes three things:

- `app.rav`: a blinker state machine, with an `always` envelope and an `every!` loop. Try it
  first with `tulpa --virtual-time app.rav`.
- The build files for the board. Their build step calls `tulpa build --target <board>`.
- A copy of the runtime.

**RP2040**

```
tulpa new blinky --board rp2040
cd blinky
export PICO_SDK_PATH=~/pico-sdk
cmake -B build && cmake --build build      # → build/blinky.uf2
```

**ESP32-C3 and the other ESP chips**

```
tulpa new blinky --board esp32c3
cd blinky
. $IDF_PATH/export.sh
idf.py set-target esp32c3 && idf.py build flash monitor
```

**Linux (Raspberry Pi and other boards)**

```
tulpa new blinky --board linux && cd blinky && make && sudo ./blinky
```

The program's `print` output goes to the board's console: USB/UART stdio on the Pico, the IDF
console on ESP32, stdout on Linux. When a contract, invariant or `always` monitor is violated,
the program calls `tulpa_hal_panic(msg)`. That function prints the message and halts the board
or resets it. Define your own `tulpa_violation(const char *msg)` to run an emergency stop
instead.

## Calling Tulpa from C

Every compiled function is exported as `tulpa_<name>` and every method as
`tulpa_<Struct>_<method>`. `app.h` declares them, together with the C layout of each struct
(`tulpa_<Struct>`). Quantity parameters are passed in the declared unit: a function taking
`v: [km/h]` takes km/h from C.

If the program has top-level code, it becomes `void tulpa_entry(void)`, which the board's
`main.c` calls.

## Runtime layers

```
app.o (generated)  ─calls→  tulpa_* ABI      (runtime/tulpa_rt.c: checks, printing, every!)
                           ─calls→  tulpa_hal_*   (one port per board)
```

| Port | File | Backed by |
|---|---|---|
| sim | `hal/sim.c` | virtual clock, I2C register files, SPI response queue |
| linux | `hal/linux.c` | `/dev/gpiochipN` (v2 uAPI, `TULPA_GPIOCHIP`), `/dev/i2c-N`, `/dev/spidevB.C`, termios, sysfs PWM |
| pico | `hal/pico.c` | Pico SDK `hardware_*` |
| esp_idf | `hal/esp_idf.c` | IDF 5.x `i2c_master`, `spi_master`, LEDC, UART, `esp_timer` |

## Porting to a new board

`tulpa runtime <dir> --target cortex-m4f` writes `tulpa_hal.h`, `tulpa_rt.c` and
`hal_template.c`. The template starts as a copy of the simulator port, which implements every
function. Replace each function's body with your board's code. There are roughly 20 of them:

```c
int tulpa_hal_gpio_init(tulpa_gpio_t *g);
int tulpa_hal_gpio_write(tulpa_gpio_t *g, int level);
int tulpa_hal_gpio_read(tulpa_gpio_t *g);
int tulpa_hal_i2c_write_read(tulpa_i2c_t *d, const uint8_t *w, size_t wn, uint8_t *r, size_t rn);
int tulpa_hal_spi_transfer(tulpa_spi_t *d, const uint8_t *tx, uint8_t *rx, size_t n);
int tulpa_hal_pwm_apply(tulpa_pwm_t *p);
uint64_t tulpa_hal_micros(void);
void tulpa_hal_delay_us(uint64_t us);
void tulpa_hal_console_write(const char *s, size_t n);
void tulpa_hal_panic(const char *msg);
/* ... */
```

Each device struct has a `void *hw` slot for the port's own handle. The functions return 0 on
success. When one returns an error, the runtime raises the same error message the interpreter
would. `TULPA_MAX_XFER` sets the largest bus transfer (the default is 256 bytes).

## How it is verified

- **Simulator.** `cargo test` builds `examples/native`, `examples/drivers` and the v3.3 test
  programs with `--target sim --exe`, and compares their output byte for byte with the
  interpreter's.
- **Cortex-M0.** `tests/firmware/run_qemu_m0.sh prog.rav` cross-compiles a program for
  Cortex-M0 and runs it on an emulated BBC micro:bit (`qemu-system-arm -M microbit`, newlib
  semihosting), then diffs the output with the interpreter's.
- **RISC-V.** `tests/firmware/run_qemu_riscv32.sh` does the same for the esp32c3 code on
  `qemu-system-riscv32 -M virt`.
- **Board builds.** CI builds the `rp2040` scaffold with the Pico SDK and checks that each
  firmware target produces an object file.
