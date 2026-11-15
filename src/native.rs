//! Native builds: compilation targets, the C runtime that ships inside the `tulpa` binary,
//! import flattening for whole-program compilation, and firmware project scaffolding.
//!
//! `tulpa build app.rav --target rp2040` compiles a whole program (structs, devices, machines,
//! `every!` loops, `always` monitors) into an object file plus a C header; the board's HAL
//! port and `tulpa_rt.c` (written by `tulpa new` or `tulpa runtime`) supply the device layer.

use crate::ast::Stmt;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The C runtime, embedded so a single `tulpa` binary can scaffold and build firmware.
pub const HAL_H: &str = include_str!("../runtime/tulpa_hal.h");
pub const RT_C: &str = include_str!("../runtime/tulpa_rt.c");
pub const HAL_SIM: &str = include_str!("../runtime/hal/sim.c");
pub const HAL_LINUX: &str = include_str!("../runtime/hal/linux.c");
pub const HAL_PICO: &str = include_str!("../runtime/hal/pico.c");
pub const HAL_ESP_IDF: &str = include_str!("../runtime/hal/esp_idf.c");

const T_APP: &str = include_str!("../runtime/templates/app.rav");
const T_PICO_CMAKE: &str = include_str!("../runtime/templates/rp2040/CMakeLists.txt");
const T_PICO_MAIN: &str = include_str!("../runtime/templates/rp2040/main.c");
const T_ESP_CMAKE: &str = include_str!("../runtime/templates/esp_idf/CMakeLists.txt");
const T_ESP_MAIN_CMAKE: &str = include_str!("../runtime/templates/esp_idf/main/CMakeLists.txt");
const T_ESP_MAIN: &str = include_str!("../runtime/templates/esp_idf/main/main.c");
const T_ESP_SDKCONFIG: &str = include_str!("../runtime/templates/esp_idf/sdkconfig.defaults");
const T_LINUX_MAKE: &str = include_str!("../runtime/templates/linux/Makefile");

/// Which board HAL a target links against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hal {
    Sim,
    Linux,
    Pico,
    EspIdf,
    /// Bare object for your own HAL (generic Cortex-M / RISC-V targets).
    Custom,
}

impl Hal {
    pub fn source(self) -> Option<(&'static str, &'static str)> {
        match self {
            Hal::Sim => Some(("hal_sim.c", HAL_SIM)),
            Hal::Linux => Some(("hal_linux.c", HAL_LINUX)),
            Hal::Pico => Some(("hal_pico.c", HAL_PICO)),
            Hal::EspIdf => Some(("hal_esp_idf.c", HAL_ESP_IDF)),
            Hal::Custom => None,
        }
    }
}

/// A compilation target for `tulpa build --target <name>`.
#[derive(Clone, Debug)]
pub struct Target {
    pub name: &'static str,
    pub about: &'static str,
    /// clang flags selecting the architecture (empty = the host).
    pub cflags: &'static [&'static str],
    /// Freestanding: violations go to tulpa_hal_panic, code is built small (-Os).
    pub embedded: bool,
    pub hal: Hal,
    /// Needs a clang with the Xtensa backend (Espressif's esp-clang).
    pub xtensa: bool,
    /// Can be linked into a host executable with `--exe`.
    pub exe: bool,
}

