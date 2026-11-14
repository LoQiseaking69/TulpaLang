# Projects & run configurations

A HermiSim project is a folder. If it contains a `hermisim.json`, HermiSim reads the project's
name, its simulation model, its entry file and its run configurations from it. Without one, the
folder still opens and you can run the current file.

## `hermisim.json`

This is the file shipped with the Morty project (trimmed):

```json
{
  "name": "Morty",
  "description": "Bipedal humanoid control reimagined in TulpaLang.",
  "model": "../../models/morty.xml",
  "entry": "main.rav",
  "run_configurations": [
    {"name": "Morty — balance demo (sim)", "kind": "cosim", "file": "main.rav", "args": ["20"], "realtime": 1.0},
    {"name": "Train residual policy (PPO)", "kind": "cosim", "file": "train.rav", "args": ["40", "resume"], "realtime": 0.0},
    {"name": "FPGA testbench (RTL → VCD)", "kind": "run", "file": "fpga/testbench.rav"},
    {"name": "Mainboard ERC / power / BOM", "kind": "run", "file": "board/motherboard.rav"},
    {"name": "Unit tests", "kind": "test", "file": "tests/morty_test.rav"}
  ]
}
```

### Project fields

| Field | Required | Meaning |
|---|---|---|
| `name` | no | Display name for the explorer and the window title. Defaults to the folder name. |
| `description` | no | Free text. |
| `model` | no | MuJoCo XML model, relative to the project folder. Loaded when the project opens and used by `cosim` configurations. |
| `entry` | no | File opened in the editor when the project opens. |
| `run_configurations` | no | List of run configurations (below). |

### Run configuration fields

| Field | Default | Meaning |
|---|---|---|
| `name` | — | Label in the run-target box and the **Run ▸ Configurations** menu. |
| `kind` | `run` | One of the kinds below. |
| `file` | — | Tulpa file, relative to the project folder. |
| `args` | `[]` | Extra program arguments, available as `ARGS`. |
| `realtime` | `1.0` | For `cosim`: `0` runs as fast as possible, `1` is real time, `2` is 2× and so on. Sets the speed slider. |

The working directory is the project folder.

### Kinds

| Kind | Runs |
|---|---|
| `run` | `tulpa <file> [args]` |
| `test` | `tulpa test <file>` |
| `verify` | `tulpa verify <file>` |
| `check` | `tulpa check <file>` |
| `bytecode` | `tulpa bytecode <file>` |
| `build` | `tulpa build <file> -o <file>.o` |
| `cosim` | Loads `model`, starts a co-simulation server, sets `TULPA_COSIM`, then runs `tulpa <file> [args]`. |

> [!TIP]
> Keep a `cosim` configuration with `realtime: 0` for training or batch experiments, and one at
> `1.0` for watching the robot. Both can point at the same file with different `args`.

## Suggested layout

```text
my-robot/
├── hermisim.json
├── main.rav          # entry point
├── lib/              # import "lib/xyz"
├── tests/            # define_test! files
├── board/            # std/eda board-as-code
├── fpga/             # std/rtl models and testbenches
└── out/              # VCDs, BOMs, netlists written by programs
```

`import` resolves relative to the importing file, then through the library search path, which
includes the toolchain's `std/` and anything on `TULPA_PATH`.
