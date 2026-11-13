//! Tulpa 3.1: unit-typed native code, CAN bus, Bluetooth LE, timestamped record/replay.

use std::path::Path;
use tulpa::run_captured;

fn run(src: &str) -> String {
    run_captured(src).unwrap_or_else(|e| panic!("{e}\n---\n{src}"))
}

fn tulpa(args: &[&str]) -> (i32, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

// ------------------------------------------------------------------ units in native code

const UNITS: &str = r#"
const WHEEL = 6.5[cm]
fn wheel_rpm(speed: [m/s]) -> [rpm] require speed >= 0 { speed / (PI * WHEEL) * 1[rev] }
fn stop_dist(v: [km/h], decel: [m/s^2]) -> [cm] { v ** 2 / (2 * decel) }
fn travel(v: [m/s], t: [s]) -> [m] { v * t }
fn chain(v: [km/h]) -> [m] { travel(v, 10[s]) }
fn eta_min(dist: [m], v: [km/h]) -> float { (dist / v).in("min") }
fn tilt(a: [deg]) -> float { sin(a) }
fn diag(w: [m], h: [m]) -> [m] { sqrt(w * w + h * h) }
fn bad_add(d: [m], t: [s]) -> [m] { d + t }
fn bad_ret(d: [m]) -> [s] { d * 2 }
fn bad_trig(d: [m]) -> float { cos(d) }
"#;

#[test]
fn native_units_are_checked_at_compile_time() {
    let prog = tulpa::parse(UNITS).unwrap();
    let r = tulpa::llvm::emit_module(&prog, "u");
    let names: Vec<&str> = r.compiled.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "wheel_rpm",
            "stop_dist",
            "travel",
            "chain",
            "eta_min",
            "tilt",
            "diag"
        ]
    );
    let why: std::collections::HashMap<&str, &str> = r
        .skipped
        .iter()
        .map(|(n, w)| (n.as_str(), w.as_str()))
        .collect();
    assert!(
        why["bad_add"].contains("unit mismatch: cannot apply '+' to length [m] and time [s]"),
        "{why:?}"
    );
    assert!(
        why["bad_ret"].contains("unit mismatch: length [m] where time [s] is expected"),
        "{why:?}"
    );
    assert!(
        why["bad_trig"].contains("cos() expects an angle"),
        "{why:?}"
    );
    let h = r.header("U_H");
    assert!(h.contains("double tulpa_stop_dist(double v /* [km/h] */, double decel /* [m/s^2] */); /* returns [cm] */"), "{h}");
}

