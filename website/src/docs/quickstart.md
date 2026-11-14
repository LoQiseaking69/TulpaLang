# Quick start

This walk-through takes about ten minutes. You'll write a small Tulpa program that uses units,
a contract, a state machine and a safety envelope, run it against the simulated device backend,
fuzz its contracts, and then open it in HermiSim.

## 1. Hello, physical world

Create `conveyor.rav`. Every snippet on this page was run against Tulpa 3.2 and the output shown is real.

```tulpa
const BELT_SPEED = 0.25[m/s]

fn travel_time(distance: [m], speed: [m/s] = BELT_SPEED) -> [s]
    require distance >= 0[m] and speed > 0[m/s]
    ensure result' >= 0[s]
{
    distance / speed
}

print(f"1.5 m takes {travel_time(1.5[m])}")
print(f"belt speed is {BELT_SPEED.to("cm/s")}")
```

Run it:

```bash
tulpa conveyor.rav
```

```text
1.5 m takes 6 s
belt speed is 25 cm/s
```

Units flow through the arithmetic, so `travel_time` returns seconds. Passing a bare number where
`[m]` is expected, or adding metres to seconds, is a runtime error with a readable message,
not a silent bug.

## 2. Add a state machine and a safety envelope

Append this to the same file:

```tulpa
let belt = GPIO::new(23)
let gate = Servo::new(0, 0)

machine Sorter {
    on estop => Fault { belt.set_low() }

    state Running {
        enter { belt.set_high(); gate.angle(90[deg]) }
        on reject => Diverting
    }
    state Diverting {
        enter { gate.angle(30[deg]) }
        on cleared => Running
    }
    state Fault {
        on reset => Running
    }
}

let s = Sorter()
always not (belt.read() == 1 and s.state == "Fault") else { belt.set_low() }

s.reject()
s.cleared()
print("state:", s.state)
```

By default every device talks to the deterministic **simulator** backend, so this runs on any
machine. Add `--trace` to see every device call, or `--hw` on a Raspberry Pi to drive real pins.

```bash
tulpa --trace conveyor.rav     # every device call, on stderr
tulpa graph conveyor.rav       # Mermaid diagram of Sorter
```

```text
[trace] <GPIO 23 out>.set_high() -> nil
[trace] <Servo 0.0 at 90 deg>.angle(90 deg) -> nil
[trace] <GPIO 23 out>.read() -> 1
...
state: Running
```

`tulpa graph` prints Mermaid source you can paste into any Markdown that renders diagrams:

```text
stateDiagram-v2
    [*] --> Running
    Running --> Diverting : reject
    Diverting --> Running : cleared
    Fault --> Running : reset
    Running --> Fault : estop
    Diverting --> Fault : estop
```

## 3. Let Tulpa find bugs for you

`tulpa verify` generates inputs that satisfy each function's `require` clauses, checks the
`ensure` clauses, and shrinks any failure to a minimal counterexample.

```bash
tulpa verify conveyor.rav
```

```text
  ok    travel_time()  300 cases, 0 discarded

verified 1 function(s): 1 passed, 0 failed
```

## 4. Open it in HermiSim

1. Launch HermiSim and use **File ▸ Open** to open the folder containing `conveyor.rav`.
2. Pick **current file** in the run target box and press **▶ Run**.
3. Press **Diagram** in the toolbar to see `Sorter` rendered as a state diagram (with a **Mermaid source** tab beside it).
4. Open the **Tulpa Terminal** and try `$ verify conveyor.rav`.

To drive a physics model instead, see [Co-simulation](cosimulation.md). The bundled
[Morty project](morty.md) is a complete worked example.

## Where next

- The [language guide](language-guide.md) covers every construct used above in depth.
- [Device integration](device-integration.md) explains the simulator, `--hw`, record/replay, CAN and BLE.
- The [example gallery](examples.html) has runnable programs with their expected output.
