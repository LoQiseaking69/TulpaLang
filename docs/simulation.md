# Simulation and verification

Tulpa tests a robot program in layers, with the same source file all the way to the board:

| Layer | Tool | What it gives you |
|---|---|---|
| Deterministic time | `--virtual-time`, `sim::virtual_time(true)` | A simulated clock that starts at 0. Runs repeat exactly. |
| Simulated buses | `sim::` hooks, record/replay | GPIO, I2C, SPI, UART, PWM, CAN and BLE without hardware. |
| Emulated chips | `std/emu` | Register-level models that your real drivers talk to, with fault injection. |
| Physics | `std/world`, `cosim::` | A built-in 2-D world for mobile robots, or MuJoCo through HermiSim. |
| Simulated network | `std/link` | Latency, loss and partitions for multi-node systems. |
| Exploration | `std/verify` | Every event order up to a depth, against the real semantics. |
| Proof | `tulpa prove` | Contracts and invariants for all inputs. See [Proving safety](safety.md). |
| Native equivalence | `tulpa build --target sim --exe` | The compiled program on the same simulated bus. |
| Deployment | `tulpa deploy` | Prove, check, build, flash. |

## Driver-in-the-loop simulation

The controller uses its real drivers. The drivers talk to emulated chips on the simulated bus.
The chips are fed by physics, and the actuator registers drive the physics back.

```tulpa
import "std/world"
import "std/emu"
from "std/drivers/mpu6050" import Mpu6050
from "std/drivers/pca9685" import Pca9685

sim::virtual_time(true)
let w = world.World(dt: 10[ms], seed: 7)
w.arena(3, 2)                                   # metres
w.disc(2.2, 1.0, 0.15)                          # a pillar
let body = w.rover(x: 0.4, y: 1.0, max_speed: 0.8[m/s])

let imu_chip = emu.Mpu6050(0x68)                # emulated parts
let pwm_chip = emu.Pca9685(0x40)
let imu = Mpu6050(I2C::new(1, 0x68))            # the real drivers
let pwm = Pca9685(I2C::new(1, 0x40))

fn plant() {                                    # the part that does not ship
    let cmd = |ch| clamp((pwm_chip.pulse(ch).in("ms") - 1.5) / 0.5, -1.0, 1.0)
    body.drive(cmd(0), cmd(1))
    w.step()
    let s = body.imu()
    imu_chip.set(ax: s.ax, ay: s.ay, az: s.az, gz: s.gz)
}
```

`examples/sim_rover` is the complete program: a rover that avoids a pillar, then halts when
an injected fault takes its IMU off the bus.

### `std/world`

- `World(dt: 10[ms], seed: 1)`, `wall(x1, y1, x2, y2)`, `arena(width, height)`,
  `disc(x, y, r)`, `step(n)`. Each step also advances the virtual clock (turn that off with
  `advance_clock: false`).
- `world.rover(x, y, heading, max_speed, wheel_base, radius)` returns a differential-drive
  `Rover`:
  - `drive(left, right)` with commands in -1..1, and `stop()`.
  - Sensors: `range(angle, limit, sigma)`, `imu(sigma)`, `speed()`, `yaw_rate()`, `yaw()`,
    `position()`, `odo_l`, `odo_r`, `battery`.
  - Collisions stop the rover and count in `bumps`.
- Motors have a first-order lag (`tau`), the battery drains with use, and sensor noise comes
  from the world's seeded generator. The same seed gives the same run on every machine.

For full 3-D rigid-body dynamics, use MuJoCo through `cosim::` in HermiSim.

### `std/emu`

| Model | Feed it | Read it back |
|---|---|---|
| `emu.Mpu6050(addr)` | `set(ax:, ay:, az:, gx:, gy:, gz:, temp:)` in physical units; it honours the ranges the driver configured | — |
| `emu.Pca9685(addr)` | — | `duty(ch)`, `pulse(ch)`, `frequency()` from the registers the driver wrote |
| `emu.Mcp3008()` | `feed(volts)` queues the answer to the next conversion | — |

Fault injection is the same on every model: `chip.fault("offline")` removes it from the bus,
`"stuck"` freezes its outputs, `"noise"` adds seeded noise, and `chip.heal()` clears the faults.

## Exploring every event order

```tulpa
import "std/verify"

let r = verify.explore(|| Door(), ["open", "close", "lock", "unlock"], depth: 6,
                       property: |d| d.state != "Jammed")
r.ok                 # false
r.counterexample     # ["open", "lock"]: the shortest sequence that breaks the property
```

`explore(make, events, depth, property, key)` builds a fresh system for every run and replays
event sequences breadth-first. Guards, actions, `always` envelopes, struct invariants and
contracts all run, and any of them failing is reported with the shortest sequence that causes
it. An event is a name sent with `.send()`, or a `[name, |sys| ...]` pair for anything else.
`complete: true` means every reachable state was visited within the depth.

`verify.deterministic(|seed| ..., seeds)` runs a scenario twice per seed and reports the first
seed whose two runs differ.

## From simulation to hardware

In a project made by `tulpa new <dir> --board <board>`:

```
tulpa deploy                  # in the project folder
tulpa deploy rover --dry-run  # show the steps without running them
```

| Step | What happens | Stops the deploy when |
|---|---|---|
| 1. prove | `tulpa prove` on `app.rav`, then testing of what it left open | an obligation is refuted (override with `--force`) |
| 2. check | `tulpa build --target sim --exe`: the program compiles natively | it does not |
| 3. build | Pico SDK (`cmake`), ESP-IDF (`idf.py build`) or `make` | the build fails |
| 4. flash | RP2040: copy the UF2 to the `RPI-RP2` drive, or `picotool load`. ESP32: `idf.py flash` (`--port`). Linux: `scp` to `--host user@board`. | flashing fails |

`--no-flash` stops after the build.
