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
        let out = tulpa::run_captured(&src).unwrap_or_else(|e| panic!("{}: {e}", main.display()));
        let want = std::fs::read_to_string(&expected).unwrap();
        assert_eq!(
            normalize(&out),
            normalize(&want),
            "output mismatch for {}",
            main.display()
        );
        checked += 1;
    }
    assert!(checked >= 6, "only {checked} examples were checked");
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
