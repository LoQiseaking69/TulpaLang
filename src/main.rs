//! `tulpa` command-line interface.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use tulpa::{parse, Backend, Interpreter, ParseError};

const USAGE: &str = "\
Tulpa 3.0 — device-integration scripting language

USAGE:
    tulpa [FLAGS] <file.tlp|file.rav> [args...]   run a program
    tulpa run [FLAGS] <file>                      same as above
    tulpa test [FLAGS] <file>...                  run define_test! blocks
    tulpa check <file>...                         parse only, report syntax errors
    tulpa repl [FLAGS]                            interactive prompt (default with no file)
    tulpa ast <file>                              dump the parsed syntax tree
    tulpa verify [--trials=N] <file>...           fuzz every fn against its own require/ensure
    tulpa graph <file>...                         print Mermaid diagrams of every state machine
    tulpa emit-llvm <file> [-o out.ll]            LLVM IR for the typed numeric subset
    tulpa build <file> [-o out.o] [--shared]      native object/library + C header (needs clang)
    tulpa bytecode <file>                         show VM bytecode per function
    tulpa lsp                                     language server over stdio

FLAGS:
    --hw        use real Linux hardware (sysfs GPIO, /dev/i2c-*, /dev/spidev*, TTYs)
    --sim       use the in-memory simulator (default)
    --trace     log every device call to stderr
    --no-vm     run everything on the tree-walking interpreter (no bytecode tier)
    --virtual-time  simulated clock: sleeps and every! ticks advance instantly
    --record F  record all device traffic to tape file F
    --replay F  answer device calls from tape F; fail on the first divergence
    -h, --help  show this help
    -V          print version
";

struct Opts {
    backend: Backend,
    trace: bool,
    record: Option<String>,
    replay: Option<String>,
    virtual_time: bool,
    no_vm: bool,
}

fn report_parse(file: &str, src: &str, e: &ParseError) {
    eprintln!("{file}:{}:{}: syntax error: {}", e.line, e.col, e.msg);
    if let Some(line) = src.lines().nth(e.line.saturating_sub(1)) {
        eprintln!("    {line}");
        eprintln!("    {}^", " ".repeat(e.col.saturating_sub(1)));
    }
}

fn load(file: &str) -> Result<(String, Vec<tulpa::ast::Stmt>), ExitCode> {
    let src = std::fs::read_to_string(file).map_err(|e| {
        eprintln!("tulpa: cannot read {file}: {e}");
        ExitCode::from(2)
    })?;
    match parse(&src) {
        Ok(p) => Ok((src, p)),
        Err(e) => {
            report_parse(file, &src, &e);
            Err(ExitCode::from(65))
        }
    }
}

fn interpreter(o: &Opts, args: &[String]) -> Interpreter {
    let mut i = Interpreter::new();
    i.backend = o.backend;
    i.trace_devices = o.trace;
    if o.virtual_time {
        i.virtual_time = Some(0.0);
    }
    if o.no_vm {
        i.use_vm = false;
    }
    if let Some(p) = &o.replay {
        match tulpa::devices::Tape::load(std::path::Path::new(p)) {
            Ok(t) => i.tape = Some(std::rc::Rc::new(std::cell::RefCell::new(t))),
            Err(e) => {
                eprintln!("tulpa: cannot load replay tape: {e}");
                std::process::exit(2);
            }
        }
    } else if let Some(p) = &o.record {
        i.tape = Some(std::rc::Rc::new(std::cell::RefCell::new(
            tulpa::devices::Tape::record(std::path::Path::new(p)),
        )));
    }
    let argv = tulpa::Value::list(args.iter().map(|a| tulpa::Value::str(a.as_str())).collect());
    tulpa::interp::define(&i.globals, "ARGS", argv);
    i
}

fn run_file(file: &str, args: &[String], o: &Opts) -> ExitCode {
    let (_, prog) = match load(file) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let mut i = interpreter(o, args);
    set_base(&mut i, file);
    let result = i.run(&prog);
    let mut code = match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{file}: runtime error: {e}");
            ExitCode::from(70)
        }
    };
    if let Some(t) = &i.tape {
        let t = t.borrow();
        match t.mode {
            tulpa::devices::TapeMode::Record => match t.save() {
                Ok(()) => eprintln!(
                    "tulpa: recorded {} device events to {}",
                    t.events.len(),
                    t.path.display()
                ),
                Err(e) => eprintln!("tulpa: failed to save tape: {e}"),
            },
            tulpa::devices::TapeMode::Replay => {
                if t.remaining() > 0 && code == ExitCode::SUCCESS {
                    eprintln!(
                        "{file}: replay diverged: script finished with {} recorded device event(s) unused (next: {} {})",
                        t.remaining(),
                        t.events[t.pos].dev,
                        t.events[t.pos].op
                    );
                    code = ExitCode::from(70);
                } else if code == ExitCode::SUCCESS {
                    eprintln!("tulpa: replay matched all {} device events", t.events.len());
                }
            }
        }
    }
    code
}

