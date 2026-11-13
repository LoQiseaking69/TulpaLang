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
| **Safety envelopes:** `always cond [else { handler }]` | Monitors checked after every device call, assignment, transition and control tick. Without a handler a violation stops the program; with one, the handler runs. |
| **Record & replay:** `--record run.tape`, `--replay run.tape` | Capture a real hardware session, then rerun the script against the tape in CI. It fails at the first device operation that differs. |
| **Structs:** fields, defaults, methods, `invariant`, operator overloading | Class invariants are re-checked after every method call and field write, and a write that would break one is rolled back. |
| **Devices:** GPIO, I2C (+ `scan`), SPI, UART, PWM, Servo, PID, `kin::` | A deterministic simulator by default. `--hw` switches to real Linux hardware. |

It also has everything you'd expect from a general-purpose scripting language:
- f-strings, `match` with guards and list patterns
- closures, keyword arguments, the `|>` pipe
- `import` modules, `try`/`catch`, `const`
- `spawn`/`await` tasks, macros (`every!` fixed-rate loops, `define_macro!`)
- a REPL, and a test runner

## Quick start

```bash
cargo build --release
T=./target/release/tulpa
$T examples/warehouse_robot/main.rav        # machine + signals + safety envelope + servos
$T examples/units/main.rav                  # dimensional analysis
$T verify examples/verify/main.rav          # fuzz every contracted function
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
- [Device integration guide](docs/device_integration.md): devices, backends, the simulator, record and replay, and wiring notes.

## Project layout

```
src/lexer.rs       tokens: comments, f-strings, hex/bin literals, primes x', unit brackets
src/parser.rs      Pratt parser → AST (src/ast.rs): structs, machines, when/always, import, |>
src/interp.rs      evaluator core: scopes, closures, contracts, macros, tasks
src/ext.rs         2.0 features: units math, kwargs, types, structs, machines, signals,
                   reactive rules, safety monitors, modules, contract fuzzing
src/units.rs       dimensional analysis (SI base vector, unit parser, conversions)
src/builtins.rs    standard library, kin:: module, string/list/dict methods, formatting
src/devices/       GPIO/I2C/SPI/UART/PWM/Servo/PID; sim.rs, linux.rs, tape.rs (record/replay)
examples/          runnable programs with expected.txt (checked by `cargo test`)
tests/             58 Rust tests + Tulpa-level tests
```

## Roadmap

- A bytecode VM, and LLVM codegen for hot control loops.
- A GPIO character-device backend (`/dev/gpiochipN`).
- Time-bounded temporal monitors (`within 200[ms] { ... }`).
- An LSP server, so editors get unit-aware hover and contract diagnostics.

## License

BSD 3-Clause. See `LICENSE`.
