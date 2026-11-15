# CLI reference

Everything in Tulpa goes through one binary, `tulpa`. With no arguments it starts the REPL.

## Commands

| Command | What it does |
|---|---|
| `tulpa <file> [args...]` | Run a program. Extra arguments are available as `ARGS`. `tulpa run <file>` is the same. |
| `tulpa repl` | Interactive prompt (also the default with no file). Commands: `:log`, `:reset`, `:quit`. |
| `tulpa test <file>...` | Run `define_test!` blocks. |
| `tulpa check <file>...` | Parse only and report syntax errors. |
| `tulpa ast <file>` | Dump the parsed syntax tree. |
| `tulpa verify [--trials=N] <file>...` | Fuzz every contracted function against its own `require` / `ensure`. |
| `tulpa graph <file>...` | Print Mermaid `stateDiagram-v2` source for every `machine`. |
| `tulpa bytecode <file>` | Show the VM bytecode generated for each eligible function. |
| `tulpa emit-llvm <file> [--target T] [-o out.ll]` | Print the LLVM IR for a program. |
| `tulpa build <file> [--target T] [-o out] [--exe] [--shared] [--sim] [-O0..3]` | Compile natively. Writes an object file plus a C header, a standalone executable (`--exe`, for targets `sim` and `linux`), or a shared library. Needs clang 14+. See [Native & embedded targets](embedded.html). |
| `tulpa targets` | List the native targets: `sim`, `linux`, `rp2040`, `esp32`, `esp32s2`, `esp32s3`, `esp32c3`, `esp32c6`, `cortex-m0`, `cortex-m4f`, `riscv32`. |
| `tulpa new <dir> --board B` | Scaffold a firmware project for a board (Pico SDK, ESP-IDF, or a Linux Makefile). |
| `tulpa runtime <dir> [--target T]` | Write the C runtime and HAL port, for use in your own build system. |
| `tulpa lsp` | Language server over stdio, for editors. |

## Flags

| Flag | Effect |
|---|---|
| `--sim` | Use the in-memory device simulator (the default). |
| `--hw` | Use real Linux hardware: gpiochip / sysfs GPIO, `/dev/i2c-*`, `/dev/spidev*`, TTYs, SocketCAN, `hci0`. Same as `TULPA_BACKEND=linux`. |
| `--trace` | Log every device call and its result to stderr. |
| `--record F` | Record all device traffic, with timestamps, to tape file `F`. |
| `--replay F` | Answer device calls from tape `F` and fail on the first divergence. |
| `--virtual-time` | Simulated clock: `sleep` and `every!` ticks advance instantly. Use it to test timing rules. |
| `--no-vm` | Run everything on the tree-walking interpreter (no bytecode tier). Same as `TULPA_NO_VM=1`. |
| `-V`, `--version` | Print the version. |
| `-h`, `--help` | Show usage. |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | OK |
| `1` | Tests failed |
| `2` | Usage error |
| `65` | Syntax error |
| `70` | Runtime error |

## Environment variables

| Variable | Used by | Purpose |
|---|---|---|
| `TULPA_BACKEND` | `tulpa` | `linux` selects the hardware backend, like `--hw`. |
| `TULPA_NO_VM` | `tulpa` | `1` disables the bytecode VM. |
| `TULPA_PATH` | `tulpa`, HermiSim | Extra library search path for `import`. |
| `TULPA_TELEMETRY` | `tlm::` | `1` makes `tlm::*` calls emit `@@tlm {json}` lines. HermiSim sets it. |
| `TULPA_COSIM` | `cosim::connect()` | Address of the co-simulation server. HermiSim sets it. |
| `TULPA_BIN` | HermiSim | Path of the `tulpa` binary to use. |
| `TULPA_CLANG` | `tulpa build` | The clang binary to use. ESP32, S2 and S3 builds need Espressif's clang, which is found automatically in `~/.espressif`. |
| `TULPA_GPIOCHIP` | native `linux` target | The gpiochip device to use (default `/dev/gpiochip0`). |

## Recipes

```bash
tulpa examples/warehouse_robot/main.rav        # machine + signals + safety envelope + servos
tulpa --virtual-time examples/timing/main.rav  # debounce + deadlines on a simulated clock
tulpa build examples/native/main.rav -o ctl.o  # ctl.o + ctl.h for firmware
tulpa build examples/drivers/main.rav --target sim --exe -o drv && ./drv   # native == interpreter
tulpa new rover --board rp2040                 # Pico SDK project → build/rover.uf2
tulpa new rover --board esp32c3                # ESP-IDF project → idf.py build flash
tulpa --record run.tape robot.rav              # capture device traffic on the rig...
tulpa --replay run.tape robot.rav              # ...and regression-test against it in CI
sudo tulpa --hw robot.rav                      # real hardware on a Raspberry Pi
```
