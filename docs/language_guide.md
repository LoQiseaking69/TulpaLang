# Tulpa Language Guide

Tulpa is a dynamically typed, expression-oriented language. It uses Python-like vocabulary with
brace-delimited blocks. Statements end at a newline or `;`.

## Lexical basics

| Thing        | Syntax |
|--------------|--------|
| Comments     | `# line`, `// line`, `/* block */` |
| Integers     | `42`, `1_000_000`, `0xFF`, `0b1010` |
| Floats       | `3.14`, `1e-3` |
| Strings      | `"text"` or `'text'`, escapes `\n \t \\ \"` |
| f-strings    | `f"x = {x}, hex = {n:#04x}, two dp = {v:.2f}"` |
| Booleans/nil | `true`, `false`, `nil` |
| Primes       | `result'`, `x'`: only meaningful inside `ensure` clauses |

A `(` or `[` on a **new line** starts a new statement. It never continues a call or index from the line above.

## Variables

```tulpa
let speed = 10            # declare
let a, b = [1, 2]         # destructure
const MAX_RPM = 3000      # constant: reassignment is an error
speed = 12                # assign (variable must already be declared)
speed += 1                # also -= *= /= %=
```

Assigning to a name you never declared is an error. This catches typos.

## Types

`int`, `float`, `str`, `bool`, `nil`, `list`, `dict`, `range`, `fn`, `task`, `quantity`, `signal`,
struct objects, machine instances, plus native objects (`GPIO`, `I2C`, `SPI`, `UART`, `PWM`, `Servo`, `PID`). Use `type(v)` to get the type name.

- `int / int` is Euclidean integer division (`7 / 2 == 3`, `-7 / 2 == -4`, `-7 / -3 == 3`). It pairs with `%` so that `a == b * (a / b) + a % b` always holds. Use a float operand to get a float result (`7.0 / 2 == 3.5`).
- `%` is the Euclidean remainder, so it is never negative. `**` is exponentiation and is right-associative.
- Integer overflow is an error. It never silently wraps.
- `str + anything` concatenates.
- Truthiness: `nil`, `false`, `0`, `0.0`, `""`, `[]` and `{}` are false. Everything else is true.

## Operators (low → high precedence)

`or ||` · `and &&` · `not` · `== != < <= > >= in not in` · `.. ..=` · `|` · `^` · `&` · `<< >>` ·
`+ -` · `* / %` · unary `- ! ~` · `**` · call / index / `.field` / `.method()`

## Collections

```tulpa
let xs = [3, 1, 2]
let zeros = [0; 8]                    # repeat literal
let cfg = {port: "/dev/ttyUSB0", "baud": 115200}
xs[0]; xs[-1]; xs[1..3]; "hello"[1..=3]
cfg.port; cfg["baud"]; cfg.baud = 9600
2 in xs; "port" in cfg; 5 in 0..10
```

**List methods:** `push append pop insert remove extend clear contains index count first last sort
sorted reverse copy join slice map filter reduce each any all sum min max enumerate hex len`

**String methods:** `upper lower strip lstrip rstrip split lines chars replace contains starts_with
ends_with find count join repeat format bytes is_digit is_alpha to_int to_float pad_left pad_right len`

**Dict methods:** `get(k, default) set has remove keys values items copy clear update len`.
If a dict holds a function under some key, `obj.key(args)` calls it, which gives you a lightweight object pattern.

## Control flow

```tulpa
if t > 80 { cool() } elif t < 10 { heat() } else { idle() }
let label = if ok { "pass" } else { "fail" }      # if is an expression

while running { step() }
loop { if done() { break } }
for i in 0..10 { }          # exclusive range
for i in 1..=10 { }         # inclusive range
for i, v in enumerate(xs) { }
for k, v in cfg { }          # dict items
```

`break` and `continue` work in all loops, including the `simple_loop!` and `every!` macros.

## Functions

```tulpa
fn area(w, h = w) { w * h }          # last expression is the return value
fn early(x) { if x < 0 { return 0 }; x }
def legacy(x) { x }                  # `def` is an alias for `fn`
let double = |x| x * 2               # closure
let triple = fn(x) => x * 3          # expression-bodied lambda
let block  = |x| { let y = x + 1; y * y }
```

Functions are hoisted, so you can call them above their definition. Closures capture their defining scope by reference.

## Pattern matching

```tulpa
match reading {
    0 => "off",
    1 | 2 => "low",
    3..=9 => "mid",
    "error" => "sensor fault",
    [] => "no data",
    [first, ..rest] => f"{first} then {len(rest)} more",
    n if n > 100 => "overrange",
    _ => "other",
}
```

