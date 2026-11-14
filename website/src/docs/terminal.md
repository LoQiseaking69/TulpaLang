# Tulpa terminal

The **Tulpa Terminal** tab in the bottom dock is a live `tulpa` REPL, started in the project
folder and docked beside the console.

## Input modes

| You type | What happens |
|---|---|
| any Tulpa code | Evaluated in the REPL. Multi-line input continues with a `...` prompt. |
| `$ <args>` | Runs a `tulpa` command, e.g. `$ check main.rav`, `$ verify lib/pinn.rav`, `$ graph main.rav`. |
| `! <cmd>` | Runs a shell command (`$SHELL -c` on Linux/macOS, `cmd /c` on Windows). |
| `:clear` / `clear` / `cls` | Clears the view. |
| `:restart` / `:reset` | Restarts the REPL. |

While a `$` or `!` job is running, anything you type is sent to its stdin. Each job ends with an
`── exit N` line, green for 0 and red otherwise.

## From the editor

Press **Ctrl+Enter** in the editor to send the current selection to the REPL. It's a quick way to
poke at a struct or check a unit conversion without running the whole program.

```text
› let v = 0.5[m/s]
› v.to("km/h")
1.8 km/h
› $ verify lib/balance.rav
```

The REPL also understands its own commands: `:log`, `:reset` and `:quit`.