fn set_base(i: &mut Interpreter, file: &str) {
    let dir = std::path::Path::new(file)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    i.base_dirs = vec![if dir.as_os_str().is_empty() {
        ".".into()
    } else {
        dir
    }];
}

/// Runs only the definitions of a program (fn, struct, machine, let/const, import, macros),
/// skipping top-level actions — used by `verify` and `graph`.
fn load_definitions(file: &str, o: &Opts) -> Result<Interpreter, ExitCode> {
    use tulpa::ast::Stmt;
    let (_, prog) = load(file)?;
    let defs: Vec<Stmt> = prog
        .into_iter()
        .filter(|s| {
            matches!(
                s,
                Stmt::Fn(_)
                    | Stmt::Struct(_)
                    | Stmt::Machine(_)
                    | Stmt::Let(..)
                    | Stmt::Import(..)
                    | Stmt::MacroDef(..)
                    | Stmt::Use(_)
            )
        })
        .collect();
    let mut i = interpreter(o, &[]);
    set_base(&mut i, file);
    let (_, buf) = Interpreter::captured();
    i.output = tulpa::interp::Output::Buffer(buf);
    if let Err(e) = i.run(&defs) {
        eprintln!("{file}: error while loading definitions: {e}");
        return Err(ExitCode::from(70));
    }
    i.output = tulpa::interp::Output::Stdout;
    Ok(i)
}

