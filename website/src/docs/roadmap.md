# Roadmap

## Shipped

| Release | Highlights |
|---|---|
| **Tulpa 2.0** | Units of measure, runtime-checked types, keyword arguments, `\|>`, structs with invariants, state machines, signals and `when` rules, `always` monitors, modules, contract fuzzing (`tulpa verify`), record/replay, PWM and Servo. |
| **Tulpa 3.0** | Bytecode VM, LLVM code generation, the gpiochip backend, timing rules (`when … for`, `after … within … ensure`), the virtual clock and the LSP. |
| **Tulpa 3.1** | Unit-typed native code, CAN and Bluetooth LE, timestamped replay, the VS Code extension and release packaging. |
| **Tulpa 3.2** | The `nn`, `pack`, `json`, `tlm` and `cosim` modules; the `std/` library (rtl, eda, rl, lipm, topics, frame); operator overloading for native objects; the library search path. |
| **HermiSim 2.0** | The Tulpa IDE: LSP editor, lock-step MuJoCo co-simulation, telemetry, waveform / machine / BOM viewers, Tulpa terminal, headless CLI, Windows installer and the Morty project. |

## Planned

- CAN-FD and ISO-TP (UDS diagnostics) on top of the CAN layer.
- BLE peripheral (GATT server) mode, so a Raspberry Pi can act as a sensor.
- Native code generation for structs and state machines.
- Publishing the VS Code extension to the Marketplace and Open VSX.
- A package registry for Tulpa modules.

## Current scope

Being clear about boundaries is part of the design:

- LLVM compilation supports a typed numeric subset, not the whole language.
- Contract fuzzing explores test cases. It isn't a mathematical proof of correctness.
- Runtime safety envelopes detect violations at defined checkpoints. They aren't independent
  hardware safety interlocks.
- Record/replay validates recorded software interactions and timing. It doesn't establish
  equivalence between a simulation and the physical world.

Applications that need functional-safety certification still need an appropriate independent
safety architecture, hardware protections and a certification process.
