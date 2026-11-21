# How it fits together

Every product in the Tulpa ecosystem runs the same language. There is one implementation of
Tulpa, in the TulpaLang repository, and each product embeds it rather than reimplementing it:

| Product | How it runs Tulpa |
|---|---|
| **Tulpa** (the CLI) | Native binary built from TulpaLang, with the interpreter, bytecode VM and LLVM backend |
| **.RAV IDE** | Bundles the `tulpa` binary and talks to it through `tulpa lsp`, the REPL and the co-simulation bridge |
| **Tulpa Field** | Links the Tulpa runtime into the Android and iOS apps, so it works offline |
| **TulpaTutor** | Compiles the Tulpa runtime to WebAssembly, so the course runs in the browser and in the apps without a server |
| **Tulpa AI** | Runs `tulpa check`, `run`, `test` and `prove` in a sandbox to verify what the model writes |

So a program that works in one place works in the others, and the examples in these docs,
in TulpaTutor's lessons and in Tulpa Field's guide are all checked against the real toolchain
in CI.

## Which one should I use?

| You want to | Use |
|---|---|
| Learn the language from scratch | [TulpaTutor](tutor.md): lessons, exercises and a playground, nothing to install |
| Write and test robot software on a computer | [.RAV IDE](hermisim.md), or the [CLI](cli.md) with the [editor extension](editors.md) |
| Simulate a robot against physics | [.RAV IDE co-simulation](cosimulation.md) on the desktop, or [Tulpa Field](android-sim.md) on a phone |
| Program your phone's sensors | [Tulpa Field and the `phone::` module](android-phone.md) |
| Build firmware for a Pico or an ESP32 | The CLI's [embedded targets](embedded.md), or **Build Firmware for Board…** in .RAV IDE |
| Get help writing or fixing Tulpa | [Tulpa AI](ai.md) |
| Run checks in CI | The CLI, plus the [headless .RAV IDE runner](headless.md) for co-simulation |

## The workflow

1. **Learn** in TulpaTutor, on the web or on your phone.
2. **Build** in .RAV IDE on the desktop or Tulpa Field on the go, with Tulpa AI to write or
   fix code.
3. **Simulate and verify**: contracts and `tulpa verify`, `tulpa prove`, deterministic
   physics, emulated chips and MuJoCo co-simulation.
4. **Ship** with `tulpa build` and `tulpa deploy`: native executables, standalone bundles and
   firmware for RP2040, ESP32, Cortex-M and RISC-V.

## Versions

Releases are kept in step. When Tulpa ships a new version, .RAV IDE, Tulpa Field and
TulpaTutor move to that runtime, and the docs here are updated with them. The
[roadmap](roadmap.md) lists what each release added, and the [downloads page](../download.html)
always has the latest builds.

## Licensing

Tulpa, .RAV IDE, Tulpa Field and TulpaTutor are source-available under the Business Source
License 1.1. Tulpa AI is a private preview. See [Licensing](licensing.md).
