# Roadmap

## Shipped

| Release | Highlights |
|---|---|
| **Tulpa 2.0** | Units of measure, runtime-checked types, keyword arguments, `\|>`, structs with invariants, state machines, signals and `when` rules, `always` monitors, modules, contract fuzzing (`tulpa verify`), record/replay, PWM and Servo. |
| **Tulpa 3.0** | Bytecode VM, LLVM code generation, the gpiochip backend, timing rules (`when … for`, `after … within … ensure`), the virtual clock and the LSP. |
| **Tulpa 3.1** | Unit-typed native code, CAN and Bluetooth LE, timestamped replay, the VS Code extension and release packaging. |
| **Tulpa 3.2** | The `nn`, `pack`, `json`, `tlm` and `cosim` modules; the `std/` library (rtl, eda, rl, lipm, topics, frame); operator overloading for native objects; the library search path. |
| **Tulpa 3.3** | Interfaces and the Driver SDK (`std/driver`, MPU-6050 / PCA9685 / MCP3008 drivers); native compilation of structs, devices, state machines, `always` monitors and `every!` loops; standalone executables; embedded targets for RP2040, ESP32, ESP32-S2/S3/C3/C6 and generic Cortex-M / RISC-V, with `tulpa new` firmware projects. |
| **HermiSim 2.0** | The Tulpa IDE: LSP editor, lock-step MuJoCo co-simulation, telemetry, waveform / machine / BOM viewers, Tulpa terminal, headless CLI, Windows installer and the Morty project. |
| **Tulpa 3.4** | Autonomy (`std/plan`: behavior trees, GOAP, HTN, missions); Tulpa Link middleware (`std/link`); static proofs (`tulpa prove`); deterministic physics and emulated chips (`std/world`, `std/emu`); exhaustive exploration (`std/verify`); native `match`; standalone bundles and `tulpa deploy`. |
| **HermiSim 2.1** | Tulpa 3.3 workflows: Run Natively, Build Firmware for Board…, New Firmware Project…, `native`/`firmware` run kinds, Driver SDK highlighting and docs. |
| **HermiSim 2.2** | Proofs dock, Deploy Project to Board, Bundle, a Tulpa Link Network dock, and the bundled Autonomy lab project. |
| **Tulpa Field 1.3** | The Tulpa 3.4 runtime on Android: planning, Tulpa Link, physics and chip emulation, with new examples and guides. |
| **Tulpa Field 1.2** | The Tulpa 3.3 runtime on Android with the Driver SDK, and *Your phone as an Imu*: `std/driver`'s `Imu` interface implemented on the phone's sensors. |

## Planned

- CAN-FD and ISO-TP (UDS diagnostics) on top of the CAN layer.
- BLE peripheral (GATT server) mode, so a Raspberry Pi can act as a sensor.
- Native dicts, closures and `try` in firmware builds, and a Tulpa Link node for microcontrollers.
- Encryption and authentication for Tulpa Link.
- More reference drivers (BMP280, VL53L0X, INA219) and an STM32 board-support package.
- Publishing the VS Code extension to the Marketplace and Open VSX.
- A package registry for Tulpa modules.

## Current scope

Being clear about boundaries is part of the design:

- Static native compilation covers structs, devices, state machines, monitors, `match` and timing
  loops. Dicts, closures, `try`, tasks and signals run on the interpreter or in bundles, not on
  microcontrollers.
- `tulpa prove` works on individual functions over linear arithmetic and treats floats as reals.
- Contract fuzzing explores test cases. It isn't a mathematical proof of correctness.
- Runtime safety envelopes detect violations at defined checkpoints. They aren't independent
  hardware safety interlocks.
- Record/replay validates recorded software interactions and timing. It doesn't establish
  equivalence between a simulation and the physical world.

Applications that need functional-safety certification still need an appropriate independent
safety architecture, hardware protections and a certification process.
