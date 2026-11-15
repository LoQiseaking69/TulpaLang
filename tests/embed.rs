//! Embedding API (used by the Android IDE).
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tulpa::embed::{self, Feed, RunConfig};

fn cfg(out: Rc<RefCell<String>>, cancel: Arc<AtomicBool>) -> RunConfig {
    RunConfig {
        base_dir: ".".into(),
        args: vec!["a1".into()],
        virtual_time: false,
        output: Rc::new(move |s: &str| out.borrow_mut().push_str(s)),
        input: None,
        cancel,
    }
}

fn on_big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(embed::RECOMMENDED_STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn run_streams_output_and_args() {
    on_big_stack(|| {
        let out = Rc::new(RefCell::new(String::new()));
        embed::run(
            "print(\"hi\", ARGS)\nprint(2 * 3[m])",
            "t.rav",
            cfg(out.clone(), Arc::default()),
        )
        .unwrap();
        assert_eq!(out.borrow().as_str(), "hi [\"a1\"]\n6 m\n");
    });
}

#[test]
fn cancel_stops_loops_and_sleep() {
    for src in [
        "let n = 0\nloop { n += 1 }",
        "while true { }",
        "sleep(60)",
        "every!(10, { 1 })",
        "fn spin() { let i = 0\n while i >= 0 { i += 1 } }\nspin()",
    ] {
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let t = std::time::Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            c2.store(true, Ordering::Relaxed);
        });
        let src = src.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        on_big_stack(move || {
            let out = Rc::new(RefCell::new(String::new()));
            tx.send(embed::run(&src, "t.rav", cfg(out, cancel)))
                .unwrap();
        });
        let r = rx.recv().unwrap();
        assert!(r.as_ref().unwrap_err().contains("interrupted"), "{r:?}");
        assert!(t.elapsed().as_secs_f64() < 5.0);
    }
}

#[test]
fn input_hook_and_parse_errors() {
    on_big_stack(|| {
        let out = Rc::new(RefCell::new(String::new()));
        let mut c = cfg(out.clone(), Arc::default());
        c.input = Some(Rc::new(|prompt: &str| Some(format!("{prompt}42\n"))));
        embed::run("let x = input(\"n=\")\nprint(x)", "t.rav", c).unwrap();
        assert_eq!(out.borrow().trim(), "n=42");
        let e = embed::run("let = 3", "bad.rav", cfg(out, Arc::default())).unwrap_err();
        assert!(e.starts_with("bad.rav:1:"), "{e}");
    });
}

#[test]
fn host_modules() {
    tulpa::host::register("rig", "volts", |_, a| {
        Ok(tulpa::Value::Float(12.0 + a.len() as f64))
    });
    on_big_stack(|| {
        let out = Rc::new(RefCell::new(String::new()));
        embed::run(
            "print(rig::volts(), rig::volts(1))",
            "t.rav",
            cfg(out.clone(), Arc::default()),
        )
        .unwrap();
        assert_eq!(out.borrow().trim(), "12.0 13.0");
    });
    assert!(tulpa::host::modules().contains(&"rig"));
    // the checker knows host modules: no "undefined name" diagnostic for rig::volts
    assert!(!embed::diagnostics("print(rig::volts())").contains("rig"));
    assert!(embed::modules().contains("\"rig\":[\"volts\"]"));
}

#[test]
fn session_keeps_state() {
    on_big_stack(|| {
        let out = Rc::new(RefCell::new(String::new()));
        let o2 = out.clone();
        let mut s = embed::Session::new(
            Rc::new(move |x: &str| o2.borrow_mut().push_str(x)),
            None,
            Arc::default(),
        );
        assert_eq!(s.feed("let v = 2.5[m/s]"), Feed::Value(None));
        assert_eq!(s.feed("fn sq(x) {"), Feed::More);
        assert!(s.pending());
        assert_eq!(s.feed("  x * x\n}"), Feed::Value(None));
        assert_eq!(s.feed("sq(12)"), Feed::Value(Some("144".into())));
        assert_eq!(s.feed("v.to(\"km/h\")"), Feed::Value(Some("9 km/h".into())));
        assert!(matches!(s.feed("1/0"), Feed::Error(_)));
        s.feed("print(\"side effect\")");
        assert_eq!(out.borrow().trim(), "side effect");
        assert!(s.names().contains(&"sq".to_string()));
    });
}

#[test]
fn analysis_functions() {
    let src = "machine Door {\n  state Closed { on open if ok => Open }\n  state Open { on close => Closed }\n  on fault => Closed\n}\nfn f(x: int) -> int\n  ensure result' >= 0\n{ x }\nprint(undefined_thing)\n";
    let d = embed::diagnostics(src);
    assert!(d.contains("undefined_thing"), "{d}");
    let m: serde_json::Value = serde_json::from_str(&embed::machines(src)).unwrap();
    assert_eq!(m[0]["name"], "Door");
    assert_eq!(m[0]["initial"], "Closed");
    assert_eq!(m[0]["transitions"][0]["guard"], "ok");
    assert_eq!(m[0]["transitions"][2]["global"], true);
    assert!(embed::symbols(src).contains("Door"));
    assert!(embed::completion("fn speedy() { 1 }\nsp", 1, 2).contains("speedy"));
    assert!(embed::completion("let q = 5[", 0, 10).contains("\"m\""));
    assert_eq!(
        embed::definition("fn speedy() { 1 }\nspeedy()", 1, 2),
        r#"{"col":3,"len":6,"line":0}"#
    );
    assert!(embed::hover("let d = 3[m]\nd", 0, 10)
        .unwrap_or_default()
        .contains("unit"));
    assert!(embed::bytecode("fn add(a, b) { a + b }")
        .unwrap()
        .contains("add"));
    let v: serde_json::Value =
        serde_json::from_str(&embed::verify(src, ".".into(), 200, Arc::default()).unwrap())
            .unwrap();
    let f = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "f")
        .unwrap();
    assert_eq!(
        f["ok"], false,
        "f(x)=x violates result' >= 0 for negative x: {f}"
    );
}

#[test]
fn tests_runner() {
    on_big_stack(|| {
        let out = Rc::new(RefCell::new(String::new()));
        let src = "define_test!(\"adds\", { assert 1 + 1 == 2 })\ndefine_test!(\"bad\", { assert 1 == 2 })";
        let j: serde_json::Value =
            serde_json::from_str(&embed::tests(src, "t.rav", cfg(out, Arc::default())).unwrap())
                .unwrap();
        assert_eq!(j["passed"], 1);
        assert_eq!(j["failed"][0]["name"], "bad");
    });
}
