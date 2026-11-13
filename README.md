# Tulpa

**A Python-flavoured, Rust-powered programming language for robotics, embedded systems, and physical computing.**

Tulpa builds physical units, engineering specifications, state machines, reactive programming,
timing constraints, and safety monitoring into the language itself, not into external libraries.

It is designed around a central idea: **software that interacts with the physical world should
be able to express the rules of that world.**

Dimensional correctness, actuator constraints, state transitions, response deadlines, and
hardware behaviour are all part of the programming model. The result is a language with the
accessibility of Python-style scripting, plus systems-oriented execution, engineering-aware
abstractions, and native hardware integration.

---

![Tulpa](TulpaLangLOGO.png)

---

## At a glance

| Characteristic | Description |
|---|---|
| Language identity | Python-flavoured, expression-oriented programming language |
| Implementation | Rust |
| Primary domain | Robotics, mechatronics, embedded systems, physical computing |
| Execution | Tree-walking interpreter, bytecode VM, LLVM native compilation |
| Engineering model | Units, contracts, invariants, state machines, reactive rules, timing constraints |
| Hardware integration | GPIO, I2C, SPI, UART, PWM, Servo, PID, CAN, Bluetooth LE |
| Verification | Runtime contracts, contract fuzzing, safety monitors, record/replay |
| Developer tooling | REPL, test runner, LSP, VS Code extension, Mermaid state diagrams, [HermiSim IDE](https://github.com/LoQiseaking69/HermiSim-DynamicsViewer) |
| License | BSD 3-Clause |

## A language designed around physical systems

Most general-purpose languages can control hardware. Their engineering constraints, though,
usually live in libraries, conventions, or application-specific frameworks.

Tulpa makes many of those constraints first-class language constructs:

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

    state Idle {
        enter { lift.move_to(150[deg]); conveyor.set_high() }
        on part => Picking { conveyor.set_low() }   # stop the belt before the arm drops
    }

    state Picking {
        enter { lift.move_to(30[deg]) }
        on gripped(m) if m <= 2[kg] => Idle
        on gripped(m) => Fault
    }

    state Fault {
        on reset => Idle
    }
}

let cell = Cell()

when load.get() > 0[kg] {                                       # reactive rule
    cell.gripped(load.get())
}

always not (conveyor.read() == 1 and lift.angle < 60[deg]) else {  # safety envelope
    cell.estop()
}