fn verify_files(files: &[String], o: &Opts, trials: usize) -> ExitCode {
    let mut failed = 0;
    let mut checked = 0;
    for file in files {
        let mut i = match load_definitions(file, o) {
            Ok(i) => i,
            Err(c) => return c,
        };
        for (name, v) in i.global_functions() {
            let tulpa::Value::Func(c) = &v else { continue };
            if c.def.ensures.is_empty() && c.def.ret.is_none() {
                continue;
            }
            checked += 1;
            let r = match i.check_fn(&v, trials) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let get = |k: &str| match &r {
                tulpa::Value::Dict(d) => d
                    .borrow()
                    .get(&tulpa::Value::str(k))
                    .ok()
                    .flatten()
                    .unwrap_or(tulpa::Value::Nil),
                _ => tulpa::Value::Nil,
            };
            if get("ok").truthy() {
                println!(
                    "  ok    {name}()  {} cases, {} discarded",
                    get("passed"),
                    get("discarded")
                );
            } else {
                failed += 1;
                let args = match get("counterexample") {
                    tulpa::Value::List(l) => l
                        .borrow()
                        .iter()
                        .map(|v| v.repr())
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => String::new(),
                };
                println!("  FAIL  {name}({args})\n        {}", get("error"));
            }
        }
    }
    println!(
        "\nverified {checked} function(s): {} passed, {failed} failed",
        checked - failed
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `tulpa emit-llvm file.rav [-o out.ll]` / `tulpa build file.rav [-o out.o] [--shared]`
fn native(rest: &[String], build: bool) -> ExitCode {
    let mut file = None;
    let mut out: Option<String> = None;
    let mut shared = false;
    let mut opt = "-O2".to_string();
    let mut it = rest[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = it.next().cloned(),
            "--shared" => shared = true,
            "-O0" | "-O1" | "-O2" | "-O3" | "-Os" => opt = a.clone(),
            f => file = Some(f.to_string()),
        }
    }
    let Some(file) = file else {
        eprintln!(
            "usage: tulpa {} <file.rav> [-o out]",
            if build { "build" } else { "emit-llvm" }
        );
        return ExitCode::from(2);
    };
    let (_, prog) = match load(&file) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let stem = std::path::Path::new(&file)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "tulpa".into());
    let report = tulpa::llvm::emit_module(&prog, &stem);
    for (name, sig) in &report.compiled {
        let ps: Vec<String> = sig
            .params
            .iter()
            .map(|(n, t)| format!("{n}: {}", t.c()))
            .collect();
        eprintln!("  native  {name}({}) -> {}", ps.join(", "), sig.ret.c());
    }
    for (name, why) in &report.skipped {
        eprintln!("  skipped {name}: {why}");
    }
    if report.compiled.is_empty() {
        eprintln!("tulpa: no functions in {file} fit the native subset");
        return ExitCode::from(1);
    }
    if !build {
        match out {
            Some(p) if p != "-" => {
                if let Err(e) = std::fs::write(&p, &report.ir) {
                    eprintln!("tulpa: {p}: {e}");
                    return ExitCode::from(2);
                }
                eprintln!("wrote {p}");
            }
            _ => print!("{}", report.ir),
        }
        return ExitCode::SUCCESS;
    }
    let out = out.unwrap_or_else(|| format!("{stem}{}", if shared { ".so" } else { ".o" }));
    let base = out
        .trim_end_matches(".so")
        .trim_end_matches(".o")
        .to_string();
    let ll = format!("{base}.ll");
    let header = format!("{base}.h");
    let guard = format!(
        "TULPA_{}_H",
        stem.to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    if let Err(e) =
        std::fs::write(&ll, &report.ir).and_then(|_| std::fs::write(&header, report.header(&guard)))
    {
        eprintln!("tulpa: {e}");
        return ExitCode::from(2);
    }
    let cc = std::env::var("TULPA_CLANG").unwrap_or_else(|_| "clang".into());
    let mut cmd = std::process::Command::new(&cc);
    cmd.args([opt.as_str(), "-Wno-override-module"]);
    if shared {
        cmd.args(["-shared", "-fPIC"]);
    } else {
        cmd.arg("-c");
    }
    cmd.args([&ll, "-o", &out]);
    if shared {
        cmd.arg("-lm");
    }
    match cmd.status() {
        Ok(s) if s.success() => {
            eprintln!("built {out} (+ {header}, {ll})");
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("tulpa: {cc} failed compiling {ll}");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("tulpa: cannot run {cc} ({e}); the IR is in {ll} and the header in {header}");
            ExitCode::from(1)
        }
    }
}

/// `tulpa bytecode file.rav`: shows which functions run on the VM and their bytecode.
fn bytecode(files: &[String]) -> ExitCode {
    use tulpa::ast::Stmt;
    for file in files {
        let (_, prog) = match load(file) {
            Ok(x) => x,
            Err(c) => return c,
        };
        for s in &prog {
            if let Stmt::Fn(def) = s {
                match tulpa::vm::compile(def) {
                    Some(c) => print!("{}", tulpa::vm::disassemble(&def.name, &c)),
                    None => println!(
                        "fn {}: interpreted (uses features outside the bytecode tier)",
                        def.name
                    ),
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn graph_files(files: &[String], o: &Opts) -> ExitCode {
    for file in files {
        let i = match load_definitions(file, o) {
            Ok(i) => i,
            Err(c) => return c,
        };
        for (name, v) in i.global_values() {
            if let tulpa::Value::MachineType(t) = v {
                println!(
                    "%% machine {name} ({file})\n{}",
                    tulpa::ext::mermaid(&t.def)
                );
            }
        }
    }
    ExitCode::SUCCESS
}

fn test_files(files: &[String], o: &Opts) -> ExitCode {
    let (mut passed, mut failed) = (0, 0);
    for file in files {
        let (_, prog) = match load(file) {
            Ok(x) => x,
            Err(c) => return c,
        };
        let mut i = interpreter(o, &[]);
        set_base(&mut i, file);
        let (src, buf) = Interpreter::captured();
        drop(src);
        i.output = tulpa::interp::Output::Buffer(buf);
        if let Err(e) = i.run(&prog) {
            eprintln!("{file}: error while loading tests: {e}");
            return ExitCode::from(70);
        }
        let (p, f) = i.run_tests();
        passed += p;
        failed += f.len();
        println!("{file}: {p} passed, {} failed", f.len());
        for (name, err) in f {
            println!("  FAIL {name}: {err}");
        }
    }
    println!("\ntotal: {passed} passed, {failed} failed");
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn balanced(src: &str) -> bool {
    // Count brackets outside strings and comments.
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = in_str {
            if c == '\\' {
                chars.next();
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' => in_str = Some(c),
            '#' => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth <= 0 && in_str.is_none()
}

fn repl(o: &Opts) -> ExitCode {
    println!(
        "Tulpa 3.0 REPL — backend: {:?}. Type :quit to exit, :help for help.",
        o.backend
    );
    let mut i = interpreter(o, &[]);
    let env = i.globals.clone();
    let stdin = std::io::stdin();
    let mut buf = String::new();
    loop {
        print!("{}", if buf.is_empty() { "tulpa> " } else { "  ...> " });
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            println!();
            break;
        }
        match line.trim() {
            ":quit" | ":q" | "exit" if buf.is_empty() => break,
            ":help" if buf.is_empty() => {
                println!("Enter statements or expressions. Multi-line blocks continue until braces balance.");
                println!(":log  show simulator transaction log    :reset  reset simulator    :quit  exit");
                continue;
            }
            ":log" if buf.is_empty() => {
                for l in &i.sim.borrow().log {
                    println!("  {l}");
                }
                continue;
            }
            ":reset" if buf.is_empty() => {
                *i.sim.borrow_mut() = tulpa::devices::SimBus::default();
                continue;
            }
            _ => {}
        }
        buf.push_str(&line);
        if !balanced(&buf) {
            continue;
        }
        let src = std::mem::take(&mut buf);
        if src.trim().is_empty() {
            continue;
        }
        match parse(&src) {
            Err(e) => report_parse("<repl>", &src, &e),
            Ok(prog) => {
                let is_expr = matches!(prog.last(), Some(tulpa::ast::Stmt::Expr(..)));
                match i.run_in(&prog, &env) {
                    Ok(v) if is_expr && !matches!(v, tulpa::Value::Nil) => println!("{}", v.repr()),
                    Ok(_) => {}
                    Err(e) => eprintln!("error: {e}"),
                }
                if let Err(e) = i.drain_tasks() {
                    eprintln!("error: {e}");
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn real_main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut o = Opts {
        backend: Backend::Sim,
        trace: false,
        record: None,
        replay: None,
        virtual_time: false,
        no_vm: false,
    };
    if std::env::var("TULPA_BACKEND")
        .map(|v| v == "linux" || v == "hw")
        .unwrap_or(false)
    {
        o.backend = Backend::Linux;
    }
    // Flags may appear anywhere before the script path.
    let mut rest = Vec::new();
    let mut seen_file = false;
    let mut want_value: Option<&'static str> = None;
    for a in args.drain(..) {
        if seen_file {
            rest.push(a);
            continue;
        }
        if let Some(flag) = want_value.take() {
            if flag == "--record" {
                o.record = Some(a)
            } else {
                o.replay = Some(a)
            }
            continue;
        }
        if let Some(v) = a.strip_prefix("--record=") {
            o.record = Some(v.to_string());
            continue;
        }
        if let Some(v) = a.strip_prefix("--replay=") {
            o.replay = Some(v.to_string());
            continue;
        }
        if a.starts_with("--trials=") {
            rest.push(a);
            continue;
        }
        match a.as_str() {
            "--record" => want_value = Some("--record"),
            "--replay" => want_value = Some("--replay"),
            "--hw" => o.backend = Backend::Linux,
            "--sim" => o.backend = Backend::Sim,
            "--trace" => o.trace = true,
            "--virtual-time" => o.virtual_time = true,
            "--no-vm" => o.no_vm = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("tulpa {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            s if s.starts_with("--") => {
                eprintln!("tulpa: unknown flag {s}\n\n{USAGE}");
                return ExitCode::from(2);
            }
            _ => {
                if !matches!(
                    a.as_str(),
                    "run"
                        | "test"
                        | "check"
                        | "repl"
                        | "ast"
                        | "verify"
                        | "graph"
                        | "emit-llvm"
                        | "build"
                        | "bytecode"
                        | "lsp"
                ) || !rest.is_empty()
                {
                    seen_file = true;
                }
                rest.push(a);
            }
        }
    }
    match rest.first().map(String::as_str) {
        None | Some("repl") => repl(&o),
        Some("run") => match rest.get(1) {
            Some(f) => run_file(f, &rest[2..], &o),
            None => {
                eprintln!("tulpa run: missing file");
                ExitCode::from(2)
            }
        },
        Some("test") => test_files(&rest[1..], &o),
        Some("check") => {
            let mut ok = true;
            for f in &rest[1..] {
                match load(f) {
                    Ok(_) => println!("{f}: ok"),
                    Err(_) => ok = false,
                }
            }
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(65)
            }
        }
        Some("verify") => {
            let mut trials = 300;
            let mut files = Vec::new();
            for a in &rest[1..] {
                match a.strip_prefix("--trials=") {
                    Some(n) => trials = n.parse().unwrap_or(300),
                    None => files.push(a.clone()),
                }
            }
            verify_files(&files, &o, trials)
        }
        Some("graph") => graph_files(&rest[1..], &o),
        Some("emit-llvm") | Some("build") => native(&rest, rest[0] == "build"),
        Some("bytecode") => bytecode(&rest[1..]),
        Some("lsp") => tulpa::lsp::serve(),
        Some("ast") => match rest.get(1).map(|f| load(f)) {
            Some(Ok((_, prog))) => {
                for s in prog {
                    println!("{s:#?}");
                }
                ExitCode::SUCCESS
            }
            Some(Err(c)) => c,
            None => ExitCode::from(2),
        },
        Some(file) => run_file(file, &rest[1..], &o),
    }
}

fn main() -> ExitCode {
    // Tree-walking evaluation recurses on the native stack; give it plenty of room.
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(real_main)
        .expect("failed to start interpreter thread")
        .join()
        .unwrap_or(ExitCode::from(101))
}
