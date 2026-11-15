# Tulpa in 10 minutes

A fast pass over the language. Every block runs — tap **Run** under it.

## Values and units

```tulpa
let n = 42
let pi = 3.14159
let name = "rover"
let speed = 0.8[m/s]
let t = 2.5[s]
print(speed * t)                 # 2 m
print((speed * t).to("cm"))      # 200 cm
```

Mixing incompatible units is an error, not a silent bug: `1[m] + 1[s]` stops the program.

## Collections and control flow

```tulpa
let xs = [3, 1, 2]
let cfg = {port: "uart0", baud: 115200}
for i, v in enumerate(xs.sorted()) { print(i, v) }
let label = if cfg.baud > 9600 { "fast" } else { "slow" }
print(label, cfg.port)
```

## Functions and contracts

```tulpa
fn stopping_distance(v: [m/s], decel: [m/s^2]) -> [m]
    require v >= 0[m/s] and decel > 0[m/s^2]
    ensure result' >= 0[m]
{
    v ** 2 / (2 * decel)
}
print(stopping_distance(13.9[m/s], 6.5[m/s^2]))
```

## Structs

```tulpa
struct Joint { name, angle = 0[deg], limit = 90[deg] }
let j = Joint(name: "shoulder")
j.angle = 45[deg]
print(j)
```

## State machines

```tulpa
machine Motor {
    state Off { on start => Running }
    state Running {
        on fault(code) if code > 0 => Fault
        on stop => Off
    }
    state Fault { on reset => Off }
}
let m = Motor()
m.start(); m.fault(3)
print(m.state, m.history)
```

## Signals, rules and safety

```tulpa
let temp = signal(20)
when temp.get() > 80 { print("too hot!") } else { print("ok again") }
always temp.get() < 120 else { print("envelope breached") }
for v in [25, 85, 90, 40, 130] { temp.set(v) }
```

## Timing on a virtual clock

```tulpa
sim::virtual_time(true)
let button = signal(0)
when button.get() == 1 for 25[ms] { print("pressed at", millis(), "ms") }
button.set(1); sleep(10[ms]); button.set(0)     # a bounce: ignored
button.set(1); sleep(50[ms])                     # a real press
```

## Next

Open the **Language guide** for the full reference, or browse the **Library**.
