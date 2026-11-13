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

`int`, `float`, `str`, `bool`, `nil`, `list`, `dict`, `range`, `fn`, `task`, plus native objects
(`GPIO`, `I2C`, `SPI`, `UART`, `PID`). Use `type(v)` to get the type name.

- `int / int` is floor division, as in C and Rust (`7 / 2 == 3`). Use a float operand to get a float result (`7.0 / 2 == 3.5`).
- `%` is Euclidean modulo, so the result is never negative for a positive divisor. `**` is exponentiation and is right-associative.
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
--hw / --sim / --trace       backend selection and device tracing (also TULPA_BACKEND=linux)
```

Exit codes: 0 means OK, 1 means tests failed, 2 means a usage error, 65 means a syntax error, and 70 means a runtime error.
