//! The dynamic tier: programs outside the static native subset run on the Tulpa VM for
//! firmware (runtime/dyn, shipped as tulpa_dyn.c). Each program here is built for the
//! simulated board and must print exactly what the interpreter prints.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tulpa(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .args(args)
        .env_remove("TULPA_LINK")
        .output()
        .expect("run tulpa");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn have_clang() -> bool {
    [
        "clang", "clang-18", "clang-17", "clang-16", "clang-15", "clang-14",
    ]
    .iter()
    .any(|c| {
        Command::new(c)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn work(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tulpa-dyn-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Builds `src` for `target` on the dynamic tier and returns the executable's path.
fn build(src: &Path, target: &str, dir: &Path) -> PathBuf {
    let exe = dir.join("prog");
    let (code, _, err) = tulpa(&[
        "build",
        "--target",
        target,
        "--exe",
        "--dynamic",
        src.to_str().unwrap(),
        "-o",
        exe.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "building {}: {err}", src.display());
    assert!(err.contains("dynamic tier"), "{err}");
    exe
}

fn same_as_interpreter(src: &str) {
    if !have_clang() {
        eprintln!("skipping {src}: clang not found");
        return;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(src);
    let (code, want, err) = tulpa(&["run", "--virtual-time", path.to_str().unwrap()]);
    assert_eq!(code, 0, "interpreter failed on {src}: {err}");
    let dir = work(&src.replace(['/', '.'], "_"));
    let exe = build(&path, "sim", &dir);
    let out = Command::new(&exe).output().expect("run firmware build");
    let got = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{src} failed on the VM: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        got, want,
        "{src}: the firmware VM and the interpreter disagree"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

macro_rules! conformance {
    ($($name:ident => $path:expr,)*) => {
        $(#[test] fn $name() { same_as_interpreter($path); })*
    };
}

conformance! {
    core_values => "tests/dyn/core.rav",
    functions_and_contracts => "tests/dyn/functions.rav",
    structs_and_interfaces => "tests/dyn/structs.rav",
    control_flow_macros_tasks_modules => "tests/dyn/control.rav",
    signals_rules_machines_timing => "tests/dyn/reactive.rav",
    physical_units => "tests/dyn/units.rav",
    devices_on_the_simulated_board => "tests/dyn/devices.rav",
    stdlib_pack_json_link_frame => "tests/dyn/stdlib.rav",
    example_autonomy => "examples/autonomy/main.rav",
    example_sim_rover => "examples/sim_rover/main.rav",
    example_drivers => "examples/drivers/main.rav",
    example_warehouse_robot => "examples/warehouse_robot/main.rav",
    example_tour => "examples/tour/main.rav",
    example_robot_arm => "examples/robot_arm/main.rav",
    example_modules => "examples/modules/main.rav",
    example_timing => "examples/timing/main.rav",
}

#[test]
fn microcontroller_builds_fall_back_to_the_vm() {
    if !have_clang() {
        return;
    }
    let dir = work("fallback");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dyn/control.rav");
    for target in ["rp2040", "esp32c3", "cortex-m4f"] {
        let obj = dir.join(format!("{target}.o"));
        let (code, _, err) = tulpa(&[
            "build",
            "--target",
            target,
            src.to_str().unwrap(),
            "-o",
            obj.to_str().unwrap(),
        ]);
        assert!(err.contains("dynamic tier"), "{target}: {err}");
        if err.contains("No available targets are compatible") {
            continue; // this clang (Apple's) has no backend for the board's CPU
        }
        assert_eq!(code, 0, "{target}: {err}");
        assert!(err.contains("tulpa_dyn.c"), "{target}: {err}");
        assert!(obj.exists());
    }
    // a program inside the static subset still compiles to native code
    let native = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/native/main.rav");
    let obj = dir.join("native.o");
    let (code, _, err) = tulpa(&[
        "build",
        "--target",
        "rp2040",
        native.to_str().unwrap(),
        "-o",
        obj.to_str().unwrap(),
    ]);
    assert!(!err.contains("dynamic tier"), "{err}");
    assert!(
        code == 0 || err.contains("No available targets are compatible"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn host_only_modules_are_a_build_error() {
    let dir = work("hostonly");
    let src = dir.join("app.rav");
    std::fs::write(&src, "let m = nn::MLP([2, 2])\nprint(m)\n").unwrap();
    let (code, _, err) = tulpa(&[
        "build",
        "--target",
        "rp2040",
        src.to_str().unwrap(),
        "-o",
        dir.join("a.o").to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(
        err.contains("nn::MLP is not available in firmware"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_and_new_project_include_the_vm() {
    let dir = work("runtime");
    let (code, _, err) = tulpa(&[
        "runtime",
        dir.join("rt").to_str().unwrap(),
        "--target",
        "rp2040",
    ]);
    assert_eq!(code, 0, "{err}");
    let vm = std::fs::read_to_string(dir.join("rt/tulpa_dyn.c")).unwrap();
    assert!(vm.contains("tulpa_dyn_run") && vm.contains("tlink::node"));
    let (code, _, err) = tulpa(&[
        "new",
        dir.join("bot").to_str().unwrap(),
        "--board",
        "esp32c3",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(dir.join("bot/main/tulpa/tulpa_dyn.c").exists());
    assert!(dir.join("bot/main/Kconfig.projbuild").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A firmware node (the C implementation of Tulpa Link) and a host node (the Rust one) on
/// the same UDP domain: discovery, a reliable typed topic, and a service call.
#[cfg(target_os = "linux")]
#[test]
fn firmware_link_node_talks_to_a_host_node_over_udp() {
    if !have_clang() {
        return;
    }
    let dir = work("udp");
    let domain = 150 + std::process::id() % 40;
    let board = dir.join("board.rav");
    std::fs::write(
        &board,
        format!(
            r#"import "std/link" as link
let n = link.node("board", domain: {domain}, transport: "udp")
let temp = n.advertise("/board/temp", "[K]", reliable: true)
n.serve("/board/double", |x| x * 2)
let cmds = []
n.subscribe("/host/cmd", "str", |v, m| cmds.push(v))
for i in 0..80 {{
    temp.publish((290 + i) * 1[K])
    link.spin_for(25[ms], 5[ms])
    if len(cmds) > 0 and i > 50 {{ break }}
}}
print("board got:", cmds)
n.close()
"#
        ),
    )
    .unwrap();
    let host = dir.join("host.rav");
    std::fs::write(
        &host,
        format!(
            r#"import "std/link" as link
let n = link.node("host", domain: {domain}, transport: "udp")
let temps = []
n.subscribe("/board/temp", "[K]", |v, m| temps.push(v), reliable: true)
let cmd = n.advertise("/host/cmd", "str")
link.spin_for(700[ms], 5[ms])
print("double:", n.call("/board/double", 21))
cmd.publish("go")
link.spin_for(300[ms], 5[ms])
print(len(temps) > 5, temps == sorted(temps), n.graph().nodes.map(|x| x.name).sorted())
n.close()
"#
        ),
    )
    .unwrap();
    let exe = build(&board, "linux", &dir);
    let fw = Command::new(&exe)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (code, out, err) = tulpa(&["run", host.to_str().unwrap()]);
    let fw_out = fw.wait_with_output().unwrap();
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "double: 42\ntrue true [\"board\", \"host\"]\n");
    assert_eq!(
        String::from_utf8_lossy(&fw_out.stdout),
        "board got: [\"go\"]\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A board whose only link is a UART: its frames cross a serial line (two ptys here) to
/// `tulpa link bridge`, which joins it to the UDP domain where a host node is listening.
#[cfg(target_os = "linux")]
#[test]
fn uart_link_node_reaches_the_network_through_the_bridge() {
    if !have_clang() {
        return;
    }
    fn pty() -> (i32, String) {
        let (mut m, mut s) = (0, 0);
        // SAFETY: openpty fills the two descriptors; the slave is put in raw mode and named.
        unsafe {
            assert_eq!(
                libc::openpty(
                    &mut m,
                    &mut s,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null()
                ),
                0
            );
            let mut t: libc::termios = std::mem::zeroed();
            libc::tcgetattr(s, &mut t);
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(s, libc::TCSANOW, &t);
            let name = std::ffi::CStr::from_ptr(libc::ttyname(s))
                .to_string_lossy()
                .to_string();
            (m, name)
        }
    }
    let (a, a_name) = pty();
    let (b, b_name) = pty();
    // the "cable": whatever one side writes, the other reads
    for (from, to) in [(a, b), (b, a)] {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                // SAFETY: plain read/write on descriptors that stay open for the test.
                let n = unsafe { libc::read(from, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    break;
                }
                unsafe { libc::write(to, buf.as_ptr().cast(), n as usize) };
            }
        });
    }
    let dir = work("bridge");
    let domain = 100 + std::process::id() % 40;
    let board = dir.join("board.rav");
    std::fs::write(
        &board,
        format!(
            r#"import "std/link" as link
let n = link.node("mcu", domain: {domain}, transport: "uart", port: "{a_name}")
let temp = n.advertise("/mcu/temp", "[K]", reliable: true)
n.serve("/mcu/double", |x| x * 2)
let cmds = []
n.subscribe("/base/cmd", "str", |v, m| cmds.push(v))
for i in 0..100 {{
    temp.publish((290 + i) * 1[K])
    link.spin_for(30[ms], 5[ms])
    if len(cmds) > 0 and i > 40 {{ break }}
}}
print("mcu got:", cmds)
"#
        ),
    )
    .unwrap();
    let host = dir.join("host.rav");
    std::fs::write(
        &host,
        format!(
            r#"import "std/link" as link
let n = link.node("base", domain: {domain}, transport: "udp")
let temps = []
n.subscribe("/mcu/temp", "[K]", |v, m| temps.push(v), reliable: true)
let cmd = n.advertise("/base/cmd", "str")
link.spin_for(900[ms], 5[ms])
print("double:", n.call("/mcu/double", 21))
cmd.publish("go")
link.spin_for(300[ms], 5[ms])
print(len(temps) > 5, temps == sorted(temps), n.graph().nodes.map(|x| x.name).sorted())
n.close()
"#
        ),
    )
    .unwrap();
    let exe = build(&board, "linux", &dir);
    let mut bridge = Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .args([
            "link",
            "bridge",
            &b_name,
            "--domain",
            &domain.to_string(),
            "--for",
            "8",
        ])
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let fw = Command::new(&exe)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (code, out, err) = tulpa(&["run", host.to_str().unwrap()]);
    let fw_out = fw.wait_with_output().unwrap();
    let _ = bridge.kill();
    let _ = bridge.wait();
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "double: 42\ntrue true [\"base\", \"mcu\"]\n");
    assert_eq!(
        String::from_utf8_lossy(&fw_out.stdout),
        "mcu got: [\"go\"]\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
