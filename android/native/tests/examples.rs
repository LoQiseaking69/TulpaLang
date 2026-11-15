//! Every bundled example runs on the embedded runtime (phone examples with synthetic sensors).
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tulpa::embed::{self, RunConfig};
use tulpa_android::phone::{self, kind};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn run_dir(dir: &Path, virtual_time: bool, stop_after_ms: u64) -> (Result<(), String>, String) {
    let src = std::fs::read_to_string(dir.join("main.rav")).unwrap();
    let dir = dir.to_path_buf();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(stop_after_ms));
        c2.store(true, Ordering::Relaxed);
    });
    std::thread::Builder::new()
        .stack_size(embed::RECOMMENDED_STACK)
        .spawn(move || {
            let out = Rc::new(RefCell::new(String::new()));
            let o = out.clone();
            let cfg = RunConfig {
                base_dir: dir,
                args: vec![],
                virtual_time,
                output: Rc::new(move |s: &str| o.borrow_mut().push_str(s)),
                input: Some(Rc::new(|_: &str| Some("7".into()))),
                cancel,
            };
            let r = embed::run(&src, "main.rav", cfg);
            let s = out.borrow().clone();
            (r, s)
        })
        .unwrap()
        .join()
        .unwrap()
}

fn setup() -> Arc<Mutex<Vec<String>>> {
    std::env::set_var("TULPA_PATH", repo().join("stdlib"));
    std::env::set_var("TULPA_TELEMETRY", "1");
    let tmp = std::env::temp_dir().join("tulpa_field_examples");
    std::fs::create_dir_all(&tmp).unwrap();
    std::env::set_current_dir(&tmp).unwrap(); // files programs write land here
    phone::register();
    phone::push(kind::ACCEL, &[0.3, 0.2, 9.79]);
    phone::push(kind::GRAVITY, &[0.3, 0.2, 9.79]);
    phone::push(kind::LINEAR, &[0.1, 0.0, 0.05]);
    phone::push(kind::GYRO, &[0.01, 0.0, 0.02]);
    phone::push(kind::MAG, &[20.0, -5.0, 40.0]);
    phone::push(kind::ORIENTATION, &[1.2, 0.05, 0.02]);
    phone::push(kind::LIGHT, &[320.0]);
    phone::push(kind::PRESSURE, &[1009.8]);
    phone::push(kind::PROXIMITY, &[5.0]);
    phone::push(kind::BATTERY, &[0.76, 0.0]);
    phone::push_location([3.1390, 101.6869, 55.0, 8.0, 1.2, 90.0]);
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let s2 = seen.clone();
    phone::set_action_handler(Box::new(move |k, _| {
        s2.lock().unwrap().push(k.to_string());
        String::new()
    }));
    seen
}

#[test]
fn all_examples() {
    let actions = setup();
    let mut failures = Vec::new();
    // phone + library examples bundled with the app
    let app = repo().join("android/app/src/main/tulpa/examples");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&app)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    // the language's own examples
    dirs.extend(
        std::fs::read_dir(repo().join("examples"))
            .unwrap()
            .map(|e| e.unwrap().path()),
    );
    dirs.sort();
    for d in &dirs {
        let name = d.file_name().unwrap().to_string_lossy().to_string();
        let vt = name == "timing";
        let (r, out) = run_dir(d, vt, 1500);
        let ok = match &r {
            Ok(()) => true,
            Err(e) => e.contains("interrupted"),
        };
        println!(
            "{:<16} {:<12} {} lines",
            name,
            if r.is_ok() {
                "completed"
            } else if ok {
                "stopped"
            } else {
                "FAILED"
            },
            out.lines().count()
        );
        if !ok {
            failures.push(format!("{name}: {}", r.clone().unwrap_err()));
        }
        // the language examples have exact expected output
        let exp = d.join("expected.txt");
        if r.is_ok() && exp.exists() && name != "native" {
            let mask = |t: &str| -> String {
                // wall-clock timestamps from log_message! differ run to run
                t.lines()
                    .map(|l| {
                        if l.starts_with('[') && l.contains(" UTC]") {
                            l.split_once(" UTC]").map(|x| x.1).unwrap_or(l).to_string()
                        } else {
                            l.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let want = std::fs::read_to_string(exp).unwrap();
            if mask(want.trim()) != mask(out.trim()) {
                failures.push(format!("{name}: output differs from expected.txt"));
            }
        }
    }
    let acts = actions.lock().unwrap().clone();
    assert!(
        acts.iter().any(|a| a == "gauge"),
        "phone examples should drive gauges: {acts:?}"
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