fn stopping_distance(v: [m/s], decel: [m/s^2]) -> [m]
    require v >= 0 and decel > 0
    ensure result' >= 0
{
    v ** 2 / (2 * decel)
}
```

This example combines several of Tulpa's defining capabilities:

- **Dimensional analysis:** physical quantities carry units and enforce dimensional compatibility.
- **Engineering specifications:** functions and structures express preconditions, postconditions, and invariants.
- **State-driven behaviour:** machine states, events, guards, and transitions define operational behaviour.
- **Reactive control:** changes in sensors and signals can trigger application logic.
- **Safety monitoring:** continuous conditions can be expressed as executable constraints. If the
  `Picking` transition left the belt running, the `always` rule above would catch it the moment
  the lift drops below 60° and fire `estop`.
- **Hardware abstraction:** the same program runs against simulated or physical devices.

These are not separate frameworks loosely connected by application code. They are integrated
into the language's syntax, AST, runtime, tooling, and execution infrastructure. The full
pick-and-place cell is in [`examples/warehouse_robot`](examples/warehouse_robot/main.rav).

## What makes Tulpa different

Tulpa combines concepts found in scientific programming, formal specification languages,
reactive systems, PLC environments, and compiled systems languages. Individual concepts have
precedents. What sets Tulpa apart is integrating them into one accessible language aimed at
physical systems.

| Capability | Syntax | Engineering value |
|---|---|---|
| **Units of measure** | `0.5[m/s]`, `90[deg]`, `2200[mAh]` | Physical quantities carry dimensions, so adding speed to distance is a dimensional error rather than a silent bug. Conversions are built in: `.to("km/h")`, `.in("rad")`. |
| **Runtime-checked types** | `fn f(v: [m/s], n: int?) -> [m]` | Gradual typing. Annotate where needed and the interpreter enforces types, including unit dimensions. |
| **Hehner-style specifications** | `require`, `ensure`, `x'`, `result'`, `invariant` | Functions, loops, and structures express preconditions, postconditions, and invariants as executable specifications. |
| **Contract fuzzing** | `check(f)`, `tulpa verify` | Generates inputs that satisfy declared preconditions, tests postconditions, and shrinks failures to minimal counterexamples. Contracts become test oracles. |
| **Native state machines** | `machine`, `state`, `on ev(args) if guard => Target { action }` | Robot behaviour is explicit: states, guarded transitions, `enter`/`exit` hooks, global transitions, history, and `.can()`. `tulpa graph` generates Mermaid diagrams. |
| **Reactive signals** | `signal()`, `derive()`, `when cond { } else { }` | Reactive rules re-evaluate when signals, variables, or devices change. Edge-triggered. |
| **Timing rules** | `when cond for 25[ms]`, `after trigger within 300[ms] ensure response else { }` | Debouncing and bounded-response deadlines are language constructs. `--virtual-time` provides a simulated clock for testing timing behaviour. |
| **Safety envelopes** | `always cond [else { handler }]` | Runtime monitors are checked after device calls, assignments, transitions, and control ticks. A violation either stops execution or runs a handler. |
| **Record and replay** | `--record run.tape`, `--replay run.tape` | Captures hardware sessions with timestamps for regression testing without the hardware. Inputs replay over time, commands must match in order, and timing rules run against the recorded clock. |
| **Invariant-aware structures** | Fields, defaults, methods, invariants, operator overloading | Structure invariants are rechecked after method calls and field writes. A write that would violate an invariant is rolled back. |
| **Hardware integration** | GPIO, I2C, SPI, UART, PWM, Servo, PID, CAN, Bluetooth LE, `kin::` | Deterministic simulation by default; Linux hardware with `--hw`. GPIO goes through `/dev/gpiochipN` with a sysfs fallback, CAN through SocketCAN, and Bluetooth LE through `hci0`. |
| **Multiple execution tiers** | Interpreter, bytecode VM, LLVM native compilation | Interactive execution, automatic VM execution of eligible functions, and native compilation of the typed numeric subset. |
| **Editor tooling** | VS Code extension, `tulpa lsp` | Diagnostics, unit-mismatch detection, undefined-name suggestions, hover, completions, symbols, and go-to-definition. Neovim and Helix setups are in [`editors/`](editors/README.md). |

## The architectural distinction

Tulpa brings together programming paradigms that are normally found in separate tools and
ecosystems.

| Programming discipline | Related established technologies | Tulpa implementation |
|---|---|---|
| Dimensional programming | Scientific computing, F#, Modelica | Units embedded in expressions; runtime type enforcement |
| Design by contract | Eiffel, SPARK, Dafny | Executable preconditions, postconditions, and invariants; contract fuzzing |
| State-based systems | UML Statecharts, SCXML | Native state-machine syntax and runtime |
| Reactive programming | ReactiveX, Lustre | Signals, derived values, reactive rules, event-triggered execution |
| Industrial control | IEC 61131-3, PLC environments | Device abstractions, state-driven logic, PID, timing constraints |
| Systems compilation | Rust, LLVM-based toolchains | Bytecode execution and LLVM native code generation |
| Hardware simulation | Hardware-in-the-loop, digital-twin workflows | Device simulation, record/replay, lock-step co-simulation |

Tulpa does not try to replace every language or framework in these disciplines. It aims to give
one programming environment where the relevant concepts from each can be used together.

## Execution architecture

Tulpa has three execution tiers:

1. **Tree-walking interpreter:** runs the whole language and backs the REPL.
2. **Bytecode VM:** eligible function bodies compile to bytecode on their first call and run
   automatically on a stack VM. In the benchmarks below, hot functions run about 4–5× faster.
   Constructs the VM doesn't cover fall back to the interpreter transparently.
3. **LLVM native compilation:** `tulpa build` emits textual LLVM IR for the typed numeric subset
   and compiles it with clang 14+ into an object file or shared library, plus a C header for
   firmware integration. Unit dimensions are checked at build time. Contracts stay in the
   generated code as runtime checks.

The interpreter and VM run the whole language. LLVM is the path to native performance and C/C++/Rust
interoperability for supported numeric workloads. Structs, lists, strings, state machines,
signals, and device or library calls do not compile to native code yet.

## Verification and safety model

Tulpa layers several mechanisms to catch errors before they reach hardware:

- Type and dimensional checks
- Preconditions and postconditions
- Structure invariants
- Contract fuzzing and counterexample shrinking
- Runtime safety envelopes
- Deterministic simulation
- Recording and replaying hardware sessions
- Differential testing between execution tiers

These mechanisms improve software verification and regression testing. They are not a formal
proof of the complete system, nor certification as a safety-rated controller. Applications that
need functional-safety certification still require an appropriate independent safety
architecture, hardware protections, and a certification process.

## General-purpose language features

Although Tulpa is specialized for physical systems, it has the features expected of a
general-purpose scripting language:

- f-strings
- `match` with guards and list patterns (`[first, ..rest]`)
- Closures and keyword arguments
- The `|>` pipe operator
- Module imports, with a library search path for `std/`
- `try` / `catch`
- `const`
- `spawn` / `await` tasks
- Macros, including `every!` fixed-rate loops and `define_macro!`
- Interactive REPL
- Built-in test runner (`define_test!`, `tulpa test`)

## Libraries (3.2)

Tulpa ships native modules plus a standard library written in Tulpa.

### Native modules

| Module | Functionality |
|---|---|
| `nn::` | Tensors, reverse-mode automatic differentiation, multilayer perceptrons, Adam/SGD, running scalers |
| `pack::` | Binary layouts (struct-style), `f32` arrays, CRC-16/32, COBS |
| `json::` | JSON encode/decode and files |
| `tlm::` | Live telemetry for plots and events |
| `cosim::` | Lock-step physics co-simulation over TCP |

### Standard library

The `std/` library provides higher-level engineering functionality implemented in Tulpa:

| Library | Functionality |
|---|---|
| `std/rtl` | Cycle-accurate RTL simulation and VCD waveform output |
| `std/eda` | Board-as-code: electrical rule checking, power budgets, netlists, and BOM generation |
| `std/rl` | Proximal Policy Optimization (PPO) on top of `nn::` |
| `std/lipm` | Linear inverted pendulum: capture point and Zero Moment Point (ZMP) |
| `std/topics` | ROS-style publish/subscribe messaging |
| `std/frame` | COBS + CRC packet framing |

See the [library guide](docs/libraries.md).

## HermiSim integration

The [HermiSim](https://github.com/LoQiseaking69/HermiSim-DynamicsViewer) IDE bundles Tulpa with:
- a MuJoCo co-simulator
- a waveform viewer
- state-machine diagrams
- live plots

It also has a Windows installer.

Its Morty humanoid project is written entirely in Tulpa. It shows the language working inside a
larger robotics and simulation environment:
- balance control
- a physics-informed model
- PPO
- an RTL torque governor
- the mainboard as code

## Install

Download a release package from [Releases](https://github.com/LoQiseaking69/TulpaLang/releases).
The `release` workflow builds them on every `v*` tag. The latest published release is
**v3.1.1**. Until a `v3.2.0` tag is pushed, build from source to get the 3.2 libraries, or use
the toolchain bundled with HermiSim.

- **Linux** (x86_64, arm64 for Raspberry Pi 3/4/5, armv7): download
  `tulpa-<version>-<target>.tar.gz`, unpack it, and run `./install.sh`, or
  `sudo ./install.sh /usr/local`.
- **Debian / Ubuntu / Raspberry Pi OS:** `sudo apt install ./tulpa_<version>_<arch>.deb`
- **macOS:** `tulpa-<version>-<x86_64|aarch64>-apple-darwin.tar.gz`
- **Windows:** use the [HermiSim installer](https://github.com/LoQiseaking69/HermiSim-DynamicsViewer),
  which bundles `tulpa.exe` and the `std/` library, or build from source.
- **VS Code:** `code --install-extension tulpa-lang-<version>.vsix`. The `.vsix` is attached to
  each release. The extension is not on the VS Code Marketplace yet.

Build from source:

```bash
cargo build --release
```

This requires Rust 1.82 or later. `scripts/release.sh` builds all the packages for the host.

## Quick start

```bash
cargo build --release
T=./target/release/tulpa

