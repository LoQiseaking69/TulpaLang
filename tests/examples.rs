//! Runs every example program and compares its output with `expected.txt`.

use std::path::Path;

fn normalize(s: &str) -> String {
    // log_message! timestamps vary between runs.
    s.lines()
        .map(|l| match (l.find('['), l.find(" UTC]")) {
            (Some(0), Some(end)) => format!("[TIME]{}", &l[end + 5..]),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn examples_match_expected_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut checked = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let dir = entry.unwrap().path();
        let main = dir.join("main.rav");
        let expected = dir.join("expected.txt");
        if !main.exists() || !expected.exists() {
            continue;
        }
        let src = std::fs::read_to_string(&main).unwrap();
        let out = tulpa::run_captured_in(&src, &dir)
            .unwrap_or_else(|e| panic!("{}: {e}", main.display()));
        let want = std::fs::read_to_string(&expected).unwrap();
        assert_eq!(
            normalize(&out),
            normalize(&want),
            "output mismatch for {}",
            main.display()
        );
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} examples were checked");
}

#[test]
fn tulpa_test_runner() {
    let exe = env!("CARGO_BIN_EXE_tulpa");
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tulpa/std_test.rav");
    let out = std::process::Command::new(exe)
        .arg("test")
        .arg(&file)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("4 passed, 0 failed"), "{stdout}");
}

#[test]
fn cli_reports_syntax_errors_with_caret() {
    let exe = env!("CARGO_BIN_EXE_tulpa");
    let dir = std::env::temp_dir().join("tulpa_cli_test");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("bad.rav");
    std::fs::write(&f, "let x = 1\nlet y = (x +\n").unwrap();
    let out = std::process::Command::new(exe)
        .arg("check")
        .arg(&f)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(65));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("syntax error") && err.contains('^'), "{err}");
}

fn tulpa(args: &[&str]) -> (i32, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn record_then_replay_detects_divergence() {
    let dir = std::env::temp_dir().join("tulpa_tape_test");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("bot.rav");
    let tape = dir.join("run.tape");
    let body = |threshold: u32| {
        format!(
            "if len(ARGS) > 0 {{ sim::i2c_set(0x48, 0, [0x19, 0x10]) }}\nlet raw = I2C::new(0x48).read_u16(0)\nprint(raw)\nGPIO::new(17).write(raw > {threshold})\n"
        )
    };
    std::fs::write(&script, body(0x1000)).unwrap();
    let (s, out, err) = tulpa(&[
        "--record",
        tape.to_str().unwrap(),
        script.to_str().unwrap(),
        "seed",
    ]);
    assert_eq!(s, 0, "{err}");
    assert_eq!(out, "6416\n");
    assert!(err.contains("recorded 3 device events"), "{err}");

    // Replay reproduces the sensor value without seeding the simulator.
    let (s, out, err) = tulpa(&["--replay", tape.to_str().unwrap(), script.to_str().unwrap()]);
    assert_eq!((s, out.as_str()), (0, "6416\n"), "{err}");
    assert!(err.contains("replay matched all 3 device events"), "{err}");

    // Changing the control logic is caught at the first differing device write.
    std::fs::write(&script, body(0x5000)).unwrap();
    let (s, _, err) = tulpa(&["--replay", tape.to_str().unwrap(), script.to_str().unwrap()]);
    assert_eq!(s, 70);
    assert!(
        err.contains(
            "replay diverged at event #2: tape has gpio17 write [01], script did gpio17 write [00]"
        ),
        "{err}"
    );
}

#[test]
fn verify_and_graph_commands() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let verify = root.join("examples/verify/main.rav");
    let (s, out, _) = tulpa(&["verify", "--trials=200", verify.to_str().unwrap()]);
    assert_eq!(s, 1);
    assert!(
        out.contains("FAIL  saturate(-101)") && out.contains("ok    mid()"),
        "{out}"
    );
    assert!(
        out.contains("verified 4 function(s): 3 passed, 1 failed"),
        "{out}"
    );

    let robot = root.join("examples/warehouse_robot/main.rav");
    let (s, out, err) = tulpa(&["graph", robot.to_str().unwrap()]);
    assert_eq!(s, 0, "{err}");
    assert!(
        out.contains("stateDiagram-v2")
            && out.contains("Picking --> Placing : gripped(mass) [guarded]"),
        "{out}"
    );
}
