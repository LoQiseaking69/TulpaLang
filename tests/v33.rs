//! Tulpa 3.3: interfaces and the driver SDK, native structs/devices/machines, embedded targets.

use std::path::{Path, PathBuf};
use std::process::Command;
use tulpa::run_captured;

fn run(src: &str) -> String {
    run_captured(src).unwrap_or_else(|e| panic!("{e}\n---\n{src}"))
}

fn tulpa(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tulpa-v33-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ------------------------------------------------------------------ interfaces

const IFACE: &str = r#"
interface Sensor {
    fn read(self)
    fn name(self)
}
interface Thermometer: Sensor {
    fn celsius(self) -> [K]
}
struct Fake impl Thermometer {
    t: [K] = 21[K]
    fn read(self) => self.t
    fn name(self) => "fake"
    fn celsius(self) -> [K] { self.t }
}
fn show(s: Sensor) { print(s.name(), s.read()) }
let f = Fake()
show(f)
print(implements(f, "Sensor"), implements(f, "Thermometer"), implements(3, "Sensor"))
try { show(5) } catch e { print("E1", e) }
try {
    struct Bad impl Thermometer { fn read(self) => 1 }
} catch e { print("E2", e) }
try {
    struct Bad2 impl Thermometer { fn read(self) => 1; fn name(self) => "x"; fn celsius(self) -> [m] { 1[m] } }
} catch e { print("E3", e) }
"#;

#[test]
fn interfaces_are_checked_and_usable_as_types() {
    let out = run(IFACE);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "fake 21 K");
    assert_eq!(lines[1], "true true false");
    assert!(
        lines[2].starts_with("E1") && lines[2].contains("expects Sensor"),
        "{out}"
    );
    assert!(
        lines[3].contains("Bad implements Thermometer but has no method"),
        "{out}"
    );
    assert!(lines[4].contains("must return [K]"), "{out}");
}

#[test]
fn interface_methods_cannot_have_bodies() {
    let err = tulpa::parse("interface I { fn f(self) { 1 } }")
        .expect_err("should fail")
        .msg;
    assert!(err.to_lowercase().contains("interface"), "{err}");
}

// ------------------------------------------------------------------ driver SDK (interpreter)

#[test]
fn driver_helpers_and_regmap() {
    let out = run(r#"
from "std/driver" import Regmap, bits, with_bits, bit, twos, u16_be, i16_le, lo8, hi8
let r = Regmap(I2C::new(1, 0x50))
r.write(0x10, 0b1010_0101)
r.set_field(0x10, 4, 4, 0x3)
print(r.field(0x10, 4, 4), r.read(0x10))
print(bits(0xF0, 4, 4), with_bits(0, 2, 3, 5), bit(4, 2), twos(0xFFFE, 16))
print(u16_be(0x12, 0x34), i16_le(0x00, 0x80), lo8(0x1234), hi8(0x1234))
"#);
    assert_eq!(out, "3 53\n15 20 true -2\n4660 -32768 52 18\n");
}

#[test]
fn bundled_drivers_run_against_the_simulator() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/drivers");
    let src = std::fs::read_to_string(dir.join("main.rav")).unwrap();
    let out = tulpa::run_captured_in(&src, &dir).unwrap();
    assert!(out.contains("probe true true true"), "{out}");
    assert!(out.contains("az 9.8066 m/s^2  gx 1.000 deg/s"), "{out}");
    assert!(out.contains("prescale [121]"), "{out}");
    assert!(out.contains("raw 512"), "{out}");
}

// ------------------------------------------------------------------ native compilation

