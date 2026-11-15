# Running, plots & gauges

Tap **▶ Run** anywhere — the editor's top bar or the Run tab. The program runs on its own thread
with a large stack, so the UI stays responsive, and **■ Stop** interrupts it cleanly even inside
`every!`, `sleep` or an endless `loop`.

## Console

Everything the program prints appears in the console. Errors are shown in red with the line they
came from; tap an error to jump to it in the editor.

When a program calls `input("prompt")`, an input field appears at the bottom of the Run screen.

```tulpa
let name = input("your name? ")
print(f"hello, {name}")
```

## Live plots

`tlm::plot(name, value)` adds a point to a live chart. Each name is its own series; quantities are
plotted in their unit.

```tulpa
every!(50[ms], 120, {
    let t = millis() / 1000.0
    tlm::plot("sine", sin(t * 3))
    tlm::plot("cosine", cos(t * 3))
})
```

`tlm::event(name, data)` drops a marker on the timeline, `tlm::status(text)` sets the status line,
and `tlm::artifact(path)` shows a produced file (a waveform, a CSV) with a **Share** button.

## Gauges

`phone::gauge(name, value, min, max)` shows a large live readout with a bar. Use it for the one or
two numbers you want to read at arm's length.

## Virtual time

The **Virtual time** switch runs the program on a simulated clock: sleeps and `every!` complete
instantly, and timing rules (`when … for`, `after … within`) run deterministically. Perfect for
checking timing logic without waiting for it. Sensor examples need real time, so leave it off for
those.

## Arguments

**Run with arguments…** in the Run menu passes values to `args()`.
