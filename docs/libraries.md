# Tulpa 3.2 libraries

3.2 adds five native modules (written in Rust, always available) and a `std/` library of
modules written in Tulpa. Together they make it possible to write a full robot stack in Tulpa:
learning controllers, wire protocols, hardware models and board design. HermiSim's Morty project
(`workspace/morty`) uses all of them.

| Module | Kind | What it's for |
|---|---|---|
| `nn::` | native | Tensors with reverse-mode autodiff, MLPs, Adam/SGD, running scalers, Gaussian policies |
| `pack::` | native | Python-`struct`-style binary packing, `f32` arrays, CRC-16/32, COBS |
| `json::` | native | JSON encode/decode, files |
| `tlm::` | native | Telemetry for HermiSim's live plots, events and artifact links |
| `cosim::` | native | Drive a physics engine (HermiSim/MuJoCo) in lock-step over TCP |
| `std/rtl` | Tulpa | Cycle-accurate RTL simulation with VCD waveforms |
| `std/eda` | Tulpa | Schematic capture as code: ERC, power budgets in units, netlist/BOM export |
| `std/rl` | Tulpa | Gaussian actor-critic, GAE, PPO on top of `nn::` |
| `std/lipm` | Tulpa | Linear inverted pendulum, ZMP, capture point, support polygons |
| `std/topics` | Tulpa | ROS-style typed publish/subscribe built on signals |
| `std/frame` | Tulpa | COBS + CRC-16 packet framing for UART/BLE/sockets |

## Library search path

