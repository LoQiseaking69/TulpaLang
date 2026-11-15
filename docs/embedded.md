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

**Not native (yet)**
- dicts and `match`
- closures and lambdas
- `try` / `catch`
- tasks and signals
- `when` rules
- string building beyond printing

Every function that is skipped is listed with the reason. `--exe` and firmware builds need the
top-level program itself to compile.

**Display units.** The interpreter remembers each quantity's display unit, so `1[deg]` prints
as `1 deg`. Native code stores SI values, so it prints `0.0174533 rad` unless you write
`.to("deg")`. Values, comparisons and contract checks are identical either way.

## Firmware projects

`tulpa new <dir> --board <board>` writes three things:

- `app.rav`: a blinker state machine, with an `always` envelope and an `every!` loop. Try it
  first with `tulpa app.rav --virtual-time`.
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
