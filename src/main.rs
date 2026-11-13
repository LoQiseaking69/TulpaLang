//! `tulpa` command-line interface.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use tulpa::{parse, Backend, Interpreter, ParseError};

const USAGE: &str = "\
Tulpa 1.0 — device-integration scripting language

USAGE:
    tulpa [FLAGS] <file.tlp|file.rav> [args...]   run a program
    tulpa run [FLAGS] <file>                      same as above
    tulpa test [FLAGS] <file>...                  run define_test! blocks
    tulpa check <file>...                         parse only, report syntax errors
    tulpa repl [FLAGS]                            interactive prompt (default with no file)
    tulpa ast <file>                              dump the parsed syntax tree

FLAGS:
    --hw        use real Linux hardware (sysfs GPIO, /dev/i2c-*, /dev/spidev*, TTYs)
    --sim       use the in-memory simulator (default)
    --trace     log every device call to stderr
    -h, --help  show this help
    -V          print version
";

struct Opts {
    backend: Backend,
    trace: bool,
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
    match i.run(&prog) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{file}: runtime error: {e}");
            ExitCode::from(70)
        }
    }
}

fn test_files(files: &[String], o: &Opts) -> ExitCode {
    let (mut passed, mut failed) = (0, 0);
    for file in files {
        let (_, prog) = match load(file) {
            Ok(x) => x,
            Err(c) => return c,
        };
        let mut i = interpreter(o, &[]);
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
        "Tulpa 1.0 REPL — backend: {:?}. Type :quit to exit, :help for help.",
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
    for a in args.drain(..) {
        if seen_file {
            rest.push(a);
            continue;
        }
        match a.as_str() {
            "--hw" => o.backend = Backend::Linux,
            "--sim" => o.backend = Backend::Sim,
            "--trace" => o.trace = true,
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
                if !matches!(a.as_str(), "run" | "test" | "check" | "repl" | "ast")
                    || !rest.is_empty()
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
