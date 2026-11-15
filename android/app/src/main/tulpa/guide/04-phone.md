# Your phone as a device: `phone::`

The `phone` module turns the handset into a sensor-rich embedded device. Readings come back as
**quantities with units**, so the dimensional analysis you rely on for robots works on your phone
too.

## Sensors

| Call | Returns |
|------|---------|
| `phone::accel()` | `[x, y, z]` in m/s² (includes gravity) |
| `phone::gravity()` | gravity vector, m/s² |
| `phone::linear()` | acceleration without gravity, m/s² |
| `phone::gyro()` | angular rate `[x, y, z]`, rad/s |
| `phone::mag()` | magnetic field, µT |
| `phone::orientation()` | `{yaw, pitch, roll}` in deg |
| `phone::heading()` | compass heading, 0–360 deg |
| `phone::tilt()` | angle away from flat, deg |
| `phone::shake()` | magnitude of linear acceleration, m/s² |
| `phone::light()` | illuminance, lux |
| `phone::pressure()` | air pressure, hPa |
| `phone::altitude()` | barometric altitude, m |
| `phone::proximity()` | distance, cm |
| `phone::battery()` · `phone::charging()` | 0–1 · bool |
| `phone::steps()` | steps since boot (needs Activity permission) |
| `phone::location()` | `{lat, lon, alt, accuracy, speed, bearing}` (enable Location first) |
| `phone::sample()` | every reading at once, as a dict |
| `phone::has("baro")` | whether a sensor is present |

Sensors stream only while a program, the REPL or the Sensors screen is running, to save battery.
The **Sensors** screen (⋮ menu) shows what your phone has and the call for each.

```tulpa
let a = phone::accel()
print(a)
print(f"tilt: {phone::tilt()}")
```

## Actuators

| Call | Effect |
|------|--------|
| `phone::vibrate(200[ms])` | buzz; a list such as `[100[ms], 50[ms], 100[ms]]` is a pattern |
| `phone::torch(true)` | flashlight on/off |
| `phone::say("hello")` | text-to-speech |
| `phone::beep(880[Hz], 150[ms])` | a tone |
| `phone::toast("text")` | a short pop-up |
| `phone::notify("title", "text")` | a system notification |
| `phone::keep_awake(true)` | keep the screen on while the program runs |
| `phone::gauge(name, value, min, max)` | a live gauge on the Run screen |

When a program ends or is stopped, the torch is switched off and the screen may sleep again.

## Patterns that work well

**Sample with `every!`**, publish into a **signal**, and let **rules** react:

```text
let tilt = signal(0[deg])
when tilt.get() > 30[deg] for 500[ms] { phone::vibrate(100[ms]) }
every!(50[ms], 400, { tilt.set(phone::tilt()) })
```

Guard the physical limits with **`always`**, and drive modes with a **`machine`** — see the
*Tilt guard* and *Shake alarm machine* examples in the library.

On a desktop, `phone::` calls report "not available outside the Tulpa Field app", so keep phone
access in a small module if you share code with HermiSim.
