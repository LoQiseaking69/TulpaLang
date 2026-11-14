# Co-simulation

HermiSim runs Tulpa programs in **lock-step** with a MuJoCo model. The Tulpa program owns time:
each `sim.step(ctrl, n)` applies controls, advances physics `n` steps, and returns an
observation. Nothing moves between calls, so control loops are deterministic and can run faster
than real time.

## A complete example

A one-joint pendulum model, `pendulum.xml`:

```xml
<mujoco model="pendulum">
  <option timestep="0.002"/>
  <worldbody>
    <body name="pole" pos="0 0 1">
      <joint name="hinge" type="hinge" axis="0 1 0" damping="0.05"/>
      <geom type="capsule" fromto="0 0 0 0 0 -0.5" size="0.03" mass="1"/>
    </body>
  </worldbody>
  <actuator><motor name="torque" joint="hinge" ctrlrange="-5 5"/></actuator>
</mujoco>
```

A PD controller in Tulpa, `hold.rav`:

```tulpa
# Hold a pendulum at 30 degrees with a PD controller, in lock-step with MuJoCo.
const TARGET = 30[deg].in("rad")
const KP = 20.0
const KD = 2.0

let sim = cosim::connect()               # address comes from $TULPA_COSIM
let info = sim.info()
print(f"{len(info.actuators)} actuator(s), {info.total_mass:.1f} kg, dt = {info.timestep} s")

let obs = sim.reset()
while obs.time < 3.0 {
    let err = TARGET - obs.qpos[0]
    let u = KP * err - KD * obs.qvel[0]
    obs = sim.step([u], 5)               # apply torque, advance 5 physics steps, observe
    tlm::plot("angle_deg", obs.qpos[0] * 180 / PI)
}
print(f"t = {obs.time:.2f} s, angle = {obs.qpos[0] * 180 / PI:.1f} deg")
```

Run it in the IDE with **▶ Run** (with `pendulum.xml` loaded via **File ▸ Open Model…**), or
headless:

```bash
python -m tulpa_bridge run hold.rav --model pendulum.xml
```

```text
1 actuator(s), 1.0 kg, dt = 0.002 s
t = 3.01 s, angle = 26.8 deg
[hermisim] exit 0 — 1505 physics steps, sim t=3.010s, wall 0.50s
```

The pendulum settles a few degrees short of 30° because a pure PD controller has steady-state
error under gravity. Adding an integral term, or using the built-in `PID::new`, closes the gap.
In the IDE, the `angle_deg` series appears live in the **Telemetry** panel.

## The `cosim::` API

| Call | Returns / does |
|---|---|
| `cosim::connect()` | Connects to `$TULPA_COSIM` (set by HermiSim). |
| `sim.info()` | `{timestep, gravity, nq, nv, nu, total_mass, actuators, joints, sensors, bodies, keys, ...}`. Each actuator lists its `name`, `joint`, `ctrlrange`, `gear` and address in `qpos` / `qvel`. |
| `sim.reset(key?)` | Resets (optionally to a named keyframe, e.g. `"stand"`) and returns an observation. |
| `sim.observe()` | Current observation without stepping. |
| `sim.step(ctrl, substeps)` | Apply `ctrl`, advance `substeps` physics steps, return an observation. |
| `sim.set_ctrl(ctrl)` | Set controls without stepping. |
| `sim.set_state(...)` | Overwrite `qpos` / `qvel`. |
| `sim.apply_force(body, f, torque)` | Apply an external wrench to a body. |
| `sim.plot(...)`, `sim.log(...)` | Send a plot sample or log line to the IDE. |
| `sim.call(method, params)` | Call a custom method registered on the server. |
| `sim.close()`, `sim.addr()` | Close the session / show the server address. |

**Observations** carry `time`, `qpos`, `qvel`, `ctrl`, `sensors` (a map from sensor name to
values), `com`, `com_vel`, `bodies` and `contacts`.

## Speed and disturbances

- The toolbar speed slider (or a run configuration's `realtime`) sets pacing: `0` is as fast as
  possible, `1` is real time, `2` is 2× and so on.
- The **Shove** buttons under the viewport push a body (back, forward, left, right) to test
  disturbance rejection while the program runs.
- The viewport shows the floor stage before a model loads and frames each model automatically.
  Double-click it or press **⌖ Frame** to re-frame.

## Wire protocol

The server speaks newline-delimited JSON over TCP, one object per line (UTF-8):

```text
request       {"id": 7, "method": "step", "params": {"ctrl": [...], "substeps": 5}}
response      {"id": 7, "result": {...}}            or  {"id": 7, "error": "message"}
notification  {"method": "plot", "params": {...}}   (no id, no response)
```

Built-in methods are `info`, `reset`, `observe`, `step`, `set_ctrl`, `set_state` and
`apply_force`, plus any method registered with `CoSimServer.register`. Because the protocol is
this small, another physics engine can be bridged by implementing the same methods.

> [!NOTE]
> Lock-step co-simulation validates control logic against a physics model. It doesn't
> establish that the model matches the physical robot. Use record/replay against real hardware
> for that side of testing.