pub const TARGETS: &[Target] = &[
    Target {
        name: "sim",
        about: "host executable on the simulated bus with a virtual clock (tests, CI)",
        cflags: &[],
        embedded: false,
        hal: Hal::Sim,
        xtensa: false,
        exe: true,
    },
    Target {
        name: "linux",
        about: "host executable on real Linux hardware (Raspberry Pi and other SBCs)",
        cflags: &[],
        embedded: false,
        hal: Hal::Linux,
        xtensa: false,
        exe: true,
    },
    Target {
        name: "rp2040",
        about: "Raspberry Pi Pico / RP2040 (Cortex-M0+), Pico SDK",
        cflags: &[
            "--target=thumbv6m-none-eabi",
            "-mcpu=cortex-m0plus",
            "-mfloat-abi=soft",
        ],
        embedded: true,
        hal: Hal::Pico,
        xtensa: false,
        exe: false,
    },
    Target {
        name: "esp32",
        about: "ESP32 (Xtensa LX6), ESP-IDF; needs esp-clang",
        cflags: &["--target=xtensa-esp-elf", "-mcpu=esp32"],
        embedded: true,
        hal: Hal::EspIdf,
        xtensa: true,
        exe: false,
    },
    Target {
        name: "esp32s2",
        about: "ESP32-S2 (Xtensa LX7), ESP-IDF; needs esp-clang",
        cflags: &["--target=xtensa-esp-elf", "-mcpu=esp32s2"],
        embedded: true,
        hal: Hal::EspIdf,
        xtensa: true,
        exe: false,
    },
    Target {
        name: "esp32s3",
        about: "ESP32-S3 (Xtensa LX7), ESP-IDF; needs esp-clang",
        cflags: &["--target=xtensa-esp-elf", "-mcpu=esp32s3"],
        embedded: true,
        hal: Hal::EspIdf,
        xtensa: true,
        exe: false,
    },
    Target {
        name: "esp32c3",
        about: "ESP32-C3 (RISC-V rv32imc), ESP-IDF",
        cflags: &[
            "--target=riscv32-unknown-elf",
            "-march=rv32imc_zicsr_zifencei",
            "-mabi=ilp32",
        ],
        embedded: true,
        hal: Hal::EspIdf,
        xtensa: false,
        exe: false,
    },
    Target {
        name: "esp32c6",
        about: "ESP32-C6 (RISC-V rv32imac), ESP-IDF",
        cflags: &[
            "--target=riscv32-unknown-elf",
            "-march=rv32imac_zicsr_zifencei",
            "-mabi=ilp32",
        ],
        embedded: true,
        hal: Hal::EspIdf,
        xtensa: false,
        exe: false,
    },
    Target {
        name: "cortex-m0",
        about: "generic Cortex-M0/M0+ object for your own HAL (tulpa runtime <dir>)",
        cflags: &[
            "--target=thumbv6m-none-eabi",
            "-mcpu=cortex-m0plus",
            "-mfloat-abi=soft",
        ],
        embedded: true,
        hal: Hal::Custom,
        xtensa: false,
        exe: false,
    },
    Target {
        name: "cortex-m4f",
        about: "generic Cortex-M4F (STM32F4, nRF52, ...) object for your own HAL",
        cflags: &[
            "--target=thumbv7em-none-eabihf",
            "-mcpu=cortex-m4",
            "-mfloat-abi=hard",
            "-mfpu=fpv4-sp-d16",
        ],
        embedded: true,
        hal: Hal::Custom,
        xtensa: false,
        exe: false,
    },
    Target {
        name: "riscv32",
        about: "generic RV32IMAC object for your own HAL",
        cflags: &[
            "--target=riscv32-unknown-elf",
            "-march=rv32imac",
            "-mabi=ilp32",
        ],
        embedded: true,
        hal: Hal::Custom,
        xtensa: false,
        exe: false,
    },
];

pub fn target(name: &str) -> Option<&'static Target> {
    TARGETS
        .iter()
        .find(|t| t.name == name || (name == "pico" && t.name == "rp2040"))
}

/// Writes tulpa_hal.h, tulpa_rt.c and (if any) the target's HAL port into `dir`.
pub fn write_runtime(dir: &Path, hal: Hal) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut files = vec![("tulpa_hal.h", HAL_H), ("tulpa_rt.c", RT_C)];
    match hal.source() {
        Some(src) => files.push(src),
        None => {
            // a custom port starts from the simulated one, which documents every function
            files.push(("hal_template.c", HAL_SIM));
        }
    }
    let mut out = Vec::new();
    for (name, body) in files {
        let p = dir.join(name);
        std::fs::write(&p, body)?;
        out.push(p);
    }
    Ok(out)
}

