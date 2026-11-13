//! Tulpa 3.0: temporal monitors + virtual clock, bytecode VM, LLVM native code, language server.

use serde_json::json;
use tulpa::{parse, Interpreter};

fn run_with(src: &str, vm: bool) -> Result<String, String> {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let prog =
                parse(&src).map_err(|e| format!("parse error {}:{}: {}", e.line, e.col, e.msg))?;
            let (mut i, out) = Interpreter::captured();
            i.use_vm = vm;
            i.run(&prog).map_err(|e| e.to_string())?;
            let s = out.borrow().clone();
            Ok(s)
        })
        .unwrap()
        .join()
        .unwrap()
}

fn run(src: &str) -> String {
    run_with(src, true).unwrap_or_else(|e| panic!("{e}\n---\n{src}"))
}

// ------------------------------------------------------------------ temporal monitors

#[test]
fn debounced_when_with_virtual_clock() {
    let src = r#"
sim::virtual_time(true)
let b = GPIO::new(4, "in")
let presses = 0
when b.read() == 1 for 30[ms] { presses += 1; print("accepted at", millis()) } else { print("released at", millis()) }
fn hold(ms) { sim::gpio_input(4, 1); sim::advance(ms * 1[ms]); sim::gpio_input(4, 0); sim::advance(5[ms]) }
hold(3); hold(10); hold(40)
print(presses)
"#;
    assert_eq!(run(src), "accepted at 53\nreleased at 63\n1\n");
}

#[test]
fn deadline_monitor_satisfied_missed_and_fatal() {
    let src = r#"
sim::virtual_time(true)
let cmd = signal(0)
let pos = signal(0)
after cmd.get() == 1 within 100[ms] ensure pos.get() >= 10 else { print("timeout at", millis()) }
cmd.set(1); sim::advance(40[ms]); pos.set(10)
cmd.set(0); pos.set(0)
cmd.set(1); sim::advance(150[ms])
print("done")
"#;
    assert_eq!(run(src), "timeout at 141\ndone\n");
    let e = run_with("sim::virtual_time(true)\nlet go = signal(false)\nafter go.get() within 20[ms] ensure false\ngo.set(true)\nsleep(25[ms])", true).unwrap_err();
    assert!(
        e.contains("deadline missed: response 'false' did not hold within 20 ms"),
        "{e}"
    );
}

#[test]
fn virtual_clock_drives_sleep_and_every() {
    let src = "sim::virtual_time(true)\nlet t0 = millis()\nsleep(2[s])\nevery!(250[ms], 4, { })\nprint(millis() - t0)";
    assert_eq!(run(src), "3000\n");
    assert!(run_with("sim::advance(1[s])", true)
        .unwrap_err()
        .contains("needs the virtual clock"));
}

#[test]
fn after_is_still_usable_as_a_variable() {
    assert_eq!(run("let after = 3\nafter = after + 1\nprint(after)"), "4\n");
}

// ------------------------------------------------------------------ bytecode VM

const PROGRAMS: &[&str] = &[
    "fn fib(n) { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\nprint(fib(20))",
    "fn f(n) { let s = 0\n for i in 0..=n { if i % 3 == 0 { continue }\n if i > 50 { break }\n s += i }\n s }\nprint(f(100))",
    "fn g(xs) { let out = []\n for x in xs { out.push(x * 2) }\n out }\nprint(g([1, 2, 3]), g(\"ab\"))",
    "fn h(a, b) { a / b + a % b - (a ** 2) }\nprint(h(-7, 2), h(7.5, 2), h(10, -3))",
    "fn q(v: [m/s]) -> [m] { v * 2[s] }\nprint(q(1.5[m/s]))",
    "fn s(x) { if x > 0 and x < 10 or x == 42 { \"yes\" } else { \"no\" } }\nprint(s(5), s(42), s(-1))",
    "fn w() { let i = 0\n let acc = []\n while i < 5 { i += 1\n if i == 2 { continue }\n acc.push(f\"i={i}\") }\n acc }\nprint(w())",
    "struct P { x = 0\n fn bump(self, n) { self.x = self.x + n\n self.x } }\nfn use_p() { let p = P()\n p.bump(3)\n p.bump(4) }\nprint(use_p())",
    "let total = 0\nfn add(n) { total += n }\nadd(5); add(6)\nprint(total)",
    "fn d(x) { let r = if x { 1 } else { 2 }\n r }\nprint(d(true), d(nil))",
];

