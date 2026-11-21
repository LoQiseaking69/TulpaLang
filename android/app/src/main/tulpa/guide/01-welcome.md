# Welcome to Tulpa Field

Tulpa Field is a pocket IDE for **Tulpa**, the language for engineers who program things that
move, sense and must not fail. The whole Tulpa toolchain runs on the phone — interpreter, bytecode
VM, units, contracts, state machines, reactive rules and the `std/` libraries. Nothing is sent to a
server, and everything works offline.

.RAV IDE is the desktop studio. Tulpa Field is built for the other half of an engineer's day —
**in the field, with real sensors in your hand** and a physics model in your pocket:

- **Your phone is a device.** `phone::accel()`, `phone::tilt()`, `phone::pressure()`,
  `phone::location()` read real sensors as unit-checked quantities. `phone::vibrate`,
  `phone::torch`, `phone::say` and `phone::beep` are its actuators.
- **Live instruments.** `tlm::plot` draws live charts; `phone::gauge` puts a big readout on the
  Run screen.
- **Physics in your pocket.** MuJoCo runs on the phone: `cosim::` programs drive a simulated
  robot you watch in 3D, the same protocol and API as .RAV IDE.
- **Machine inspector.** Every `machine` in your file is drawn as a state graph you can tap.
- **Verify on the go.** Fuzz every contract in a file and run its tests with one tap.
- **A REPL with history** and a symbol keyboard built for Tulpa's syntax.
- **A library of examples** and this guide, with every code block runnable.

## Your first minute

1. Open **Learn → Library** and pick **Bubble level**.
2. Tap **Run**. Lay the phone on a table and watch the gauge.
3. Tap **Open as project** to get your own editable copy.

Or try a block from this guide right now — tap **Run** under it:

```tulpa
let distance = 42[km]
let time = 1.5[h]
let v = distance / time
print(f"average speed: {v.in("km/h")} km/h")
print(v.to("m/s"))
```

## Where things are

| Tab | What it does |
|-----|--------------|
| **Home** | the welcome page: continue where you left off, quick starts, recent projects |
| **Code** | the editor, with diagnostics, completion, outline and the symbol bar |
| **Run** | console, live plots, gauges, program input, stop |
| **Sim** | the 3D MuJoCo view: models, free-run, push, co-simulation |
| **REPL** | an interactive Tulpa session |
| **Learn** | the example library and this guide |

The **⋮** menu holds the instruments: Sensors, Machines, Verify, Bytecode and Settings.