$T examples/warehouse_robot/main.rav        # machine + signals + safety envelope + servos
$T examples/units/main.rav                  # dimensional analysis
$T verify examples/verify/main.rav          # fuzz every contracted function
$T --virtual-time examples/timing/main.rav  # debounce + deadlines on a simulated clock
$T build examples/native/main.rav -o ctl.o  # native object + ctl.h for firmware (needs clang 14+)
$T bytecode examples/units/main.rav         # inspect VM compilation
$T examples/fieldbus/main.rav               # CAN motor controller + BLE battery gauge
$T graph examples/warehouse_robot/main.rav  # Mermaid state diagrams
$T --record run.tape robot.rav              # capture device traffic
$T --replay run.tape robot.rav              # regression-test recorded traffic
$T test tests/tulpa/std_test.rav            # define_test! blocks
$T                                          # REPL
sudo $T --hw robot.rav                      # real hardware on a Pi
```

To install the binary, run `scripts/deploy.sh [PREFIX]`. To cross-compile for a Raspberry Pi, run
`scripts/cross_compile.sh`.

## Best use cases

### 1. Robotics and mechatronics
Tulpa's language constructs map directly onto what physical systems need:
- State machines represent operational behaviour.
- Reactive rules connect sensor conditions to events.
- Safety envelopes express conditions that must always hold.
- Units preserve dimensional correctness.
- Device abstractions connect logic to actuators and sensors.
- Simulation supports development before deployment.

### 2. Hardware regression testing
Record a session on a real device, then replay it during later development and CI runs. That way
software changes are tested against recorded hardware interactions without needing the physical
rig for every test.

### 3. Safety-oriented control logic
Contracts, structure invariants, dimensional checks, and runtime monitors catch invalid
operations and unexpected states. `tulpa verify` searches for contract violations before
deployment. These mechanisms complement certified safety systems and independent hardware
safeguards; they do not replace them.

### 4. Sensor and actuator bring-up
Probe hardware buses with `I2C::scan()`, inspect registers from the REPL, and watch device
transactions with `--trace`.

### 5. Embedded systems and control education
Python-like syntax, simulated hardware, built-in PID and kinematics, and readable dimensional
errors make a practical environment for learning programming, control theory, and hardware
integration.

### 6. Engineering simulation and experimentation
Together, `nn::`, `cosim::`, `std/rtl`, `std/eda`, and `std/rl` support experiments across
numerical computation, simulation, hardware design, and control development.

## Design philosophy and originality

Tulpa's originality is not a claim that state machines, dimensional analysis, contracts, or
reactive programming are new. Each is an established idea with a long history across
programming languages and engineering disciplines.

What distinguishes Tulpa is making them cooperate as native parts of one language:

- A physical quantity carries dimensional information through an expression.
- A function specifies the conditions under which it is valid.
- A structure enforces its own engineering invariants.
- A state machine represents the lifecycle of a physical process.
- Reactive rules connect changing inputs to state transitions.
- Timing constructs express bounded-response requirements.
- Safety monitors enforce runtime conditions.
- Recorded hardware sessions become repeatable regression tests.
- Eligible numeric workloads move from interpretation to native compilation.

The result is a programming model for software whose correctness depends on physical dimensions,
device state, timing, and operational constraints, not just computed output.

Tulpa is best understood as an engineering-oriented language for robotics and physical systems.
It is not simply a Python alternative or a hardware-control library.

## Current scope

Tulpa already has a substantial language implementation, runtime, tooling, hardware abstraction
layer, and engineering library ecosystem. Its next stage is about expanding execution coverage,
strengthening system-level guarantees, and improving adoption.

Some important boundaries of its current scope:

- LLVM compilation supports a typed numeric subset, not the whole language.
- Contract fuzzing explores test cases; it is not a mathematical proof of correctness.
- Runtime safety envelopes detect violations at defined checkpoints. They are not independent
  hardware safety interlocks.
- Record/replay validates recorded software interactions and timing. It does not establish
  equivalence between a simulation and the physical world.
- Whether Tulpa suits production depends on the target hardware, workload, timing requirements,
  testing, and safety architecture.

These boundaries separate an ambitious engineering language implementation from a fully
validated industrial control platform.

## Docs

- [Language guide](docs/language_guide.md): syntax, types, units, structs, machines, reactive
  rules, verification, modules, and the standard library.
- [Library guide](docs/libraries.md): `nn`, `pack`, `json`, `tlm`, `cosim`, and the `std/`
  library.
- [Device integration guide](docs/device_integration.md): devices, backends, the simulator,
  record/replay, and wiring notes.

## Project layout

```
src/lexer.rs       tokens: comments, f-strings, hex/bin literals, primes x', unit brackets
src/parser.rs      Pratt parser → AST (src/ast.rs): structs, machines, when/always, import, |>
src/interp.rs      evaluator core: scopes, closures, contracts, macros, tasks
src/ext.rs         units math, kwargs, types, structs, machines, signals, reactive rules,
                   safety monitors, modules + library search path, contract fuzzing
