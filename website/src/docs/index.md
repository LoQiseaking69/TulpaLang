# Introduction

Welcome to the documentation for **Tulpa**, a Python-flavoured, Rust-powered programming
language for robotics, embedded systems and physical computing, and **.RAV IDE**, the
simulation suite and IDE built around it.

Tulpa's central idea is that software which interacts with the physical world should be able to
express the rules of that world. Units of measure, engineering contracts, state machines,
reactive rules, timing constraints and safety envelopes are part of the language itself rather
than external libraries.

.RAV IDE puts that language in one window with a MuJoCo co-simulator, a waveform viewer,
state-machine diagrams, live telemetry plots and a board/BOM viewer.

## How these docs are organised

| Section | What you'll find |
|---|---|
| **Getting started** | Installing the toolchain and the IDE, then writing and running your first program. |
| **Tulpa language** | The full language guide, the libraries, device integration, the Driver SDK, native and embedded targets (RP2040, ESP32), autonomy, the Tulpa Link middleware, simulation and proofs, the CLI and editor setup, plus a gallery of runnable examples. |
| **.RAV IDE** | A tour of the IDE, project files and run configurations, the co-simulation protocol, telemetry, the integrated terminal, headless CI use and the bundled Morty humanoid project. |
| **Project** | Roadmap and how to contribute to these docs. |

## At a glance

| | Tulpa | .RAV IDE |
|---|---|---|
| What it is | Programming language + toolchain | IDE and simulation suite |
| Implementation | Rust | Python, Qt, MuJoCo |
| Current version | 3.2 | 2.0 |
| Runs on | Linux (x86_64, arm64, armv7), macOS, Windows | Windows installer; Linux / macOS from source |
| License | Business Source License 1.1 | Business Source License 1.1 |

> [!TIP]
> New here? Read the [quick start](quickstart.md), then skim the
> [language guide](language-guide.md) sections on **units**, **state machines** and
> **safety envelopes**. They're what make Tulpa different.

## Source repositories

- [TulpaLang](https://github.com/LoQiseaking69/TulpaLang): the language, toolchain, standard library and VS Code extension.
- [HermiSim-DynamicsViewer](https://github.com/LoQiseaking69/HermiSim-DynamicsViewer): the IDE, co-simulator, Windows installer and the Morty project.
