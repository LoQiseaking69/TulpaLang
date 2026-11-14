# Installation

Tulpa ships as a single binary, `tulpa`, plus the `std/` library. HermiSim bundles that toolchain,
so on Windows the IDE installer is the quickest way to get both.

## HermiSim on Windows (recommended)

Download `HermiSim-Setup-<version>-x64.exe` from
[HermiSim releases](https://github.com/LoQiseaking69/HermiSim-DynamicsViewer/releases).
The installer:

- bundles the IDE, MuJoCo and the Tulpa toolchain (`tulpa.exe` + `std/`)
- can add `tulpa` to your `PATH` and associate `.rav` / `.tlp` files
- installs per user without admin rights, or for all users

Editable projects, including Morty, are copied to `Documents\HermiSim`.

## Tulpa release packages

Release packages are built by the `release` workflow on every `v*` tag and attached to
[Tulpa releases](https://github.com/LoQiseaking69/TulpaLang/releases).

| Platform | Package | Install |
|---|---|---|
| Linux x86_64 / arm64 / armv7 (Raspberry Pi 3/4/5) | `tulpa-<version>-<target>.tar.gz` | unpack, then `./install.sh` or `sudo ./install.sh /usr/local` |
| Debian / Ubuntu / Raspberry Pi OS | `tulpa_<version>_<arch>.deb` | `sudo apt install ./tulpa_<version>_<arch>.deb` |
| macOS (Intel / Apple silicon) | `tulpa-<version>-<x86_64\|aarch64>-apple-darwin.tar.gz` | unpack and put `tulpa` on your `PATH` |
| Windows | HermiSim installer | see above, or build from source |
| VS Code | `tulpa-lang-<version>.vsix` | `code --install-extension tulpa-lang-<version>.vsix` |

> [!NOTE]
> The latest published release is **v3.1.1**. Until a `v3.2.0` tag is pushed, build from source
> (or use the toolchain bundled with HermiSim) to get the 3.2 libraries: `nn::`, `pack::`,
> `json::`, `tlm::`, `cosim::` and `std/`.

## Build Tulpa from source

Requires Rust **1.82** or later.

```bash
git clone https://github.com/LoQiseaking69/TulpaLang.git
cd TulpaLang
cargo build --release
./target/release/tulpa --version
```

Install it with `scripts/deploy.sh [PREFIX]`. `scripts/release.sh` builds the tarball, `.deb` and
`.vsix` for the host, and `scripts/cross_compile.sh` cross-compiles for a Raspberry Pi.

Native compilation (`tulpa build`) additionally needs **clang 14+** on your `PATH`.

## Run HermiSim from source (Linux, macOS, Windows)

Clone both repositories side by side. HermiSim looks for `../TulpaLang/target/release/tulpa`
automatically.

```bash
git clone https://github.com/LoQiseaking69/HermiSim-DynamicsViewer.git
git clone https://github.com/LoQiseaking69/TulpaLang.git
(cd TulpaLang && cargo build --release)
cd HermiSim-DynamicsViewer
pip install -r requirements.txt
python main.py                      # the IDE
python main.py --classic            # the original model viewer
python -m tulpa_bridge doctor       # check the toolchain and MuJoCo
```

### Pointing HermiSim at a toolchain

HermiSim searches, in order: the path configured under **Settings ▸ Tulpa Toolchain**, the
`TULPA_BIN` environment variable, then `tulpa` on your `PATH` and the bundled / sibling builds.
Run `python -m tulpa_bridge doctor` to see which one it picked.

## Verify the install

```bash
tulpa --version
echo 'print(f"{0.5[m/s].to("km/h"):.1f}")' > hello.rav
tulpa hello.rav          # 1.8 km/h
```