src/vm.rs          bytecode compiler + VM (automatic tier for eligible functions)
src/llvm.rs        LLVM IR generator for the typed numeric subset (tulpa build / emit-llvm)
src/lsp.rs         language server (tulpa lsp)
src/units.rs       dimensional analysis (SI base vector, unit parser, conversions)
src/builtins.rs    standard library, kin:: module, string/list/dict methods, formatting
src/nn.rs          nn:: tensors, reverse-mode autodiff, layers, optimisers
src/stdx.rs        pack:: json:: tlm:: cosim::
stdlib/            std/ library written in Tulpa (rtl, eda, rl, lipm, topics, frame)
src/devices/       GPIO/I2C/SPI/UART/PWM/Servo/PID/CAN/BLE; sim.rs, linux.rs, tape.rs (record/replay)
examples/          runnable programs with expected.txt (checked by cargo test)
tests/             84 Rust tests (run under both tiers) + Tulpa-level tests
editors/           VS Code extension (editors/vscode) + Neovim/Helix setup
scripts/           release.sh (tarball, .deb, .vsix), deploy.sh, cross_compile.sh
```

## Performance

Release-build benchmarks:

| Benchmark | Interpreter | VM (default) | Native (`tulpa build`) |
|---|---|---|---|
| `fib(30)`, call-heavy | 1.15 s | 0.28 s | 0.005 s |
| 1M-iteration loop + 1M-step PID simulation | 1.39 s | 0.28 s | — |

Every test runs under both the interpreter and the VM (`TULPA_NO_VM=1`). Native output is
differential-tested against the interpreter.

## Roadmap

### Shipped
- **2.0:** units of measure, runtime-checked types, keyword arguments, `|>`, structs with
  invariants, state machines, signals and `when` rules, `always` monitors, modules, contract
  fuzzing (`tulpa verify`), record/replay, PWM and Servo.
- **3.0:** the bytecode VM, LLVM code generation, the gpiochip backend, timing rules
  (`when … for`, `after … within … ensure`), the virtual clock, and the LSP.
- **3.1:** unit-typed native code, CAN and Bluetooth LE, timestamped replay, the VS Code
  extension, and release packaging.
- **3.2:** the `nn`, `pack`, `json`, `tlm`, and `cosim` modules; the `std/` library (rtl, eda,
  rl, lipm, topics, frame); operator overloading for native objects; and the library search path.

### Planned
- CAN-FD and ISO-TP (UDS diagnostics) on top of the CAN layer.
- BLE peripheral (GATT server) mode, so a Raspberry Pi can act as a sensor.
- Native code generation for structs and state machines.
- Publishing the VS Code extension to the Marketplace and Open VSX.
- A package registry for Tulpa modules.

## License

BSD 3-Clause. See [`LICENSE`](LICENSE).