The arms are tried in order. If no arm matches, that's a runtime error.

## Hehner-style specifications

Inspired by Eric Hehner's *a Practical Theory of Programming*. In that theory a specification is a
boolean expression over the initial state (`x`) and the final state (`x'`).

```tulpa
fn set_joint(angle, speed)
    require -90 <= angle and angle <= 90
    require speed > 0
    ensure result' == angle
{
    ...
    angle
}

fn bump(x) ensure x' == x + 1 { x = x + 1; x }

let i = 0
let total = 0
while i < n invariant total == i * (i - 1) / 2 {
    total += i
    i += 1
}

assert total >= 0, "total went negative"
```

- `require`: checked after the arguments are bound. It may refer to parameters and to anything in scope.
- `ensure`: checked after the body runs.
  - `p` is a parameter's initial value and `p'` is its final value.
  - `result'` is the return value.
- `invariant`: checked before every loop test, including the final one.
- A violation is a runtime error that quotes the clause's source, for example `postcondition violated in bump(): ensure x' == x + 1 (result' = 41)`.

## Errors

```tulpa
try {
    sensor.read_u16(0x00)
} catch err {          # err is the message string; the name is optional
    print("sensor offline:", err)
}
error("custom failure")  # raise
```

Uncaught runtime errors print the line and a call trace, then exit with code 70.

## Macros

| Macro | Purpose |
|-------|---------|
| `simple_loop!(n, { ... })` / `repeat!` | repeat a block n times |
| `every!(ms, { ... })` / `every!(ms, n, { ... })` | fixed-rate loop. It doesn't drift, and if the body overruns it skips the missed ticks instead of trying to catch up. Returns the tick count |
| `time_execution!({ ... })` | run the block, print the elapsed time, return the block's value |
| `log_message!(msg)` | print a timestamped message |
| `println!(fmt, ...)`, `print!`, `format!` | Rust-style formatting: `{}` `{:?}` `{:.2}` `{:>8}` `{:08.3}` `{:#x}` `{0}` |
| `vec!(a, b, ...)` | build a list |
| `assert!(c)`, `assert_eq!(a, b)` | assertions |
| `dbg!(v)` | print a value to stderr and return it |
| `unwrap_or_default!(expr, default)` | `default` if `expr` errors or is `nil` |
| `define_test!(name, { ... })` | register a test for `tulpa test` |

You can define your own macros. Arguments are **call-by-name**: each time the body uses a parameter,
the argument expression or block is evaluated again, in the caller's scope.

```tulpa
define_macro!(retry, (times, body) {
    let n = 0
    while n < times {
        try { body; break } catch e { n += 1 }
    }
})
retry!(3, { uart.send("AT\r\n") })
```

## Tasks

```tulpa
let t = spawn read_sensor("imu")     # deferred call → task
let both = await [t, spawn read_sensor("gps")]
spawn fn() { print("runs before exit if never awaited") }
```

Tasks run cooperatively and in a deterministic order. A task runs when it's awaited, when you call
`join_all()`, or at the end of the program. Use `every!` for periodic work.


## Units of measure

Write a unit in brackets directly after a number, with no space: `5[m]`, `0.5[m/s]`, `9.81[m/s^2]`,
`90[deg]`, `2200[mAh]`. A quantity stores its SI value and its dimension, and remembers the unit
it was written in, for display.

```tulpa
let v = 120[m] / 9.5[s]          # 12.63 m/s
v.to("km/h")                     # quantity in another unit
v.in("mph")                      # plain float in that unit
v.value; v.unit; v.si            # display value, unit string, SI value
2[m] + 50[cm]                    # 2.5 m
5[m] / 2[m]                      # 2.5: the dimensions cancel, giving a plain float
sin(30[deg]); sqrt(16[m^2])      # math functions understand angles and dimensions
f"{v:.1f}"                       # "12.6 m/s"
quantity(3, "ft")                # build a quantity at runtime
```

**Rules:**
- `+`, `-`, `%` and comparisons need matching dimensions. `*` and `/` combine dimensions; `**` takes an integer exponent.
- Comparing against a bare `0` is always allowed (`speed > 0`).
- `float(q)` is an error. Say what you mean with `.in("unit")` or `.si`.

**Built-in units** (combine them with `*`, `.`, `/` and `^n`):

| Group | Units |
|---|---|
| Length | `m km cm mm um nm in ft yd mi` |
| Mass | `kg g mg t lb oz` |
| Time | `s ms us ns min h` |
| Current, voltage, resistance, charge | `A mA uA V mV kV ohm kohm Mohm C mAh Ah` |
| Base units | `K mol cd` |
| Angle | `rad deg rev` |
| Frequency | `Hz kHz MHz rpm` |
| Force, energy, power | `N kN J kJ Wh W mW kW` |
| Pressure | `Pa kPa bar psi` |
| Speed, acceleration | `kph mph gee` |

`C` is the coulomb, not Celsius. Offset temperature scales aren't modelled; use `K`.

## Type annotations

Annotations are optional and **checked at runtime**:

```tulpa
fn drive(speed: [m/s], duration: [s] = 2[s], label: str? = nil) -> [m] { speed * duration }
```

| Annotation | Accepts |
|---|---|
| `any` | anything |
| `int`, `float`, `str`, `bool`, `list`, `dict`, `range`, `task`, `nil` | that type |
| `num` | `int` or `float` |
| `fn` | a function or callable |
| `quantity` | any quantity |
| `signal` | a signal |
| `[unit]` | a quantity of that dimension |
| a struct or machine name, or a device type such as `GPIO` | an instance of it |
| any of the above followed by `?` | that type, or `nil` |

Struct fields accept the same annotations.

## Keyword arguments and the pipe

```tulpa
area(4, scale: 10)            # positional, then keyword
Motor(pin: 12, speed: 0)      # works for struct and machine constructors too
xs |> map(|x| x * 2) |> sum   # a |> f(b) == f(a, b);  a |> f == f(a);  a |> o.m == o.m(a)
```

## Structs

```tulpa
struct Motor {
    pin: int
    speed = 0                                  # default (may refer to earlier fields)
    invariant -100 <= self.speed and self.speed <= 100
    fn set(self, s) { self.speed = s }         # method: first parameter is self
    fn stopped() { Motor(pin: 0) }             # no self: static method, called as Motor.stopped()
    fn to_str(self) { f"motor@{self.pin}" }    # used by print / str / f-strings
    fn init(self) { }                          # optional hook, runs after construction
}
```

- **Construction:** positional (`Motor(12)`), keyword (`Motor(pin: 12)`), or a mix.
- **Invariants:** checked after construction, after every method call, and after every field write. A field write that breaks an invariant is **rolled back** before the error is raised.
- **Operator overloading:** `op_add op_sub op_mul op_div op_mod op_eq op_cmp`. `op_cmp` returns a number that is negative, zero or positive. `2 * v` and `1 + v` try `op_rmul`/`op_radd` first, then fall back to `op_mul`/`op_add`.
- **Reflection:** `Type.fields()` and `Type.name()`.

## State machines

```tulpa
machine Door(locked = false) {              # parameters become per-instance variables
    on estop => Closed { print("E-STOP") }  # global: valid in every state
    state Closed {                          # the first state is the initial state
        enter { print("closed") }
        on open if not locked => Open
        on lock => Locked { locked = true } # action runs between exit and enter
    }
    state Open {
        exit { print("leaving") }
        on close => Closed
        on knock { print("internal transition, no exit/enter") }
    }
    state Locked {
        on unlock(code) if code == 1234 => Closed { locked = false }
    }
}

let d = Door()                 # or Door(locked: true)
d.send("open")                 # true if a transition fired, false if none matched
d.open()                       # shorthand for send("open")
d.unlock(1234)                 # event arguments bind to the declared parameters
d.state; d.history; d.locked   # current state, visited states, instance variables
d.can("open"); d.is("Open"); d.events(); d.restart()
Door.mermaid()                 # Mermaid stateDiagram source (also: tulpa graph file.rav)
```

How transitions are chosen and run:
- The current state's transitions are tried in order, then the global ones. The first one whose guard passes fires.
- An event the machine defines always takes precedence over a convenience method of the same name. The exceptions are `send`, `can`, `is`, `in_state` and `events`.
- A transition to an unknown state is a **syntax error**.

## Signals and reactive rules

```tulpa
let temp = signal(20[K])            # a reactive value (keeps the last 256 samples)
let f = derive(|| temp.get() * 2)   # computed on read
temp.set(300[K]); temp.value = 301[K]
temp.get(); temp.prev(); temp.history(); temp.avg(10); temp.min(); temp.max()
temp.rate()                         # change per second between the last two samples
temp.subscribe(|new, old| print(old, "->", new))

when temp.get() > 310[K] { fan.set_high() } else { fan.set_low() }
```

`when` is **edge-triggered**: the body runs when the condition goes from false to true, and the
optional `else` runs when it goes back to false. A rule is re-evaluated:
- when it's registered
- after every signal change, variable assignment, device call and state-machine transition
- on every `every!` tick

If rules keep re-triggering each other for 100 rounds, that's reported as an error.

## Safety envelopes

```tulpa
always not (heater.read() == 1 and fan.read() == 0)           # fail-stop
always speed <= 2[m/s] else { speed = 2[m/s]; log.push("limited") }
```

An `always` monitor is checked at the same points as `when` rules.
- With no `else`, a violation is a runtime error naming the clause: `safety envelope violated: always ...`.
- With an `else` handler, the handler runs once per violation. It re-arms when the condition holds again.

Monitors run *after* the action that caused the violation, so put the corrective action (stop, e-stop, clamp) in the handler.

## Contract-driven verification

```tulpa
let r = check(my_fn, 500)   # {ok, passed, discarded, counterexample, error, name}
```

`check` generates arguments from the parameter annotations: ints, floats, bools, strings, lists, or
quantities in the annotated unit. It keeps only argument sets that satisfy every `require`, then
calls the function. Any `ensure` violation, type error or crash counts as a failure, and the
failing input is **shrunk** toward the smallest counterexample. Generation is seeded per function, so
results are reproducible.

`tulpa verify file.rav [--trials=N]` loads only the definitions in a file, runs `check` on every
function that has an `ensure` or a return annotation, and exits with code 1 if any fail.

## Modules

```tulpa
import "lib/drive.rav"              # binds `drive` (the file stem) to a module
import "lib/drive" as d             # `.rav` is optional; alias with `as`
from "lib/drive" import Twist, wheels
drive.TRACK_WIDTH; d.wheels(t)
```

- Paths are resolved relative to the importing file.
- Each module runs once and is cached.
- Paths not found next to the importing file are searched on the library path (`$TULPA_PATH`, the
  installed `lib/tulpa`, the source tree's `stdlib/`). So `import "std/rtl"` works from anywhere.
  See the [library guide](libraries.md).
- Names starting with `_` stay private.
- A circular import is an error.


## Timing rules

```tulpa
when button.read() == 1 for 25[ms] { start() }          # debounce: must hold 25 ms
when temp.get() > 80 for 2[s] { alarm() } else { clear() }

after motor.read() == 1 within 300[ms] ensure at_limit.get() else { motor.set_low() }
```

- `when cond for D`: the body fires once the condition has held continuously for duration `D`. If the condition drops before then, the timer resets.
- `after trigger within D ensure response [else { handler }]`: arms on each rising edge of `trigger`. It's satisfied as soon as `response` holds. If `D` passes first, the handler runs; with no handler, the program stops with a `deadline missed` error.
- Durations accept time quantities (`250[ms]`) or plain seconds.

Time-based rules are re-evaluated as time passes. When one is registered, sleeps are sliced and
re-check the rules every 2 ms, and every `every!` tick checks them too.

**Virtual clock.** `--virtual-time`, or `sim::virtual_time(true)` from a script, replaces the wall
clock with a simulated one:
- `sleep`, `sleep_ms`, `every!` and `sim::advance(dur)` move it forward instantly.
- With timing rules registered, it moves in 1 ms steps.
- `millis()` reads it.

Timing logic then runs deterministically and fast in tests. `sim::gpio_input`, `sim::i2c_set` and
`sim::uart_feed` count as external events, so rules see them immediately.

## Execution tiers

1. **Interpreter.** Runs everything.
2. **Bytecode VM.** On its first call, each function is compiled to stack bytecode with resolved
   local slots and executed with int/float fast paths. It's 4–5× faster than the interpreter and
   is on by default.
   - Functions using closures, nested definitions, `match`, `try`, macros or destructuring stay on the interpreter.
   - Both tiers share values, contracts, types and error traces.
   - `tulpa bytecode file.rav` shows each function's bytecode, or why it stays interpreted.
   - `--no-vm` or `TULPA_NO_VM=1` turns the VM off.
3. **Native code.** `tulpa build file.rav -o ctl.o [--shared] [-O3]` compiles the typed numeric
   subset through LLVM and writes `ctl.h` with C prototypes. Each function is exported as
   `tulpa_<name>`. `tulpa emit-llvm` prints the IR instead.

   **What compiles natively:**
   - `int`, `float`, `num` and `bool` parameters (unannotated means `float`)
   - locals, arithmetic, comparisons, `and`/`or`
   - `if`, `while` (including invariants), `for i in a..b`, `loop`, `break`/`continue`, `return`
   - `assert`, `const` numbers, calls between native functions
   - math builtins: `sqrt sin cos tan asin acos atan atan2 hypot exp log log10 abs floor ceil round min max clamp pow float int radians degrees`

   **Semantics match the interpreter:**
   - Integer `/` and `%` are Euclidean.
   - Integer overflow and division by zero are caught.
   - `require`, `ensure`, loop `invariant` and `assert` become checks that call
     `tulpa_violation(msg)`. That's a weak symbol which prints the message and aborts, so you can
     override it in firmware (for example, to trigger an emergency stop).

   **Unit-typed parameters** (`v: [km/h]`, `-> [m]`) compile too.
   - Dimensions are checked when you build, so `d + t` with a length and a time is rejected with a unit-mismatch message.
   - The generated code works in SI doubles.
   - At the C boundary, values pass in the declared unit: `tulpa_stop_dist(36.0 /* km/h */, ...)`.
   - Calls between native functions convert automatically.
   - `.in("unit")` and `.to("unit")` with a literal unit, `.si`, `sqrt`, `abs`, `min`/`max`/`clamp`, and trig on angles are all supported.

   Functions outside the subset are listed with the reason they were skipped.

## Editor support

`tulpa lsp` speaks the Language Server Protocol over stdio and provides:
- diagnostics: syntax errors, unit mismatches between constant quantities, and never-defined names with "did you mean" suggestions
- hover: function signatures with types and contracts, struct fields and invariants, machine states and events, unit dimensions
- completion: keywords, builtins and document symbols; unit names inside `[...]`; methods after `.`
- document symbols and go-to-definition

Setup for VS Code and Neovim is in `editors/`.

## Standard library (globals)

- **I/O:** `print(...)`, `input(prompt)`, `read_file`, `write_file`, `append_file`
- **Conversion:** `str int float bool repr type list dict`
  - `int("0x1F")` parses hex and `int("101", 2)` takes a radix.
- **Math:** `abs min max sum round floor ceil sqrt pow exp log log10 sin cos tan asin acos atan atan2 hypot radians degrees clamp lerp`
  - Constants: `PI TAU E HIGH LOW`.
- **Sequences:** `len range map filter reduce sorted reversed enumerate zip any all keys values items`
- **Bytes and bits:** `hex bin ord chr bytes decode int_from_bytes(list, "big"|"little", signed) int_to_bytes(n, len, order)`, `n.bit(k)`
- **Time:** `sleep(s) sleep_ms(ms) millis() micros() now()`
- **Random:** `random() randint(a, b) choice(xs)`
- **Misc:** `format(v, spec)`, `error(msg)`, `exit(code)`, `join_all()`, `ARGS` (the script's arguments)
- **2.0:** `quantity(v, unit)`, `signal(v, capacity)`, `derive(fn)`, `check(fn, trials)`

## Modules

Everything is built in. `use gpio::GPIO` is accepted, for readability and so scripts written for
other Tulpa drafts still run, but it isn't required.

- `math::pi`, `math::sqrt`, … (`math::` is an alias for the globals)
- `time::millis`, `time::sleep`
- `kin::velocity(v0, a, t)`, `kin::position(x0, v0, a, t)`, `kin::acceleration(dv, t)`,
  `kin::time_of_flight(v, deg)`, `kin::range(v, deg)`, `kin::max_height(v, deg)`
- `kin::fk2(l1, l2, θ1, θ2)` returns `[x, y]`. `kin::ik2(l1, l2, x, y, elbow_up=false)` returns `[θ1, θ2]`. Angles are in degrees.
- `GPIO::new`, `I2C::new`, `SPI::new`, `UART::new`, `PID::new`, `sim::*`: see the
  [device guide](device_integration.md)

## CLI

```
tulpa file.rav [args]        run (args available as ARGS)
tulpa test file.rav ...      run define_test! blocks
tulpa check file.rav ...     syntax check only
tulpa ast file.rav           dump the AST
tulpa                        REPL (:log, :reset, :quit)
tulpa verify file.rav        fuzz contracted functions (--trials=N)
tulpa build file.rav -o x.o  native object + header (--shared, -O0..3); emit-llvm prints IR
tulpa bytecode file.rav      VM bytecode per function
tulpa lsp                    language server over stdio
--virtual-time / --no-vm     simulated clock / interpreter only
tulpa graph file.rav         Mermaid diagrams of all state machines
--hw / --sim / --trace       backend selection and device tracing (also TULPA_BACKEND=linux)
--record F / --replay F      record device traffic to a tape / replay and check against it
```

Exit codes: 0 means OK, 1 means tests failed, 2 means a usage error, 65 means a syntax error, and 70 means a runtime error.