`import "std/rtl"` (or just `import "rtl"` when there's no local file of that name) is
resolved against, in order:

1. the importing file's directory (as before)
2. each entry of `$TULPA_PATH`
3. `<tulpa binary>/../lib/tulpa`, the installed layout used by the `.deb`, the tarball and HermiSim's bundle
4. `<tulpa binary>/lib`
5. the source tree's `stdlib/`, for `cargo build` builds

## `nn::`: tensors and autodiff

Tensors are dense row-major 2-D `f64` matrices. A vector is `1×n` and a scalar is `1×1`. Any
operation on a tensor that requires gradients is recorded. Calling `loss.backward()` then fills
`.grad()` on every parameter that contributed to `loss`.

```tulpa
nn::seed(7)
let net = nn::MLP([4, 64, 64, 2], "tanh")        # sizes, activation, dropout=0, out_act="linear"
let opt = nn::Adam(net.params(), 3e-4)
opt.clip_grad(1.0)

let x = nn::tensor([[0.1, 0.2, 0.3, 0.4]])
let y = nn::tensor([[1.0, 0.0]])
let loss = (net.forward(x) - y).square().mean()  # operators work on tensors
opt.zero_grad(); loss.backward(); opt.step()
print(loss.item())
```

**Constructors:**
- `tensor(data, requires_grad=false)`, `param(data)`
- `zeros(r, c)`, `ones`, `full(r, c, v)`
- `randn(r, c, scale=1)`, `rand`, `eye(n)`
- `cat(list, dim=1)`, `stack(rows)`

**Tensor methods:**
- **Shape and access:** `shape rows cols numel item tolist flat row(i) get(i,j) set(i,j,v)`
- **Arithmetic:** `+ - * /` with tensors (broadcasting `1×n`, `n×1`, `1×1`) or numbers. `matmul`/`mm`, `pow`, `square`, `sqrt`, `abs`, `neg`, `scale`, `add_scalar`
- **Activations:** `tanh relu sigmoid softplus exp log clamp(lo, hi) dropout(p, training)`
- **Reductions:** `sum(dim=nil)`, `mean(dim=nil)`. `dim` is 0 (over rows) or 1 (over columns). Also `max min argmax norm isfinite`
- **Reshaping:** `t()`, `slice_cols(a, b)`, `slice_rows(a, b)`
- **Autograd:** `backward grad zero_grad detach param clone`
- **Target networks:** `copy_from(t)`, `lerp_from(t, tau)`

**Layers:**
- `Linear(in, out, gain=1)` has `forward params weight bias`.
- `MLP(sizes, act, dropout, out_act, out_gain)` has `forward params n_params train(bool) eval save(path) load(path) copy_from(other, tau=1)`.

**Optimisers:** `Adam(params, lr=1e-3, weight_decay=0)` and `SGD(params, lr=0.01, momentum=0)`.
Both have `step zero_grad grad_norm clip_grad(max) set_lr lr steps`.

**Utilities:**
- `Scaler(features, clip=10)`: Welford running mean/std for online normalisation. Methods: `update(rows) transform(rows) mean std count ready(n) save load`.
- `mse(a, b)`
- `gaussian_log_prob(x, mu, log_std)`, which returns `R×1`
- `sample_gaussian(mu, log_std)`
- `save(path, params)`, `load(path, params)`
- `gauss(mu, s)`, `uniform()`, `seed(n)`

Performance: matrix kernels run in Rust, so a 60→256→256→128→60 network trains at thousands of
samples per second, even from interpreted Tulpa.

## `pack::`: binary layouts

```tulpa
let b = pack::pack("<HHf", 0xBEEF, 7, 1.5)       # byte list
let v = pack::unpack("<HHf", b)                    # [48879, 7, 1.5]
let w = pack::f32s([0.1, 0.2, 0.3])                # little-endian float32 array
pack::to_f32s(w); pack::calcsize(">3d")
pack::crc16(b)       # CRC-16/CCITT-FALSE
pack::crc32(b)       # IEEE
pack::cobs_encode(b); pack::cobs_decode(...)
```

**Format codes:**
- Byte order: `<` (little-endian, the default), `>` or `!` (big-endian)
- Codes: `x b B ? h H i I q Q f d`
- Repeat counts work as in Python: `"<60f"`

Values may be passed as separate arguments or as one list. Out-of-range integers are errors; they never wrap.

## `json::`

`json::encode(v, pretty=false)`, `json::decode(s)`, `json::load(path)`, `json::save(path, v)`.
- Quantities encode as their SI value.
- Structs encode as objects.
- Tensors encode as nested lists.

## `tlm::`: telemetry

When `TULPA_TELEMETRY=1` is set (HermiSim sets it), these emit `@@tlm {json}` lines on stdout.
Without it they are no-ops, so programs stay quiet on the command line.

- `tlm::plot(name, value, t=nil)` adds a sample to a live plot series. Without `t`, the series is plotted against time since the program started.
- `tlm::event(name, data)` sends a marker or log event.
- `tlm::status(text)` updates the IDE status bar.
- `tlm::artifact(path, kind)` makes the IDE offer to open a file the program produced (a VCD, CSV or netlist).
- `tlm::enabled()` reports whether telemetry is on.

## `cosim::`: co-simulation

```tulpa
let sim = cosim::connect()            # address from $TULPA_COSIM (set by HermiSim)
let info = sim.info()                 # {model, timestep, actuators, joints, sensors, bodies, ...}
let obs = sim.reset()
loop {
    let ctrl = controller(obs)
    obs = sim.step(ctrl, 5)           # apply controls, advance 5 physics steps, observe
    if obs.time > 10.0 { break }
}
```

- **Protocol:** newline-delimited JSON-RPC over TCP. Requests are `{"id", "method", "params"}`; replies are `{"id", "result"}` or `{"id", "error"}`.
- **Methods:** `info reset observe step(ctrl, substeps) set_ctrl set_state apply_force(body, f, torque) plot log call(method, params) close addr`.
- **Observations:** `time qpos qvel ctrl sensors{name: [..]} com com_vel bodies contacts`.
- **Running:** HermiSim's *Run in Simulator* and `hermisim run prog.rav --model m.xml` start the server and set `TULPA_COSIM` for you.
- **Other engines:** any engine can be bridged by implementing the same few methods.

## `std/rtl`: RTL simulation

Register-transfer-level semantics, as in Verilog:
- Wires (`sim.sig`) are driven in `comb(self)`.
- Registers (`sim.reg`) are loaded with `.d(v)` in `seq(self)`. All registers commit together on the clock edge.
- `sim.trace("out.vcd")` writes a standard VCD file that you can open in HermiSim's waveform viewer or in GTKWave.

The library ships `Divider` and `Fifo` building blocks. Morty's `fpga/` folder contains a complete UART + torque-governor core, checked bit-exact against a behavioural model.

## `std/eda`: boards as code

**Building a board:** `Board.part(ref, kind, value, pins, cost, draw: [W])`,
`regulator(ref, kind, value, input_rail, outputs, efficiency)`, `power(net, volts)`, `ground(net)`,
`connect(net, pins)`, `decouple(part, pin, gnd, count)`.

**Checking it:**
- `erc()` flags unconnected pins, power or ground pins on the wrong kind of net, and single-pin or empty nets.
- `power_budget()` sums each rail's load in real units (`[W]`, `[V]`, `[A]`) back through the regulators' efficiencies.

**Output:** `bom()`, `total_cost()`, `write_netlist`, `write_bom`, `write_json`.

## `std/rl`, `std/lipm`, `std/topics`, `std/frame`

See the header comment of each file in `stdlib/` for a runnable example.
- **`rl`:** `PPO(obs_dim, act_dim, hidden, lr, clip, epochs, ...)` with `act(obs, deterministic)`, `buffer.add`, `update(last_obs)`, `save/load`.
- **`lipm`:** `omega`, `capture_point`, `zmp`, `residual`, `step`, `support_polygon`, `margin`, `hull`.
- **`topics`:** `Bus.publish/subscribe/topic`, plus `JointState`, `Wrench`, `Imu` and `Float64MultiArray` constructors.
- **`frame`:** `encode(payload, type, seq)` and `Decoder.feed(bytes)`, which resynchronises on the next `0x00` after corruption.
