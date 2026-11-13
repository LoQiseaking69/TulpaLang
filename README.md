# Tulpa

A Python-flavoured, Rust-powered language for **robots and hardware**. Physical units,
specifications, state machines and safety rules are part of the language itself, not libraries.

___
![img](https://github.com/LoQiseaking69/TulpaLang/blob/main/TulpaLangLOGO.png)
___

```tulpa
struct Joint {
    servo
    angle: [deg] = 90[deg]
    invariant 10[deg] <= self.angle and self.angle <= 170[deg]
    fn move_to(self, a: [deg]) { self.angle = a; self.servo.angle(a) }
}

let lift = Joint(Servo::new(0, 0))
let conveyor = GPIO::new(23)
let load = signal(0[kg])

machine Cell {
    on estop => Fault { conveyor.set_low() }
    state Idle    { enter { lift.move_to(150[deg]); conveyor.set_high() }
                    on part => Picking }
    state Picking { enter { lift.move_to(30[deg]) }
                    on gripped(m) if m <= 2[kg] => Idle
                    on gripped(m) => Fault }
    state Fault   { on reset => Idle }
}
let cell = Cell()

when load.get() > 0[kg] { cell.gripped(load.get()) }               # reactive rule
always not (conveyor.read() == 1 and lift.angle < 60[deg]) else {  # safety envelope
    cell.estop()
}

fn stopping_distance(v: [m/s], decel: [m/s^2]) -> [m]
    require v >= 0 and decel > 0
    ensure result' >= 0
{ v ** 2 / (2 * decel) }
```

## What makes it different

| Feature | What it gives you |
|---|---|
| **Units of measure:** `0.5[m/s]`, `90[deg]`, `2200[mAh]` | Every value carries its dimension, so `speed + distance` is an error, not a silent bug. Conversions are built in: `.to("km/h")`, `.in("rad")`. |
| **Runtime-checked types:** `fn f(v: [m/s], n: int?) -> [m]` | Gradual typing. Annotate where you want it and the interpreter enforces it, unit dimensions included. |
| **Hehner-style specs:** `require`, `ensure` with `x'`/`result'`, `invariant` | Functions, loops and structs carry their own pre- and post-conditions. |
| **Contract fuzzing:** `check(f)`, `tulpa verify` | Your `ensure` clauses double as test oracles. Tulpa generates inputs that satisfy each `require`, hunts for violations, and shrinks any failure to a minimal counterexample. |
| **State machines:** `machine` / `state` / `on ev(args) if guard => Target { action }` | Robot behaviour as data: enter/exit hooks, global transitions such as e-stop, history, `.can()`, and Mermaid diagrams via `tulpa graph`. |
| **Reactive signals:** `signal()`, `derive()`, `when cond { } else { }` | Edge-triggered rules that re-evaluate when signals, variables or devices change. |
| **Timing rules:** `when cond for 25[ms] { }`, `after trigger within 300[ms] ensure response else { }` | Debouncing and bounded-response deadlines are language constructs. A virtual clock (`--virtual-time`) makes timing logic testable in milliseconds of wall time. |
| **Safety envelopes:** `always cond [else { handler }]` | Monitors checked after every device call, assignment, transition and control tick. Without a handler a violation stops the program; with one, the handler runs. |
| **Record & replay:** `--record run.tape`, `--replay run.tape` | Capture a real hardware session with timestamps, then rerun it in CI with no hardware, in milliseconds.<br>- Inputs are replayed as signals over time; commands must match in order.<br>- Timing rules are judged against the recorded timing. |
| **Structs:** fields, defaults, methods, `invariant`, operator overloading | Class invariants are re-checked after every method call and field write, and a write that would break one is rolled back. |
| **Devices:** GPIO, I2C (+ `scan`), SPI, UART, PWM, Servo, PID, **CAN** (SocketCAN), **Bluetooth LE** (GATT client + scan), `kin::` | A deterministic simulator by default; the BLE simulator is a real GATT server.<br>`--hw` switches to real Linux hardware: GPIO via `/dev/gpiochipN` with a sysfs fallback, `can0`, `hci0`. |
| **Three execution tiers** | A tree-walking interpreter, a bytecode VM that hot functions use automatically (4–5× faster), and `tulpa build` for LLVM-compiled native code with a C header.<br>- Unit dimensions are checked at compile time, and C passes values in the declared units.<br>- Contracts are kept as checks in native code too. |
| **Editor support:** VS Code extension + `tulpa lsp` | Diagnostics, including constant unit mismatches and undefined names with suggestions; hover with signatures, contracts and unit dimensions; completion, including unit names inside `[...]`; symbols; go-to-definition. |

It also has everything you'd expect from a general-purpose scripting language:
- f-strings, `match` with guards and list patterns
- closures, keyword arguments, the `|>` pipe
- `import` modules, `try`/`catch`, `const`
- `spawn`/`await` tasks, macros (`every!` fixed-rate loops, `define_macro!`)
- a REPL, and a test runner

## Libraries (3.2)

Native modules: `nn::` (tensors + autodiff, MLP, Adam), `pack::` (binary layouts, CRC, COBS),
`json::`, `tlm::` (live telemetry) and `cosim::` (lock-step physics co-simulation).
The `std/` library is written in Tulpa: `std/rtl` (cycle-accurate RTL + VCD), `std/eda` (boards
as code: ERC, power budgets, netlist/BOM), `std/rl` (PPO), `std/lipm` (capture point/ZMP),
`std/topics` (ROS-style pub/sub) and `std/frame` (COBS+CRC packet framing).
See the [library guide](docs/libraries.md).

The [HermiSim](https://github.com/LoQiseaking69/HermiSim-DynamicsViewer) IDE bundles Tulpa with
a MuJoCo co-simulator, waveform viewer and live plots. Its Morty humanoid project is written
entirely in Tulpa.

## Install

Download a release package. Release builds come from the `release` workflow on every `v*` tag.
- Linux (x86_64, arm64 Raspberry Pi 3/4/5, armv7): `tulpa-<version>-<target>.tar.gz`. Unpack it and run `./install.sh`, or `sudo ./install.sh /usr/local`.
- Debian/Ubuntu/Raspberry Pi OS: `sudo apt install ./tulpa_<version>_<arch>.deb`
- VS Code: `code --install-extension tulpa-lang-<version>.vsix`

Or build from source with `cargo build --release` (Rust 1.75+). `scripts/release.sh` builds all the packages for the host.

## Quick start

```bash
cargo build --release
T=./target/release/tulpa
$T examples/warehouse_robot/main.rav        # machine + signals + safety envelope + servos
$T examples/units/main.rav                  # dimensional analysis
$T verify examples/verify/main.rav          # fuzz every contracted function
$T --virtual-time examples/timing/main.rav  # debounce + deadlines on a simulated clock
$T build examples/native/main.rav -o ctl.o  # native object + ctl.h for firmware (needs clang)
$T bytecode examples/units/main.rav         # see what the VM compiles
$T examples/fieldbus/main.rav               # CAN motor controller + BLE battery gauge
$T graph examples/warehouse_robot/main.rav  # Mermaid state diagrams
$T --record run.tape robot.rav              # capture device traffic...
$T --replay run.tape robot.rav              # ...and regression-test against it
$T test tests/tulpa/std_test.rav            # define_test! blocks
$T                                          # REPL
sudo $T --hw robot.rav                      # real hardware on a Pi
```

Install the binary with `scripts/deploy.sh [PREFIX]`. Cross-compile for a Raspberry Pi with `scripts/cross_compile.sh`.

## Best use cases

1. **Robot cells and mechatronics.** Each part of the language does one job:
   - State machines hold the behaviour.
   - `when` rules glue sensors to events.
   - `always` envelopes encode the "never do this" rules.
   - Units keep the physics honest.
2. **Hardware regression testing.** Record a session on the real rig once, then replay it on every commit. No hardware is needed in CI.
3. **Safety-critical glue code.** Contracts, struct invariants and runtime-checked unit types catch bad commands before they reach an actuator. `tulpa verify` shows a contract can fail before the hardware does.
4. **Sensor and actuator bring-up.** Probe the bus with `I2C::scan()`, work through registers interactively in the REPL, and watch every transaction with `--trace`.
5. **Teaching control and embedded systems.** Python-like syntax, a simulator that can't break anything, PID and kinematics built in, and unit errors explained in plain words.

## Docs

- [Language guide](docs/language_guide.md): syntax, types, units, structs, machines, reactive rules, verification, modules and the standard library.
- [Library guide](docs/libraries.md): `nn`, `pack`, `json`, `tlm`, `cosim` and the `std/` library.
- [Device integration guide](docs/device_integration.md): devices, backends, the simulator, record and replay, and wiring notes.

## Project layout

```
src/lexer.rs       tokens: comments, f-strings, hex/bin literals, primes x', unit brackets
src/parser.rs      Pratt parser → AST (src/ast.rs): structs, machines, when/always, import, |>
src/interp.rs      evaluator core: scopes, closures, contracts, macros, tasks
src/ext.rs         2.0 features: units math, kwargs, types, structs, machines, signals,
                   reactive rules, safety monitors, modules, contract fuzzing
src/vm.rs          bytecode compiler + VM (automatic tier for eligible functions)
src/llvm.rs        LLVM IR generator for the typed numeric subset (tulpa build / emit-llvm)
src/lsp.rs         language server (tulpa lsp)
src/units.rs       dimensional analysis (SI base vector, unit parser, conversions)
src/builtins.rs    standard library, kin:: module, string/list/dict methods, formatting
src/nn.rs          nn:: tensors, reverse-mode autodiff, layers, optimisers
src/stdx.rs        pack:: json:: tlm:: cosim::
stdlib/            std/ library written in Tulpa (rtl, eda, rl, lipm, topics, frame)
src/devices/       GPIO/I2C/SPI/UART/PWM/Servo/PID; sim.rs, linux.rs, tape.rs (record/replay)
examples/          runnable programs with expected.txt (checked by `cargo test`)
tests/             78 Rust tests (run under both tiers) + Tulpa-level tests
editors/           VS Code extension (editors/vscode) + Neovim/Helix setup
scripts/release.sh builds tarball, .deb and .vsix for the host
```

## Performance

| Benchmark (release build) | Interpreter | VM (default) | Native (`tulpa build`) |
|---|---|---|---|
| `fib(30)`, call-heavy | 1.15 s | 0.28 s | 0.005 s |
| 1M-iteration loop + 1M-step PID simulation | 1.39 s | 0.28 s | — |

Every test runs under both the interpreter and the VM (`TULPA_NO_VM=1`), and native output is
differential-tested against the interpreter.

## Roadmap

Every roadmap item so far has shipped:
- 2.0: the bytecode VM, LLVM codegen, the gpiochip backend, timing rules, and the LSP.
- 3.1: native unit-typed code, the VS Code extension, CAN and BLE, and timed replay.

Ideas for what's next:
- CAN-FD and ISO-TP (UDS diagnostics) on top of the CAN layer.
- BLE peripheral (GATT server) mode, so a Pi can act as a sensor.
- Native code for structs and state machines.
- A package registry for Tulpa modules.

## License

BSD 3-Clause. See `LICENSE`.
