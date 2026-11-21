# Building the installer

The Windows installer bundles the IDE, MuJoCo, the Tulpa toolchain (`tulpa.exe` + `std/`) and
the VS Code extension into one `RAVIDE-Setup-<version>-x64.exe`.

## Build locally

On Windows, with Rust, Python, Node.js and [Inno Setup 6](https://jrsoftware.org/isinfo.php)
installed, and the Tulpa repository cloned next to .RAV IDE:

```powershell
powershell -ExecutionPolicy Bypass -File installer\windows\build.ps1 -TulpaSrc ..\Tulpa -Version 2.3.0
```

The script:

1. builds Tulpa and the VS Code extension
2. stages the toolchain (`installer/stage_toolchain.py`)
3. renders the brand assets (`tools/make_brand_assets.py`)
4. builds the PyInstaller bundle (`installer/ravide.spec`) and smoke-tests it
5. compiles `installer/windows/ravide.iss` with Inno Setup

## Build in CI

The `windows-installer` workflow does the same on every `v*` tag and attaches the installer to the
GitHub release. If the Tulpa repository is private, add a `TULPA_REPO_TOKEN` secret with read access to it
so the workflow can check it out.

## What the installer does

- Installs per user without admin rights, or for all users.
- Optionally adds `tulpa` to `PATH`.
- Optionally associates `.rav` and `.tlp` files with .RAV IDE.
- Copies the editable projects (including Morty) to `Documents\RAV IDE`.