/// `tulpa new <dir> --board <board>`: a firmware project that builds app.rav for the board.
pub fn new_project(dir: &Path, board: &str) -> Result<Vec<PathBuf>, String> {
    let t = target(board).ok_or_else(|| {
        format!(
            "unknown board '{board}' (use one of: {})",
            TARGETS
                .iter()
                .filter(|t| t.hal != Hal::Custom && t.name != "sim")
                .map(|t| t.name)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    if dir.exists()
        && std::fs::read_dir(dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        return Err(format!("{} already exists and is not empty", dir.display()));
    }
    let name: String = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "app".into())
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let (led, button) = match t.hal {
        Hal::Pico => ("25", "15"),
        Hal::EspIdf => ("2", "0"),
        _ => ("17", "27"),
    };
    let fill = |s: &str| {
        s.replace("@NAME@", &name)
            .replace("@TARGET@", t.name)
            .replace(
                "@BOARD@",
                t.about.split(',').next().unwrap_or(t.name).trim(),
            )
            .replace("@LED@", led)
            .replace("@BUTTON@", button)
    };
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dir).map_err(io)?;
    let mut out = Vec::new();
    let mut put = |rel: &str, body: String| -> Result<(), String> {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        std::fs::write(&p, body).map_err(io)?;
        out.push(p);
        Ok(())
    };
    match t.hal {
        Hal::Pico => {
            put("app.rav", fill(T_APP))?;
            put("CMakeLists.txt", fill(T_PICO_CMAKE))?;
            put("main.c", fill(T_PICO_MAIN))?;
            put("tulpa/tulpa_hal.h", HAL_H.into())?;
            put("tulpa/tulpa_rt.c", RT_C.into())?;
            put("tulpa/hal_pico.c", HAL_PICO.into())?;
        }
        Hal::EspIdf => {
            put("app.rav", fill(T_APP))?;
            put("CMakeLists.txt", fill(T_ESP_CMAKE))?;
            put("sdkconfig.defaults", fill(T_ESP_SDKCONFIG))?;
            put("main/CMakeLists.txt", fill(T_ESP_MAIN_CMAKE))?;
            put("main/main.c", fill(T_ESP_MAIN))?;
            put("main/tulpa/tulpa_hal.h", HAL_H.into())?;
            put("main/tulpa/tulpa_rt.c", RT_C.into())?;
            put("main/tulpa/hal_esp_idf.c", HAL_ESP_IDF.into())?;
        }
        Hal::Linux | Hal::Sim => {
            put("app.rav", fill(T_APP))?;
            put("Makefile", fill(T_LINUX_MAKE))?;
        }
        Hal::Custom => {
            return Err(format!(
                "{board} is a generic target; use `tulpa runtime <dir> --target {board}`"
            ))
        }
    }
    Ok(out)
}

/// Whole-program compilation: merges the definitions of every imported module (recursively,
/// each module once) ahead of the program. Returns the merged program and the names bound to
/// modules (`import "x" as m` → `m`), which the code generator resolves statically.
pub fn flatten_imports(prog: Vec<Stmt>, file: &Path) -> Result<(Vec<Stmt>, Vec<String>), String> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut aliases = Vec::new();
    let mut defs = Vec::new();
    let base = file.parent().unwrap_or(Path::new(".")).to_path_buf();
    collect(&prog, &base, &mut seen, &mut aliases, &mut defs)?;
    let mut names: HashSet<String> = prog.iter().filter_map(def_name).collect();
    let mut out: Vec<Stmt> = Vec::new();
    for d in defs {
        if let Some(n) = def_name(&d) {
            if !names.insert(n) {
                continue; // the program's own definition (or an earlier module's) wins
            }
        }
        out.push(d);
    }
    out.extend(prog);
    Ok((out, aliases))
}

fn def_name(s: &Stmt) -> Option<String> {
    match s {
        Stmt::Fn(d) => Some(d.name.clone()),
        Stmt::Struct(d) => Some(d.name.clone()),
        Stmt::Interface(d) => Some(d.name.clone()),
        Stmt::Machine(d) => Some(d.name.clone()),
        Stmt::Let(n, _, true, _) if n.len() == 1 => Some(n[0].clone()),
        _ => None,
    }
}

fn resolve(path: &str, base: &Path) -> Option<PathBuf> {
    let mut full = base.join(path);
    if !full.exists() && full.extension().is_none() {
        full.set_extension("rav");
    }
    if full.exists() {
        return Some(full);
    }
    crate::stdx_path::resolve(path)
}

fn collect(
    prog: &[Stmt],
    base: &Path,
    seen: &mut HashSet<PathBuf>,
    aliases: &mut Vec<String>,
    defs: &mut Vec<Stmt>,
) -> Result<(), String> {
    for s in prog {
        let Stmt::Import(path, alias, names, _) = s else {
            continue;
        };
        let file = resolve(path, base).ok_or_else(|| format!("module \"{path}\" not found"))?;
        if names.is_empty() {
            aliases.push(match alias {
                Some(a) => a.clone(),
                None => file
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            });
        }
        let canon = file.canonicalize().unwrap_or(file.clone());
        if !seen.insert(canon.clone()) {
            continue;
        }
        let src = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let module = crate::parser::parse(&src)
            .map_err(|e| format!("{}:{}: {}", file.display(), e.line, e.msg))?;
        let mbase = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        collect(&module, &mbase, seen, aliases, defs)?;
        for m in module {
            match &m {
                Stmt::Fn(_) | Stmt::Struct(_) | Stmt::Interface(_) | Stmt::Machine(_) => {
                    defs.push(m)
                }
                Stmt::Let(_, _, true, _) => defs.push(m),
                _ => {}
            }
        }
    }
    Ok(())
}