#[test]
fn vm_and_interpreter_agree() {
    for p in PROGRAMS {
        let a = run_with(p, true);
        let b = run_with(p, false);
        assert_eq!(a, b, "tiers disagree on:\n{p}");
        assert!(a.is_ok(), "{p}: {a:?}");
    }
}

#[test]
fn vm_compiles_common_shapes_and_declines_others() {
    let prog = parse("fn a(n) { for i in 0..n { } }\nfn b() { let f = |x| x }\nfn c() { match 1 { _ => 2 } }\nfn d(x) { x.y = 1\n x[0] }").unwrap();
    let compiled: Vec<bool> = prog
        .iter()
        .filter_map(|s| {
            if let tulpa::ast::Stmt::Fn(d) = s {
                Some(tulpa::vm::compile(d).is_some())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(compiled, vec![true, false, false, true]);
}

#[test]
fn vm_errors_keep_lines_traces_and_contracts() {
    let e = run_with(
        "fn inner(x) {\n  let y = 1\n  y + x.nope\n}\nfn outer() { inner(1) }\nouter()",
        true,
    )
    .unwrap_err();
    assert!(
        e.contains("line 3:") && e.contains("at inner()") && e.contains("at outer()"),
        "{e}"
    );
    assert_eq!(
        run("fn bump(x) ensure x' == x + 1 { x = x + 1\n x }\nprint(bump(41))"),
        "42\n"
    );
    let e = run_with("fn f(x: int) { x }\nf(1.5)", true).unwrap_err();
    assert!(e.contains("f() parameter 'x' expects int"), "{e}");
    let e = run_with(
        "let lim = 0\nalways lim < 3\nfn up() { lim += 5 }\nup()",
        true,
    )
    .unwrap_err();
    assert!(e.contains("safety envelope violated"), "{e}");
}

// ------------------------------------------------------------------ LLVM native code

const NATIVE: &str = r#"
const K = 3
fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
fn fdiv(a: int, b: int) -> int require b != 0 { a / b }
fn emod(a: int, b: int) -> int require b != 0 { a % b }
fn wrap(a: float) ensure -180.0 <= result' and result' < 180.0 { (a + 180.0) % 360.0 - 180.0 }
fn poly(x: float) -> float { K * x * x - 2.0 * x + 1.0 }
fn tri(n: int) -> int { let s = 0
  let i = 0
  while i <= n invariant s >= 0 { s += i
    i += 1 }
  s }
fn mix(x: int, y: float) -> float { if x > 2 and y < 10.0 { sqrt(x * y) } else { max(x, y) } }
fn label(name: str) { name }
"#;

#[test]
fn llvm_report_and_ir_shape() {
    let prog = parse(NATIVE).unwrap();
    let r = tulpa::llvm::emit_module(&prog, "t");
    let names: Vec<&str> = r.compiled.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["fib", "fdiv", "emod", "wrap", "poly", "tri", "mix"]);
    assert_eq!(r.skipped.len(), 1);
    assert!(r.skipped[0]
        .1
        .contains("type 'str' has no native representation"));
    assert!(
        r.ir.contains("define i64 @tulpa_fib(i64 %a.n)")
            && r.ir.contains("@llvm.sadd.with.overflow.i64")
    );
    assert!(r
        .ir
        .contains("precondition violated in fdiv(): require b != 0"));
    let h = r.header("T_H");
    assert!(h.contains("double tulpa_mix(int64_t x, double y);"), "{h}");
}

fn have_clang() -> bool {
    std::process::Command::new("clang")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn native_code_matches_the_interpreter() {
    if !have_clang() {
        eprintln!("clang not found; skipping native differential test");
        return;
    }
    let dir = std::env::temp_dir().join("tulpa_native_test");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("ctl.rav");
    std::fs::write(&src, NATIVE).unwrap();
    let obj = dir.join("ctl.o");
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .args(["build", src.to_str().unwrap(), "-o", obj.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(st.success());
    // inputs exercised identically in C and in Tulpa
    let ints = [-7i64, -3, -1, 0, 1, 2, 5, 13];
    let floats = [-725.5f64, -180.0, -0.5, 0.0, 3.25, 179.9, 540.0];
    let mut c = String::from("#include <stdio.h>\n#include \"ctl.h\"\nint main(void){\n");
    let mut t = String::from(NATIVE);
    for a in ints {
        for b in ints.iter().filter(|b| **b != 0) {
            c += &format!("printf(\"%lld %lld\\n\",(long long)tulpa_fdiv({a},{b}),(long long)tulpa_emod({a},{b}));\n");
            t += &format!("print(fdiv({a}, {b}), emod({a}, {b}))\n");
        }
        if a >= 0 {
            c += &format!(
                "printf(\"%lld %lld\\n\",(long long)tulpa_fib({a}),(long long)tulpa_tri({a}));\n"
            );
            t += &format!("print(fib({a}), tri({a}))\n");
        }
    }
    for x in floats {
        c += &format!("printf(\"%.6f %.6f\\n\",tulpa_wrap({x:?}),tulpa_poly({x:?}));\n");
        t += &format!("print(format(wrap({x:?}), \".6f\"), format(poly({x:?}), \".6f\"))\n");
    }
    for (x, y) in [(3i64, 4.0f64), (1, 2.5), (5, 20.0)] {
        c += &format!("printf(\"%.6f\\n\",tulpa_mix({x},{y:?}));\n");
        t += &format!("print(format(mix({x}, {y:?}), \".6f\"))\n");
    }
    c += "return 0;}\n";
    let main_c = dir.join("main.c");
    std::fs::write(&main_c, c).unwrap();
    let exe = dir.join("app");
    let st = std::process::Command::new("clang")
        .args([
            "-O2",
            main_c.to_str().unwrap(),
            obj.to_str().unwrap(),
            "-lm",
            "-I",
            dir.to_str().unwrap(),
            "-o",
            exe.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(st.success());
    let native =
        String::from_utf8(std::process::Command::new(&exe).output().unwrap().stdout).unwrap();
    let interp = run(&t);
    assert_eq!(native, interp);

    // contract violations abort with the clause on stderr
    let bad_c = dir.join("bad.c");
    std::fs::write(
        &bad_c,
        "#include \"ctl.h\"\nint main(void){ return (int)tulpa_fdiv(1, 0); }\n",
    )
    .unwrap();
    let bad = dir.join("bad");
    assert!(std::process::Command::new("clang")
        .args([
            bad_c.to_str().unwrap(),
            obj.to_str().unwrap(),
            "-lm",
            "-I",
            dir.to_str().unwrap(),
            "-o",
            bad.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    let out = std::process::Command::new(&bad).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("precondition violated in fdiv(): require b != 0"));
}

// ------------------------------------------------------------------ language server

fn lsp(msgs: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut st = tulpa::lsp::State::default();
    msgs.iter()
        .flat_map(|m| tulpa::lsp::handle(&mut st, m))
        .collect()
}

const DOC: &str = "const MAX = 2[m/s]\nfn drive(v: [m/s]) -> [m] require v > 0 { v * 1[s] }\nmachine Door { state Closed { on open => Open } state Open { on close => Closed } }\nlet d = Door()\nprint(drve(MAX), 2[m] + 3[s])\nlet x = 5[";

fn open(text: &str) -> serde_json::Value {
    json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": "file:///a.rav", "text": text}}})
}

fn at(method: &str, line: u64, ch: u64) -> serde_json::Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": {"textDocument": {"uri": "file:///a.rav"}, "position": {"line": line, "character": ch}}})
}

#[test]
fn lsp_initialize_and_diagnostics() {
    let r = lsp(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})]);
    assert_eq!(r[0]["result"]["capabilities"]["hoverProvider"], true);
    let doc = DOC.trim_end_matches("\nlet x = 5[");
    let r = lsp(&[open(doc)]);
    let diags = r[0]["params"]["diagnostics"].as_array().unwrap();
    let msgs: Vec<&str> = diags
        .iter()
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert!(
        msgs.contains(&"'drve' is never defined (did you mean 'drive'?)"),
        "{msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("unit mismatch: cannot apply '+' to length [m] and time [s]")),
        "{msgs:?}"
    );
    assert_eq!(msgs.len(), 2, "{msgs:?}");
    // syntax errors are reported with their position
    let r = lsp(&[open("let x = (1 +\n")]);
    assert_eq!(r[0]["params"]["diagnostics"][0]["severity"], 1);
}

#[test]
fn lsp_hover_completion_symbols_definition() {
    let doc = DOC.trim_end_matches("\nlet x = 5[");
    let r = lsp(&[open(doc), at("textDocument/hover", 4, 12)]); // on MAX
    let hover = r[1]["result"]["contents"]["value"].as_str().unwrap();
    assert!(
        hover.contains("const MAX = 2[m/s]") && hover.contains("speed"),
        "{hover}"
    );
    let r = lsp(&[open(doc), at("textDocument/hover", 1, 4)]); // on drive
    let hover = r[1]["result"]["contents"]["value"].as_str().unwrap();
    assert!(
        hover.contains("fn drive(v: [m/s]) -> [m]") && hover.contains("require v > 0"),
        "{hover}"
    );
    let r = lsp(&[open(doc), at("textDocument/hover", 2, 9)]); // on Door
    assert!(r[1]["result"]["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("states: Closed → Open"));

    let r = lsp(&[open(DOC), at("textDocument/completion", 5, 10)]);
    let labels: Vec<&str> = r[1]["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"m") && labels.contains(&"rpm") && !labels.contains(&"let"),
        "{labels:?}"
    );
    let r = lsp(&[
        open("let s = \"a\"\ns.up"),
        at("textDocument/completion", 1, 4),
    ]);
    let labels: Vec<&str> = r[1]["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"upper") && labels.contains(&"send"),
        "{labels:?}"
    );

    let r = lsp(&[
        open(doc),
        json!({"jsonrpc": "2.0", "id": 3, "method": "textDocument/documentSymbol", "params": {"textDocument": {"uri": "file:///a.rav"}}}),
    ]);
    let names: Vec<&str> = r[1]["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["MAX", "drive", "Door", "Closed", "Open"]);

    let r = lsp(&[open(doc), at("textDocument/definition", 3, 9)]); // Door() → machine Door
    assert_eq!(
        r[1]["result"]["range"]["start"],
        json!({"line": 2, "character": 8})
    );
}

#[test]
fn lsp_over_stdio() {
    let frame = |m: serde_json::Value| {
        let b = m.to_string();
        format!("Content-Length: {}\r\n\r\n{b}", b.len())
    };
    let input = [
        frame(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})),
        frame(open("print(undefined_thing)")),
        frame(json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"})),
        frame(json!({"jsonrpc": "2.0", "method": "exit"})),
    ]
    .concat();
    use std::io::Write;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_tulpa"))
        .arg("lsp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("tulpa-lsp") && text.contains("'undefined_thing' is never defined"),
        "{text}"
    );
}
