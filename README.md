# Tulpa

A Python-flavoured, Rust-powered scripting language for **device integration and robotics**,
with Hehner-style specifications built into the syntax.

___
![img](https://github.com/LoQiseaking69/TulpaLang/blob/main/TulpaLangLOGO.png)
___

```tulpa
const ADDR = 0x48                      # TMP102 temperature sensor

fn raw_to_celsius(raw)
    require 0 <= raw and raw <= 0xFFFF
    ensure -55.0 <= result' and result' <= 150.0
{
    let counts = raw >> 4
    if counts & 0x800 { counts = counts - 4096 }
    counts * 0.0625
}

let sensor = I2C::new(1, ADDR)
let fan    = GPIO::new(18, "out")
let pid    = PID::new(2.0, 0.4, 0.05)

every!(100, {                          # fixed-rate 10 Hz control loop
    let c = raw_to_celsius(sensor.read_u16(0x00))
    let effort = pid.update(40.0, c, 0.1)
    fan.write(effort < 0)
    print(f"{c:6.2f} °C  fan={fan.read()}")
})
```

## What's in it

- **Syntax that feels like Python, with braces.** Semicolons are optional. You get f-strings, `elif`, `and`/`or`/`not`, default arguments, closures, lambdas (`|x| x * 2`, `fn(x) => x * 2`), lists, dicts, ranges (`0..n`, `0..=n`), slices and destructuring.
- **Hardware objects built in:** `GPIO`, `I2C`, `SPI` and `UART`, plus a `PID` controller and `kin::` kinematics (`fk2`/`ik2` for a 2-link arm, plus projectile helpers).
- **Two backends, same script:**
  - The in-memory **simulator** is the default. It's deterministic and you can script it through `sim::*`, so you can develop and CI-test robot code on a laptop.
  - `--hw` switches to **real Linux hardware**: sysfs GPIO, `/dev/i2c-N` (with repeated-start register reads), `/dev/spidevB.C` and termios serial ports.
- **Hehner-style specs** that are checked at runtime:
  - `require` sets a precondition.
  - `ensure` sets a postcondition. Inside it, `x` means the initial value, `x'` the final value, and `result'` the return value.
  - `while ... invariant ...` adds loop invariants.
  - `assert` checks a condition inline.
- **Macros:**
  - Built in: `simple_loop!`, `every!` (a fixed-rate loop), `time_execution!`, `log_message!`, `println!`/`format!` with Rust-style `{:.2}`, `assert_eq!`, `define_test!`.
  - Your own: `define_macro!`, with call-by-name arguments.
- **Tasks:** `spawn f(x)` creates a task and `await t` (or `await [t1, t2]`) collects results. Tasks run cooperatively and deterministically.
- **Errors:** `try { } catch e { }` and `error("...")`. Runtime errors carry line numbers and call traces.
- **Tooling:** a CLI with `run`, `test`, `check`, `ast`, a REPL, and a `--trace` flag that logs every device call.

## Quick start

```bash
cargo build --release
./target/release/tulpa examples/hello_world/main.rav
./target/release/tulpa examples/robot_arm/main.rav
./target/release/tulpa --trace examples/device_control/main.rav   # see every bus transaction
./target/release/tulpa test tests/tulpa/std_test.rav              # define_test! blocks
./target/release/tulpa                                            # REPL
sudo ./target/release/tulpa --hw my_robot.rav                     # real hardware on a Pi
```

Install the binary with `scripts/deploy.sh [PREFIX]`. Build for a Raspberry Pi with `scripts/cross_compile.sh`.
Source files use `.rav` or `.tlp`.

## Best use cases

1. **Bring-up and bench scripts for sensors and actuators.** Poke registers and watch the bus with `--trace`, without writing a C driver first.
2. **Testing robot logic with no hardware.** The same script runs against the simulator in CI, with `sim::i2c_set` / `sim::uart_feed` / `sim::spi_respond` standing in for the device, and then on the robot with `--hw`.
3. **Safety-critical glue code.** Use `require` / `ensure` / `invariant` to spell out the physical limits: joint angles, speeds, sensor ranges. A bug then becomes a loud, located error, not a broken servo.
4. **Teaching embedded programming and control.** Python-like syntax, a REPL, a PID controller and kinematics built in, and a simulator that can't fry anything.
5. **Small automation daemons on SBCs.** `every!` handles fixed-rate loops, and the UART helpers handle AT-command modems and GPS modules.

## Docs

- [Language guide](docs/language_guide.md): the full syntax and standard library reference.
- [Device integration guide](docs/device_integration.md): devices, backends, the simulator, and wiring notes.

## Project layout

```
src/lexer.rs      tokens (comments: #, //, /* */; f-strings; hex/bin literals; primes x')
src/parser.rs     Pratt parser → AST (src/ast.rs)
src/interp.rs     tree-walking evaluator: scopes, closures, contracts, macros, tasks
src/builtins.rs   global functions, kin:: module, string/list/dict methods, formatting
src/devices/      GPIO/I2C/SPI/UART/PID; sim.rs (simulator), linux.rs (real hardware)
examples/         runnable programs, each with expected.txt (checked by `cargo test`)
tests/            Rust integration tests + Tulpa-level tests
```

## Roadmap

- A bytecode VM, and LLVM codegen for hot loops. The current interpreter does about 3M calls/s in release builds.
- A GPIO character-device backend (`/dev/gpiochipN`) alongside sysfs.
- True preemptive scheduling for tasks, plus a `select` on device events.
- Modules and imports across files.

## License

BSD 3-Clause. See `LICENSE`.
