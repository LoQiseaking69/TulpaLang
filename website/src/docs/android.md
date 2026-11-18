# Tulpa Field: install & tour

**Tulpa Field** is the TulpaLang IDE for Android and iOS. It runs the whole Tulpa runtime on the phone,
offline: the interpreter and bytecode VM, units, contracts, state machines, signals and timing
rules, and the `std/` libraries. It adds three things the desktop can't give you: the phone's own
sensors as a programmable device, MuJoCo physics with a 3D view in your pocket, and a touch-first
editor.

## Install

### Android

1. On your phone, download the APK from the [Tulpa Field page](../android.html) or from the
   [releases list](https://github.com/LoQiseaking69/TulpaLang/releases?q=field).
2. Open it. Android asks once to allow installs from your browser or file manager.
3. Install, then open **Tulpa Field**. Later releases install over the top.

Releases are signed by HermiTech LLC. To check a download, compare the certificate fingerprint
(`apksigner verify --print-certs TulpaField-*.apk`) with:

```text
BC:21:75:A5:1D:B6:56:42:10:25:82:4C:2F:8A:0D:20:DD:48:9F:06:76:4A:DD:BB:18:30:26:39:E3:08:E8:CF
```

**Requirements:** Android 8.0 (API 26) or newer, arm64 / armv7 / x86_64, OpenGL ES 3.0.

### iPhone and iPad

Every release also carries `TulpaField-<version>.ipa`. No Apple Developer account is needed: the
IPA is ad-hoc signed, and a sideloading tool re-signs it with your free Apple ID.

1. Install [AltStore](https://altstore.io) or [SideStore](https://sidestore.io), or use
   [Sideloadly](https://sideloadly.io) on a computer.
2. In AltStore/SideStore, add the source
   `https://github.com/LoQiseaking69/TulpaLang/releases/download/field-v<version>/altstore-source.json`,
   or open the downloaded IPA in the app. In Sideloadly, drag the IPA in and sign in.
3. Trust your Apple ID under Settings › General › VPN & Device Management, and turn on Developer
   Mode when iOS asks.

Apple limits free Apple IDs to 3 sideloaded apps, and the signature lasts 7 days; AltStore and
SideStore refresh it in the background. TrollStore installs the IPA permanently on the iOS versions
it supports. **Requirements:** iOS 16 or newer, arm64, Metal. Details and building from source:
[ios/README.md](https://github.com/LoQiseaking69/TulpaLang/blob/main/ios/README.md).

## The tabs

| Tab | What it does |
|-----|--------------|
| **Home** | the welcome page: continue the last file, quick starts, recent projects |
| **Code** | the editor |
| **Run** | console, live plots and gauges, program input, stop |
| **Sim** | the 3D MuJoCo view and co-simulation |
| **REPL** | an interactive Tulpa session |
| **Learn** | the example library and the in-app guide |

The grid button in the top bar opens the instruments: **Sensors**, **Machines**, **Verify &
tests**, **Bytecode** and **Settings**.

## The editor

- Syntax colouring, live diagnostics (red underline and gutter marks), and a ⚠ count that lists
  every problem.
- Completion strip above the keyboard: keywords, your own names, and `module::` members (type
  `phone::` to see every sensor).
- Inspect, go to definition, outline, find/replace, undo/redo.
- A key row with `{ } ( ) [ ] => : " . |`, cursor keys, Tab, and unit keys (`[m]`, `[s]`,
  `[deg]`, long-press for the full palette).
- Auto-indent and bracket pairing. Files save automatically when you run or switch screens.

## Run, plots and gauges

Programs run on their own large-stack thread; **Stop** interrupts loops, `sleep` and `every!`
cleanly. The console turns errors into links to the failing line. `tlm::plot` draws live charts,
`phone::gauge` puts big readouts on screen, `tlm::event` marks the timeline, and `tlm::artifact`
offers produced files (a CSV, a VCD waveform) for sharing. **Virtual time** runs timing logic on a
simulated clock.

## 3D simulation & co-simulation

MuJoCo runs on the device. The **Sim** tab shows the loaded model in 3D (drag to orbit, two
fingers to pan, pinch to zoom, double-tap to frame), free-runs it, resets it and pushes it.
Programs that call `cosim::connect()` drive it in lock-step. See
[3D simulation & co-sim](android-sim.html) for the details.

## Your phone as a device

The `phone::` module reads the accelerometer, gyroscope, magnetometer, orientation, light,
barometer, proximity, battery, steps and GPS as unit-checked quantities, and drives vibration,
the torch, speech, tones, toasts and notifications. See [the phone:: module](android-phone.html).

## Machines, verify and the REPL

- **Machines** draws every `machine` in the open file as a state graph. Tap a state to jump to
  it; copy the diagram as Mermaid.
- **Verify & tests** fuzzes every function with an `ensure` clause or a return type, shrinks
  failures to a minimal counterexample you can replay in the REPL, and runs `define_test!`
  blocks.
- **REPL** keeps one session alive, with history, *load file*, interrupt and reset.

## Learn

The **Library** has 36 runnable examples: the language's own (including the Driver SDK demo, the
autonomy mission, the Tulpa Link fleet and the simulated rover), 16 written for the phone (among them *Your phone as
an Imu*, which implements `std/driver`'s `Imu` interface on the handset's sensors), and two
co-simulation projects (a cart-pole and the full Morty humanoid). The **Guide** has chapters on
every part of the app plus the complete language reference; every Tulpa code block in it has a
**Run** button.

## Projects and sharing

Projects are folders in the app's storage. Start one from a template or from any example, open
`.rav` files shared from other apps (they land in **Inbox**), and share a file or a whole project
as a `.zip`.
