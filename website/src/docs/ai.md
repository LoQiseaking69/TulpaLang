# Tulpa AI

Tulpa AI is the agentic platform for Tulpa developers: an AI assistant that writes, fixes and
explains Tulpa code, and verifies what it writes with the real `tulpa` toolchain before it
answers. It's in a **private preview** on Android and iOS.

> [!NOTE]
> Tulpa AI is a separate app from [Tulpa Field](android.md). Tulpa Field is an offline IDE
> that runs Tulpa on your phone; Tulpa AI is an assistant that helps you write it.

## What it can do

| Task | What happens |
|---|---|
| **Write** | Generates programs, drivers, state machines and missions in idiomatic Tulpa. |
| **Fix** | Takes your code and a compiler diagnostic and returns a version that passes `tulpa check`. |
| **Explain** | Walks through code and language features: units, contracts, machines, signals, safety envelopes. |
| **Document** | Writes doc comments and usage notes for existing code. |
| **Agent mode** | Plans a change, runs `tulpa check`, `run`, `test` and `prove` in a sandbox, repairs from the diagnostics and presents a diff for you to approve. |

The apps ship with a knowledge pack built from these docs, the examples and the `std/`
library, so the assistant can cite the right API. It also knows .RAV IDE, Tulpa Field and
TulpaTutor.

## How the model is built

The Tulpa AI model is specialized on chat and agent examples generated from the Tulpa
toolchain itself. Every training example is grounded in real `tulpa check` and `tulpa run`
output: programs that compile and produce the stated result, genuine compiler errors and their
fixes, explanations, docs, and multi-step tool-use trajectories.

Training follows a fixed order:

1. Run the exam on the starting model.
2. Fine-tune on the Tulpa data.
3. Run the same exam on the new model.
4. Promote the new model only if its overall score beats the model currently serving.

So a release can't make the assistant worse at Tulpa than the one before it.

## The exam

Every model is graded by the real `tulpa` compiler, not by another model:

| Section | Measures |
|---|---|
| Knowledge | Compiler-verified questions on the language and its libraries |
| Code generation | pass@1 on executable tasks: the program must run and print the right output |
| Held-out programs | Real Tulpa programs that were never in the training data |
| Agent | End-to-end tasks in the agent loop against `tulpa` 3.5.0 |

The latest results are on the [Tulpa AI page](../ai.html#model).

## Apps

- **Android**: a native app with streaming chat, the agent, projects, an editor and tools.
- **iOS**: a SwiftUI app with the same screens (iOS 17+), installed with AltStore or
  SideStore during the preview.

The model runs on HermiTech's servers; the weights are not distributed. For preview access or
commercial licensing, contact HermiTech LLC through
[GitHub](https://github.com/LoQiseaking69/TulpaLang).
