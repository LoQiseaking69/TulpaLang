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

## Interfaces and drivers

```tulpa
interface Thermometer { fn temperature(self) -> [K] }
struct Fake impl Thermometer {                  # checked when it's defined
    t: [K] = 294[K]
    fn temperature(self) -> [K] { self.t }
}
fn report(s: Thermometer) { print(s.temperature()) }      # 294 K
report(Fake())
```

`std/driver` defines the standard `Imu`, `Rangefinder`, `Adc` and `PwmController`
interfaces. `std/drivers/` holds real drivers (MPU-6050, PCA9685, MCP3008) that run here on
the simulated bus. On a computer, `tulpa build --target rp2040` compiles the same code into
firmware. Try **Your phone as an Imu** in the Library.

## Planning, middleware and simulation

```tulpa
import "std/plan"
let p = plan.Planner()
p.action("undock", {docked: true}, {docked: false})
p.action("drive", {docked: false}, {at: "shelf"})
print(p.plan({docked: true, at: "dock"}, {at: "shelf"}))      # ["undock", "drive"]
```

`std/plan` also has behavior trees and missions that replan. `std/link` connects nodes with
unit-typed topics; on the phone they run on a simulated network. `std/world` and `std/emu` put
physics and emulated chips behind the real drivers. Try **Autonomy**, **Tulpa Link** and
**Rover** in the Library. Proving contracts (`tulpa prove`) and building firmware run on a
computer.

## Next

Open the **Language guide** for the full reference, or browse the **Library**.
