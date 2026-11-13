# Tulpa for VS Code

Language support for [Tulpa](https://github.com/LoQiseaking69/TulpaLang), a language for robots and hardware.

- Syntax highlighting, including unit literals (`0.5[m/s]`), contracts (`require`, `ensure`, `x'`), state machines, `when`/`always`/`after … within`, and macros.
- A language server, run by `tulpa lsp`:
  - Diagnostics: syntax errors, unit mismatches such as `2[m] + 3[s]`, and undefined names with suggestions.
  - Hover: signatures with types and contracts, struct and machine summaries, unit dimensions.
  - Completion, including unit names inside `[...]`; document outline; go-to-definition.
- Commands: **Run File** (F5), **Run Tests**, **Verify Contracts**, **Build Native Object**, **Show State Machine Diagrams**.

Requires the `tulpa` executable on your `PATH`, or set `tulpa.serverPath`. Add flags for Run File with
`tulpa.runArgs`, for example `["--virtual-time"]` or `["--hw"]`.