fn native_matches_interpreter(name: &str, src: &str) {
    if !have("clang") {
        eprintln!("clang not found; skipping native differential test {name}");
        return;
    }
    let d = tmpdir(name);
    let file = d.join("main.rav");
    std::fs::write(&file, src).unwrap();
    let f = file.to_str().unwrap();
    let exe = d.join("prog");
    let (code, _, err) = tulpa(&[
        "build",
        f,
        "--target",
        "sim",
        "--exe",
        "-o",
        exe.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "native build failed:\n{err}");
    let native = Command::new(&exe).output().unwrap();
    let (icode, iout, ierr) = tulpa(&[f, "--virtual-time"]);
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        iout,
        "native and interpreter output differ ({name})\n{ierr}"
    );
    assert_eq!(
        native.status.code().unwrap_or(-1) == 0,
        icode == 0,
        "exit status differs ({name})"
    );
}

#[test]
fn native_structs_machines_and_monitors() {
    native_matches_interpreter(
        "machine",
        r#"
struct Joint {
    name: str
    angle: [deg] = 0[deg]
    limit: [deg] = 90[deg]
    invariant self.angle <= self.limit
    fn move_by(self, d: [deg]) { self.angle = self.angle + d }
}
machine Gripper {
    state Open { on grab => Closed }
    state Closed { on release => Open }
}
let j = Joint("elbow")
let g = Gripper()
let hits = 0
always j.angle < 80[deg] else { print("near limit", j.angle) }
fn step(n: int, by: [deg] = 15[deg]) -> int {
    let k = 0
    while k < n { j.move_by(by); k = k + 1 }
    return k
}
print(step(3), j.angle.to("deg"), g.state)
g.grab()
print(g.state, g.state == "Closed")
let led = GPIO::new(25)
every!(10[ms], 3, { led.toggle() })
print(f"{j.name}: {j.angle.to("rad"):.3f} {format(time::millis(), "d")}")
print(step(n: 2, by: 5[deg]), j.angle.to("deg"))
"#,
    );
}

#[test]
fn native_driver_library_matches_interpreter() {
    if !have("clang") {
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/drivers");
    let d = tmpdir("drivers");
    let exe = d.join("drivers");
    let main = dir.join("main.rav");
    let (code, _, err) = tulpa(&[
        "build",
        main.to_str().unwrap(),
        "--target",
        "sim",
        "--exe",
        "-o",
        exe.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{err}");
    let out = Command::new(&exe).current_dir(&dir).output().unwrap();
    let want = std::fs::read_to_string(dir.join("expected.txt")).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), want);
}

#[test]
fn native_header_describes_structs_and_devices() {
    let prog = tulpa::parse(
        r#"
interface Rangefinder { fn distance(self) -> [m] }
struct Sonar impl Rangefinder {
    trig: GPIO
    range: [m] = 4[m]
    fn distance(self) -> [m] { if self.trig.read() { self.range } else { 0[m] } }
}
fn closest(s: Sonar, other: [m]) -> [m] { if s.distance() < other { s.distance() } else { other } }
"#,
    )
    .unwrap();
    let r = tulpa::llvm::emit_module(&prog, "sonar");
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    assert!(
        r.ir.contains("%S.Sonar = type { %tulpa.gpio, double }"),
        "{}",
        r.ir
    );
    assert!(
        r.ir.contains("define double @tulpa_Sonar_distance(ptr"),
        "{}",
        r.ir
    );
    let h = r.header("SONAR_H");
    assert!(h.contains("typedef struct tulpa_Sonar"), "{h}");
    assert!(h.contains("tulpa_gpio"), "{h}");
    assert!(h.contains("#include \"tulpa_hal.h\""), "{h}");
}

#[test]
fn unsupported_native_features_are_explained() {
    let prog =
        tulpa::parse("fn f() { try { 1 } catch e { 2 } }\nfn g(d: dict) { print(d) }").unwrap();
    let r = tulpa::llvm::emit_module(&prog, "m");
    let reasons: Vec<&str> = r.skipped.iter().map(|(_, why)| why.as_str()).collect();
    assert!(
        reasons.iter().any(|w| w.contains("try/catch")),
        "{reasons:?}"
    );
    assert_eq!(r.skipped.len(), 2, "{reasons:?}");
}

// ------------------------------------------------------------------ embedded targets & tooling

#[test]
fn targets_command_lists_boards() {
    let (code, out, _) = tulpa(&["targets"]);
    assert_eq!(code, 0);
    for t in [
        "rp2040",
        "esp32",
        "esp32s3",
        "esp32c3",
        "esp32c6",
        "cortex-m4f",
        "linux",
        "sim",
    ] {
        assert!(out.contains(t), "missing {t}:\n{out}");
    }
}

#[test]
fn new_project_scaffolds_each_board() {
    for (board, files) in [
        ("rp2040", &["CMakeLists.txt", "main.c", "app.rav"][..]),
        (
            "esp32c3",
            &[
                "CMakeLists.txt",
                "sdkconfig.defaults",
                "main/CMakeLists.txt",
                "main/main.c",
                "app.rav",
            ][..],
        ),
        ("linux", &["Makefile", "app.rav"][..]),
    ] {
        let d = tmpdir(&format!("new-{board}"));
        let p = d.join("proj");
        let (code, out, err) = tulpa(&["new", p.to_str().unwrap(), "--board", board]);
        assert_eq!(code, 0, "{board}: {out}{err}");
        for f in files {
            assert!(p.join(f).exists(), "{board}: missing {f}");
        }
        let app = std::fs::read_to_string(p.join("app.rav")).unwrap();
        assert!(!app.contains('@'), "{board}: unreplaced placeholder\n{app}");
        tulpa::parse(&app)
            .unwrap_or_else(|e| panic!("{board}: template does not parse: {}", e.msg));
    }
}

#[test]
fn runtime_command_writes_hal_sources() {
    let d = tmpdir("rt");
    let (code, _, err) = tulpa(&["runtime", d.to_str().unwrap(), "--target", "rp2040"]);
    assert_eq!(code, 0, "{err}");
    assert!(d.join("tulpa_hal.h").exists());
    assert!(d.join("tulpa_rt.c").exists());
    let hal = std::fs::read_dir(&d)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().contains("pico"));
    assert!(hal, "pico HAL not written");
}

