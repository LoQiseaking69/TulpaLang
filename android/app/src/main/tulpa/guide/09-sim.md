# 3D simulation & co-simulation

Tulpa Field carries a full **MuJoCo** physics engine and a 3D viewer. A Tulpa program drives a
simulated robot through `cosim::`, exactly as it does in HermiSim on the desktop: the same
protocol, the same API, the same programs.

## The Sim tab

- **Pick a model** from the title (or the cube button): the bundled *cartpole*, *starter_model*
  and *morty* humanoid, any MJCF `.xml` in your project, or **Import** one from your files.
- **Drag** to orbit, **two fingers** to pan, **pinch** to zoom, **double-tap** to frame the model.
- **▶ / ⏸** free-runs the physics in real time; **↺** resets; **✋** pushes the robot (away from
  the camera); **real time / max speed** sets the pace of co-simulation.
- The HUD shows sim time, fps, and whether a program is driving the model.

## Co-simulation in three lines

```text
let sim = cosim::connect()       # the phone's MuJoCo model (or a remote HermiSim)
let o = sim.reset()
o = sim.step(ctrl, 10)           # apply controls, advance 10 physics steps, observe
```

When you run a program that uses `cosim::`, Tulpa Field loads its model, starts the co-sim
server on the phone and switches to the Sim tab so you can watch.

**Which model?** In order: the `"model"` in a `hermisim.json` next to the program (HermiSim
workspaces work unchanged), else the first MJCF `.xml` in the program's folder, else the model
open in the Sim tab.

## What a program can do

| Call | Effect |
|------|--------|
| `sim.info()` | model, timestep, actuators (with ranges), joints, sensors, bodies, keyframes |
| `sim.reset(key)` | reset to a keyframe (name or index), returns an observation |
| `sim.step(ctrl, n)` | set controls, step `n` times, observe |
| `sim.observe()` | `time qpos qvel ctrl act_force sensors com com_vel root_pos bodies contacts` |
| `sim.set_ctrl(ctrl)` · `sim.set_state(qpos, qvel)` | write controls / state |
| `sim.apply_force(body, f, torque)` | external wrench on a body |
| `sim.plot(name, v)` · `sim.log(text)` | live plot / console line from the simulator side |

## Examples

- **Cart-pole balance:** an LQR controller balances a pole and recovers from a shove.
- **Morty — humanoid balance:** the complete Morty stack from HermiSim, with capture-point
  balance, a physics-informed COM model, the FPGA governor link and a behaviour machine,
  driving an 18-actuator humanoid on your phone.

From the Sim tab's ⓘ sheet, **New co-sim project for this model** creates a project with a
`hermisim.json` and a starter controller that lists the model's actuators.

## Driving a desktop HermiSim instead

In **Settings → Co-simulation**, enter `host:port` of a HermiSim co-sim server on your network.
Programs then drive the desktop model, which renders in HermiSim's 3D view.
