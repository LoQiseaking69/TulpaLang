# Headless CLI & CI

Everything the IDE runs can also run without a display, which is how HermiSim's own tests and
CI work.

## `tulpa_bridge`

From a source checkout, use `python -m tulpa_bridge`. The installed Windows build ships the
same tool as `hermisim-cli`, and `python main.py run|exec|doctor` is equivalent.

| Command | What it does |
|---|---|
| `run <script> --model <m.xml> [--realtime X] [--timeout S] [--quiet] [-- args...]` | Starts a co-simulation server for the model, sets `TULPA_COSIM`, and runs the script in lock-step. `--realtime 0` (the default) runs as fast as possible. |
| `exec <script> [-- args...]` | Runs a Tulpa program with no simulator. |
| `doctor` | Reports the toolchain, the stdlib, MuJoCo, a co-sim self-test and the render backend. |

```bash
python -m tulpa_bridge run workspace/morty/main.rav --model models/morty.xml -- 20 hard
python -m tulpa_bridge doctor
```

```text
tulpa:   /path/to/TulpaLang/target/release/tulpa
version: tulpa 3.2.0
stdlib:  /path/to/TulpaLang/stdlib
mujoco:  4.0.1
cosim:   ok (tulpa ↔ 127.0.0.1:35881: 100 physics steps, sim t=0.200s)
render:  ok (egl backend, 64x48 frame, 2760 lit px)
```

`doctor` exits non-zero when the toolchain or the co-sim self-test fails. A failed render probe
is only fatal with `--render`, because CI runners usually have no GPU.

## Example GitHub Actions job

```yaml
jobs:
  sim-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { path: HermiSim-DynamicsViewer }
      - uses: actions/checkout@v4
        with: { repository: LoQiseaking69/TulpaLang, path: TulpaLang }
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
        working-directory: TulpaLang
      - uses: actions/setup-python@v5
        with: { python-version: "3.12" }
      - run: pip install -r requirements.txt pytest
        working-directory: HermiSim-DynamicsViewer
      - run: |
          python -m tulpa_bridge doctor
          pytest
          python -m tulpa_bridge run workspace/morty/main.rav --model models/morty.xml --timeout 300 -- 10
        working-directory: HermiSim-DynamicsViewer
        env: { MUJOCO_GL: egl }
```

## IDE smoke test and screenshots

```bash
pytest
QT_QPA_PLATFORM=offscreen MUJOCO_GL=egl python tools/ide_smoke.py screens/
```

`ide_smoke.py` drives the real IDE offscreen and writes screenshots. The images in these docs
were produced that way.