#[test]
fn native_units_match_the_interpreter() {
    if std::process::Command::new("clang")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("clang not found; skipping");
        return;
    }
    let dir = std::env::temp_dir().join("tulpa_units_native");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("u.rav");
    std::fs::write(&src, UNITS).unwrap();
    let obj = dir.join("u.o");
    let (s, _, e) = tulpa(&["build", src.to_str().unwrap(), "-o", obj.to_str().unwrap()]);
    assert_eq!(s, 0, "{e}");
    let c = "#include <stdio.h>\n#include \"u.h\"\nint main(void){printf(\"%.6f %.6f %.6f %.6f %.6f %.6f %.6f\\n\",tulpa_wheel_rpm(0.5),tulpa_stop_dist(36.0,2.0),tulpa_travel(1.5,4.0),tulpa_chain(36.0),tulpa_eta_min(1200.0,36.0),tulpa_tilt(30.0),tulpa_diag(3.0,4.0));return 0;}\n";
    std::fs::write(dir.join("m.c"), c).unwrap();
    let exe = dir.join("m");
    assert!(std::process::Command::new("clang")
        .args([
            dir.join("m.c").to_str().unwrap(),
            obj.to_str().unwrap(),
            "-lm",
            "-I",
            dir.to_str().unwrap(),
            "-o",
            exe.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    let native =
        String::from_utf8(std::process::Command::new(&exe).output().unwrap().stdout).unwrap();
    let interp_src = UNITS.lines().filter(|l| !l.starts_with("fn bad")).collect::<Vec<_>>().join("\n")
        + "\nprint(format(wheel_rpm(0.5[m/s]).in(\"rpm\"), \".6f\"), format(stop_dist(36[km/h], 2[m/s^2]).in(\"cm\"), \".6f\"), format(travel(1.5[m/s], 4[s]).in(\"m\"), \".6f\"), format(chain(36[km/h]).in(\"m\"), \".6f\"), format(eta_min(1200[m], 36[km/h]), \".6f\"), format(tilt(30[deg]), \".6f\"), format(diag(3[m], 4[m]).in(\"m\"), \".6f\"))";
    assert_eq!(native, run(&interp_src));
}

// ------------------------------------------------------------------ CAN

#[test]
fn can_bus_simulation() {
    let src = r#"
let a = CAN::new("can0")
let b = CAN::new("can0")
let other = CAN::new("can1")
a.send(0x123, [1, 2])
print(b.receive(), a.receive(), other.receive())
b.set_filter(0x200, 0x700)
sim::can_inject("can0", 0x18FF50E5, [1, 2, 3])
sim::can_inject("can0", 0x205, [9])
print(len(a.receive_all()), b.receive_all(), len(sim::can_sent("can0")))
try { a.send(0x123, [0; 9]) } catch e { print(e) }
try { a.send(0x800, [], false) } catch e { print(e) }
"#;
    assert_eq!(
        run(src),
        "{\"id\": 291, \"data\": [1, 2], \"extended\": false} nil nil\n2 [{\"id\": 517, \"data\": [9], \"extended\": false}] 1\nCAN frames carry at most 8 bytes, got 9\nCAN id 0x800 out of range for standard frames\n"
    );
}

// ------------------------------------------------------------------ BLE

const BLE_SETUP: &str = r#"
sim::ble_device("AA:BB:CC:DD:EE:01", "HRM-Pro", {
    "heart_rate": {"heart_rate_measurement": [0, 72]},
    "battery_service": {"battery_level": [87]},
    "nordic_uart": {"nordic_uart_rx": [], "nordic_uart_tx": "x" * 300},
}, -48)
"#;

#[test]
fn ble_gatt_discovery_read_write_notify() {
    let src = format!(
        "{BLE_SETUP}\nlet d = BLE::connect(\"aa:bb:cc:dd:ee:01\")\nprint(BLE::scan(0.1)[0].name, d.services(), d.mtu())\nprint(d.read(\"battery_level\"), d.read_text(\"2a00\"), len(d.read(\"nordic_uart_tx\")))\nd.subscribe(\"heart_rate_measurement\")\nsim::ble_notify(\"AA:BB:CC:DD:EE:01\", \"2a37\", [0, 80])\nprint(d.notifications())\nd.write(\"nordic_uart_rx\", \"GO\")\nd.write(\"6e400002-b5a3-f393-e0a9-e50e24dcca9e\", [1], false)\nprint(sim::ble_written(\"AA:BB:CC:DD:EE:01\", \"nordic_uart_rx\"))\nd.unsubscribe(\"2a37\")\nsim::ble_notify(\"AA:BB:CC:DD:EE:01\", \"2a37\", [0, 81])\nprint(d.notifications(), d.read(\"2a37\"))\ntry {{ d.read(\"2a6e\") }} catch e {{ print(e) }}\nd.disconnect()\ntry {{ d.read(\"2a19\") }} catch e {{ print(e) }}"
    );
    assert_eq!(
        run(&src),
        "HRM-Pro [\"1800\", \"180d\", \"180f\", \"6e400001-b5a3-f393-e0a9-e50e24dcca9e\"] 185\n[87] HRM-Pro 300\n[{\"uuid\": \"2a37\", \"data\": [0, 80]}]\n[1]\n[] [0, 81]\nBLE: device has no characteristic 2a6e\nBLE device AA:BB:CC:DD:EE:01 is disconnected\n"
    );
}

#[test]
fn ble_uuid_normalisation() {
    use tulpa::devices::ble_normalize_uuid as n;
    assert_eq!(n("0x2A19").unwrap(), "2a19");
    assert_eq!(n("00002a19-0000-1000-8000-00805f9b34fb").unwrap(), "2a19");
    assert_eq!(n("battery_level").unwrap(), "2a19");
    assert_eq!(
        n("6E400001-B5A3-F393-E0A9-E50E24DCCA9E").unwrap(),
        "6e400001-b5a3-f393-e0a9-e50e24dcca9e"
    );
    assert!(n("nope").is_err());
}

// ------------------------------------------------------------------ timestamped record/replay

#[test]
fn timed_replay_reproduces_deadline_outcomes() {
    let dir = std::env::temp_dir().join("tulpa_timed_replay");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("cell.rav");
    std::fs::write(
        &script,
        r#"
let motor = GPIO::new(22)
let limit = GPIO::new(23, "in")
let late = []
after motor.read() == 1 within 150[ms] ensure limit.read() == 1 else { late.push(millis()) }
let travel_ms = if len(ARGS) > 0 { int(ARGS[0]) } else { 100 }
let started = signal(-1)
when started.get() >= 0 and millis() - started.get() >= travel_ms { sim::gpio_input(23, 1) }
motor.set_high()
started.set(millis())
let t0 = millis()
while limit.read() == 0 and millis() - t0 < 1000 { sleep(5[ms]) }
motor.set_low()
print(len(late))
"#,
    )
    .unwrap();
    let s = script.to_str().unwrap();
    for (travel, late) in [("40", "0"), ("230", "1")] {
        let tape = dir.join(format!("run{travel}.tape"));
        let t = tape.to_str().unwrap();
        // record on the virtual clock so the test is fast and deterministic
        let (c, out, err) = tulpa(&["--virtual-time", "--record", t, s, travel]);
        assert_eq!((c, out.trim()), (0, late), "{err}");
        let text = std::fs::read_to_string(&tape).unwrap();
        assert!(
            text.starts_with("# tulpa device tape v2") && text.lines().count() < 20,
            "{text}"
        );
        // replay: inputs and timing come from the tape; the simulated mechanics are disabled
        let (c, out, err) = tulpa(&["--replay", t, s, "99999"]);
        assert_eq!(
            (c, out.trim()),
            (0, late),
            "replay of {travel} ms run: {err}"
        );
        assert!(
            err.contains("replay matched all 4 device commands"),
            "{err}"
        );
    }
}

#[test]
fn ble_and_can_record_replay() {
    let dir = std::env::temp_dir().join("tulpa_ble_tape");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("s.rav");
    std::fs::write(
        &script,
        format!("if len(ARGS) > 0 {{ {BLE_SETUP} }}\nlet d = BLE::connect(\"AA:BB:CC:DD:EE:01\")\nprint(d.read(\"battery_level\"))\nd.write(\"nordic_uart_rx\", \"hi\")\nlet c = CAN::new(\"can0\")\nc.send(0x10, [1])\nprint(\"ok\")\n"),
    )
    .unwrap();
    let tape = dir.join("s.tape");
    let (c, out, err) = tulpa(&[
        "--record",
        tape.to_str().unwrap(),
        script.to_str().unwrap(),
        "seed",
    ]);
    assert_eq!((c, out.as_str()), (0, "[87]\nok\n"), "{err}");
    // no simulated device exists on replay: every answer comes from the tape
    let (c, out, err) = tulpa(&["--replay", tape.to_str().unwrap(), script.to_str().unwrap()]);
    assert_eq!((c, out.as_str()), (0, "[87]\nok\n"), "{err}");
}

#[test]
fn vscode_extension_files_are_valid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("editors/vscode");
    for f in [
        "package.json",
        "language-configuration.json",
        "syntaxes/tulpa.tmLanguage.json",
    ] {
        let text = std::fs::read_to_string(root.join(f)).unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(|e| panic!("{f}: {e}"));
    }
    let pkg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("package.json")).unwrap()).unwrap();
    assert_eq!(pkg["version"], env!("CARGO_PKG_VERSION"));
}