#[test]
fn cross_compiles_for_cortex_m_and_riscv() {
    if !have("clang") {
        return;
    }
    let d = tmpdir("cross");
    let f = d.join("app.rav");
    std::fs::write(
        &f,
        "let led = GPIO::new(25)\nfn blink(n: int) { let i = 0\n while i < n { led.toggle(); time::sleep(100[ms]); i = i + 1 } }\nblink(3)\n",
    )
    .unwrap();
    for t in ["rp2040", "cortex-m4f", "esp32c3"] {
        let o = d.join(format!("app-{t}.o"));
        let (code, _, err) = tulpa(&[
            "build",
            f.to_str().unwrap(),
            "--target",
            t,
            "-o",
            o.to_str().unwrap(),
        ]);
        if code != 0 && err.contains("No available targets") {
            eprintln!("clang lacks the {t} backend; skipping");
            continue;
        }
        assert_eq!(code, 0, "{t}: {err}");
        let bytes = std::fs::read(&o).unwrap();
        assert_eq!(&bytes[..4], b"\x7fELF", "{t}: not an ELF object");
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        let want = if t == "esp32c3" { 243 } else { 40 }; // EM_RISCV / EM_ARM
        assert_eq!(machine, want, "{t}: wrong e_machine");
    }
}

#[test]
fn sim_hooks_are_rejected_on_real_hardware() {
    let d = tmpdir("simhook");
    let f = d.join("a.rav");
    std::fs::write(&f, "sim::i2c_set(0x68, 0x75, [0x68])\nprint(1)\n").unwrap();
    let o = d.join("a.o");
    let (code, _, err) = tulpa(&[
        "build",
        f.to_str().unwrap(),
        "--target",
        "rp2040",
        "-o",
        o.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(err.contains("sim"), "{err}");
}
