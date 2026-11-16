//! `tulpa` command-line interface.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use tulpa::{parse, Backend, Interpreter, ParseError};

const USAGE: &str = "\
Tulpa {VERSION} — a language for robots and hardware

USAGE:
    tulpa [FLAGS] <file.tlp|file.rav> [args...]   run a program
    tulpa run [FLAGS] <file>                      same as above
    tulpa test [FLAGS] <file>...                  run define_test! blocks
    tulpa check <file>...                         parse only, report syntax errors
    tulpa repl [FLAGS]                            interactive prompt (default with no file)
    tulpa ast <file>                              dump the parsed syntax tree
    tulpa verify [--trials=N] <file>...           fuzz every fn against its own require/ensure
    tulpa prove [--strict] [--json] <file>...     statically prove contracts, invariants, bounds
    tulpa bundle <file> [-o out] [--hw] [--runtime tulpa-binary]
                                                  standalone executable with the full language
    tulpa deploy [dir] [--port P] [--host user@host] [--dry-run] [--no-flash] [--force]
                                                  prove, check, build and flash a `tulpa new` project
    tulpa link graph|echo <topic> [--domain N] [--for SECS]   inspect a live Tulpa Link network
    tulpa link bridge <serial-port> [--baud N] [--domain N]   join a board's UART link node to it
    tulpa graph <file>...                         print Mermaid diagrams of every state machine
    tulpa emit-llvm <file> [--target T] [-o out.ll]  LLVM IR for the program
    tulpa build <file> [--target T] [-o out]      native object + C header (needs clang)
        --target T   sim | linux | rp2040 | esp32 | esp32s3 | esp32c3 | esp32c6 | ... (tulpa targets)
        --exe        link a runnable program (targets sim, linux)
        --dynamic    run the program on the Tulpa VM for firmware (automatic on microcontroller
                     targets when the program is outside the static native subset)
        --sim        allow sim::* calls (you link hal_sim.c, e.g. to test firmware in QEMU)
        --shared     shared library (host)
    tulpa new <dir> --board B                     firmware project for a board (rp2040, esp32c3, ...)
    tulpa runtime <dir> [--target T]              write the C runtime + HAL port for your own build
    tulpa targets                                 list native targets
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
            Ok(t) => {
                // a timed tape replays on a virtual clock driven by its timestamps
                if t.events.iter().any(|e| e.t.is_some()) {
                    i.virtual_time = Some(0.0);
                }
                i.tape = Some(std::rc::Rc::new(std::cell::RefCell::new(t)))
            }
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
                        "{file}: replay diverged: script finished with {} recorded device command(s) unused (next: {} {})",
                        t.remaining(),
                        t.next_command().map(|c| c.0).unwrap_or("?"),
                        t.next_command().map(|c| c.1).unwrap_or("?")
                    );
                    code = ExitCode::from(70);
                } else if code == ExitCode::SUCCESS {
                    eprintln!(
                        "tulpa: replay matched all {} device commands",
                        t.command_count()
                    );
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

/// Builds a hosted bundle: `runtime` + the program and its imports.
fn make_bundle(
    file: &str,
    out: &str,
    runtime: Option<&str>,
    hardware: bool,
) -> Result<usize, String> {
    let mut p = tulpa::bundle::collect(std::path::Path::new(file))?;
    p.hardware = hardware;
    let rt = match runtime {
        Some(r) => std::path::PathBuf::from(r),
        None => std::env::current_exe().map_err(|e| e.to_string())?,
    };
    tulpa::bundle::write(&rt, std::path::Path::new(out), &p)?;
    Ok(p.files.len())
}

/// `tulpa bundle file.rav [-o out] [--hw] [--runtime BIN]`
fn bundle(args: &[String]) -> ExitCode {
    let (mut file, mut out, mut runtime, mut hw) = (None, None, None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = it.next().cloned(),
            "--runtime" => runtime = it.next().cloned(),
            "--hw" => hw = true,
            f => file = Some(f.to_string()),
        }
    }
    let Some(file) = file else {
        eprintln!("usage: tulpa bundle <file> [-o out] [--hw] [--runtime tulpa-binary]");
        return ExitCode::from(2);
    };
    let stem = std::path::Path::new(&file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".into());
    let out = out.unwrap_or_else(|| {
        if cfg!(windows) {
            format!("{stem}.exe")
        } else {
            stem
        }
    });
    match make_bundle(&file, &out, runtime.as_deref(), hw) {
        Ok(n) => {
            eprintln!(
                "bundled {out}: {n} source file(s) + the Tulpa runtime{}",
                if hw { ", hardware backend" } else { "" }
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tulpa bundle: {e}");
            ExitCode::from(2)
        }
    }
}

/// `tulpa deploy`: the simulation-to-hardware pipeline for a `tulpa new` project.
///   1. prove      static proof of contracts and invariants; anything refuted stops the deploy
///   2. check      the program compiles natively for the simulator (fast, no SDK needed)
///   3. build      the board's own build (Pico SDK, ESP-IDF, or make)
///   4. flash      picotool / UF2 drive, idf.py flash, or scp to a Linux board
fn deploy(args: &[String], o: &Opts) -> ExitCode {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    let (mut dir, mut port, mut host) = (String::from("."), None, None);
    let (mut dry, mut flash, mut force) = (false, true, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => port = it.next().cloned(),
            "--host" => host = it.next().cloned(),
            "--dry-run" => dry = true,
            "--no-flash" => flash = false,
            "--force" => force = true,
            d => dir = d.to_string(),
        }
    }
    let root = PathBuf::from(&dir);
    let app = root.join("app.rav");
    if !app.is_file() {
        eprintln!("tulpa deploy: {} has no app.rav (create a project with `tulpa new <dir> --board <board>`)", root.display());
        return ExitCode::from(2);
    }
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap_or_default();
    let name = std::fs::canonicalize(&root)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "app".into());
    let board = if root.join("sdkconfig.defaults").is_file() {
        "esp-idf"
    } else if read("CMakeLists.txt").contains("pico_sdk") {
        "rp2040"
    } else if root.join("Makefile").is_file() {
        "linux"
    } else {
        eprintln!("tulpa deploy: cannot tell which board {} is for (no CMakeLists.txt, sdkconfig.defaults or Makefile)", root.display());
        return ExitCode::from(2);
    };
    println!("deploying {name} ({board})");

    // 1. prove
    let (_, prog) = match load(&app.to_string_lossy()) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let report = prove_and_test(&app.to_string_lossy(), &prog, o, 300);
    use tulpa::prove::Status;
    println!(
        "  [1/4] prove   {} proved, {} unproved, {} skipped",
        report.count(Status::Proved),
        report.count(Status::Unproved),
        report.count(Status::Skipped)
    );
    let refuted: Vec<_> = report
        .obligations
        .iter()
        .filter(|o| o.status == Status::Refuted)
        .collect();
    for o in &refuted {
        println!("        REFUTED {}: {} {}", o.ctx, o.text, o.detail);
    }
    if !refuted.is_empty() && !force {
        eprintln!("tulpa deploy: stopped: fix the refuted obligations or pass --force");
        return ExitCode::from(1);
    }

    let me = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tulpa"));
    let run = |step: &str, cmd: &mut Command| -> bool {
        let shown = format!(
            "{} {}",
            cmd.get_program().to_string_lossy(),
            cmd.get_args()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!("  {step}  {shown}");
        if dry {
            return true;
        }
        match cmd.status() {
            Ok(s) if s.success() => true,
            Ok(s) => {
                eprintln!("tulpa deploy: `{shown}` failed ({s})");
                false
            }
            Err(e) => {
                eprintln!("tulpa deploy: cannot run `{shown}`: {e}");
                false
            }
        }
    };

    // 2. native check on the simulator
    let check = std::env::temp_dir().join(format!("tulpa-deploy-{}-{name}", std::process::id()));
    let ok = run(
        "[2/4] check ",
        Command::new(&me)
            .args(["build", "--target", "sim", "--exe"])
            .arg(&app)
            .arg("-o")
            .arg(&check)
            .stdout(std::process::Stdio::null()),
    );
    let _ = std::fs::remove_file(&check);
    let _ = std::fs::remove_dir_all(check.with_extension("runtime"));
    if !ok {
        return ExitCode::from(1);
    }

    // 3. build
    let tulpa_dir = me.parent().map(Path::to_path_buf).unwrap_or_default();
    let path = format!(
        "{}:{}",
        tulpa_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let built = match board {
        "rp2040" => {
            run(
                "[3/4] build ",
                Command::new("cmake")
                    .args(["-B", "build"])
                    .current_dir(&root)
                    .env("PATH", &path),
            ) && run(
                "[3/4] build ",
                Command::new("cmake")
                    .args(["--build", "build"])
                    .current_dir(&root)
                    .env("PATH", &path),
            )
        }
        "esp-idf" => run(
            "[3/4] build ",
            Command::new("idf.py")
                .arg("build")
                .current_dir(&root)
                .env("PATH", &path),
        ),
        _ => run(
            "[3/4] build ",
            Command::new("make").current_dir(&root).env("TULPA", &me),
        ),
    };
    if !built {
        return ExitCode::from(1);
    }
    if !flash {
        println!("  [4/4] flash   skipped (--no-flash)");
        return ExitCode::SUCCESS;
    }

    // 4. flash
    let done = match board {
        "rp2040" => {
            let uf2 = root.join("build").join(format!("{name}.uf2"));
            let drive = port.map(PathBuf::from).or_else(|| {
                [
                    "/Volumes/RPI-RP2".to_string(),
                    format!(
                        "/media/{}/RPI-RP2",
                        std::env::var("USER").unwrap_or_default()
                    ),
                    "/run/media/RPI-RP2".into(),
                ]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.is_dir())
            });
            match drive {
                Some(d) => run("[4/4] flash ", Command::new("cp").arg(&uf2).arg(&d)),
                None => run(
                    "[4/4] flash ",
                    Command::new("picotool")
                        .args(["load", "-f", "-x"])
                        .arg(&uf2),
                ),
            }
        }
        "esp-idf" => {
            let mut c = Command::new("idf.py");
            if let Some(p) = &port {
                c.args(["-p", p]);
            }
            run(
                "[4/4] flash ",
                c.arg("flash").current_dir(&root).env("PATH", &path),
            )
        }
        _ => match &host {
            Some(h) => {
                run(
                    "[4/4] flash ",
                    Command::new("scp")
                        .arg(root.join(&name))
                        .arg(format!("{h}:{name}")),
                ) && {
                    println!("        run it with: ssh {h} sudo ./{name}");
                    true
                }
            }
            None => {
                println!("  [4/4] flash   built ./{name} here; pass --host user@board to copy it to a board");
                true
            }
        },
    };
    if done {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `tulpa link graph` / `tulpa link echo <topic>`: joins the UDP network as an observer.
fn link_tool(args: &[String], o: &Opts) -> ExitCode {
    let mut domain = 0u32;
    let mut secs: Option<f64> = None;
    let mut words = Vec::new();
    let mut baud = 115200u32;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--domain" => domain = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--for" => secs = it.next().and_then(|v| v.parse().ok()),
            "--baud" => baud = it.next().and_then(|v| v.parse().ok()).unwrap_or(115200),
            w => words.push(w.to_string()),
        }
    }
    if let [cmd, port] = words.as_slice() {
        if cmd == "bridge" {
            return match tulpa::link::bridge(port, baud, domain as u16, secs) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("tulpa link bridge: {e}");
                    ExitCode::from(1)
                }
            };
        }
    }
    let src = match words
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["graph"] => format!(
            r#"import "std/link"
let n = link.node("tulpa-link-{pid}", domain: {domain}, transport: "udp")
link.spin_for({t}[s], 20[ms])
let g = n.graph()
for x in g.nodes {{
    if x.name == n.name {{ continue }}
    print(x.name)
    for t in x.publishes {{ print("  pub ", t) }}
    for t in x.subscribes {{ print("  sub ", t) }}
    for t in x.services {{ print("  srv ", t) }}
}}
if len(g.nodes) == 1 {{ print("no nodes found on domain {domain}") }}
n.close()"#,
            pid = std::process::id(),
            t = secs.unwrap_or(1.0)
        ),
        ["echo", topic] => format!(
            r#"import "std/link"
let n = link.node("tulpa-link-{pid}", domain: {domain}, transport: "udp")
n.subscribe("{topic}", "any", |v, m| print(f"[{{m.from}} #{{m.seq}}]", v))
link.spin_for({t}[s], 5[ms])
n.close()"#,
            pid = std::process::id(),
            t = secs.unwrap_or(1e9)
        ),
        _ => {
            eprintln!("usage: tulpa link graph [--domain N] [--for SECS]\n       tulpa link echo <topic> [--domain N] [--for SECS]\n       tulpa link bridge <serial-port> [--baud N] [--domain N] [--for SECS]   join a board's UART link to the UDP domain");
            return ExitCode::from(2);
        }
    };
    let prog = match parse(&src) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tulpa link: {}", e.msg);
            return ExitCode::from(70);
        }
    };
    let mut i = interpreter(o, &[]);
    match i.run(&prog) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tulpa link: {e}");
            ExitCode::from(70)
        }
    }
}

/// Static proof, then testing of every function the prover left open.
fn prove_and_test(
    file: &str,
    prog: &[tulpa::ast::Stmt],
    o: &Opts,
    trials: usize,
) -> tulpa::prove::Report {
    let mut report = tulpa::prove::prove_program(prog);
    // test the functions the prover could not settle
    let open = report.open_contexts();
    if !open.is_empty() {
        if let Ok(mut i) = load_definitions(file, o) {
            for (name, v) in i.global_functions() {
                let ctx = format!("{name}()");
                if !open.contains(&ctx) {
                    continue;
                }
                let Ok(tulpa::Value::Dict(d)) = i.check_fn(&v, trials) else {
                    continue;
                };
                let get = |k: &str| {
                    d.borrow()
                        .get(&tulpa::Value::str(k))
                        .ok()
                        .flatten()
                        .unwrap_or(tulpa::Value::Nil)
                };
                if get("ok").truthy() {
                    report.tested(&ctx, trials);
                } else {
                    let args = match get("counterexample") {
                        tulpa::Value::List(l) => l
                            .borrow()
                            .iter()
                            .map(|v| match v {
                                tulpa::Value::Float(f)
                                    if *f != 0.0 && (f.abs() < 1e-4 || f.abs() >= 1e15) =>
                                {
                                    format!("{f:e}")
                                }
                                v => v.repr(),
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                        _ => String::new(),
                    };
                    report.refute(&ctx, &get("error").to_string(), &args);
                }
            }
        }
    }
    report
}

/// `tulpa prove`: static proof first, then testing to find counterexamples for whatever is
/// left open. Exit code 1 if anything is refuted (or, with --strict, not proved).
fn prove_files(files: &[String], o: &Opts, trials: usize, strict: bool, json: bool) -> ExitCode {
    use tulpa::prove::Status;
    if files.is_empty() {
        eprintln!("usage: tulpa prove [--strict] [--json] [--trials=N] <file>...");
        return ExitCode::from(2);
    }
    let mut bad = false;
    let mut reports = Vec::new();
    for file in files {
        let (_, prog) = match load(file) {
            Ok(p) => p,
            Err(c) => return c,
        };
        let report = prove_and_test(file, &prog, o, trials);
        bad |= report.count(Status::Refuted) > 0
            || (strict && report.count(Status::Unproved) + report.count(Status::Skipped) > 0);
        if json {
            reports.push(report.to_json(file));
            continue;
        }
        println!("{file}");
        let mut ctx = String::new();
        for ob in &report.obligations {
            if ob.ctx != ctx {
                ctx = ob.ctx.clone();
                println!("  {ctx}");
            }
            let mark = match ob.status {
                Status::Proved => "proved  ",
                Status::Refuted => "REFUTED ",
                Status::Unproved => "unproved",
                Status::Skipped => "skipped ",
            };
            let line = if ob.line > 0 {
                format!("  (line {})", ob.line)
            } else {
                String::new()
            };
            println!("    {mark}  {}{line}", ob.text);
            if !ob.detail.is_empty() {
                println!("              {}", ob.detail);
            }
        }
        println!(
            "\n{} proved, {} refuted, {} unproved, {} skipped",
            report.count(Status::Proved),
            report.count(Status::Refuted),
            report.count(Status::Unproved),
            report.count(Status::Skipped)
        );
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&reports).unwrap_or_default()
        );
    }
    if bad {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
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

/// `tulpa emit-llvm file.rav [--target T] [-o out.ll]` /
/// `tulpa build file.rav [--target T] [-o out] [--exe] [--shared] [-O2]`
fn native(rest: &[String], build: bool) -> ExitCode {
    let mut file = None;
    let mut out: Option<String> = None;
    let mut shared = false;
    let mut exe = false;
    let mut sim_hal = false;
    let mut dynamic = false;
    let mut target_name: Option<String> = None;
    let mut opt: Option<String> = None;
    let mut it = rest[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = it.next().cloned(),
            "--dynamic" => dynamic = true,
            "--shared" => shared = true,
            "--exe" => exe = true,
            "--sim" => sim_hal = true,
            "--target" => target_name = it.next().cloned(),
            t if t.starts_with("--target=") => {
                target_name = Some(t["--target=".len()..].to_string())
            }
            "-O0" | "-O1" | "-O2" | "-O3" | "-Os" | "-Oz" => opt = Some(a.clone()),
            f if f.starts_with('-') => {
                eprintln!("tulpa {}: unknown option {f}", rest[0]);
                return ExitCode::from(2);
            }
            f => file = Some(f.to_string()),
        }
    }
    let Some(file) = file else {
        eprintln!(
            "usage: tulpa {} <file.rav> [--target T] [-o out]",
            if build { "build" } else { "emit-llvm" }
        );
        return ExitCode::from(2);
    };
    if exe && target_name.is_none() {
        target_name = Some(
            if cfg!(target_os = "linux") {
                "linux"
            } else {
                "sim"
            }
            .to_string(),
        );
    }
    let target = match &target_name {
        None => None,
        Some(n) => match tulpa::native::target(n) {
            Some(t) => Some(t),
            None => {
                eprintln!("tulpa: unknown target '{n}'; `tulpa targets` lists them");
                return ExitCode::from(2);
            }
        },
    };
    if exe && !target.is_some_and(|t| t.exe) {
        eprintln!("tulpa: --exe links a host program; use --target sim or --target linux (firmware targets produce an object for the board's SDK)");
        return ExitCode::from(2);
    }
    let (_, prog) = match load(&file) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let whole = prog.clone();
    let (prog, aliases) = match tulpa::native::flatten_imports(prog, std::path::Path::new(&file)) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("tulpa: {e}");
            return ExitCode::from(2);
        }
    };
    let stem = std::path::Path::new(&file)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "tulpa".into());
    let opts = tulpa::llvm::Options {
        embedded: target.is_some(),
        sim: sim_hal || target.is_some_and(|t| t.hal == tulpa::native::Hal::Sim),
        module_aliases: aliases,
    };
    let mut report = tulpa::llvm::emit_program(&prog, &stem, &opts);
    // The dynamic tier: when the program's top level is outside the static native subset and
    // the target is a microcontroller, the whole program runs on the Tulpa VM for firmware.
    let top_failed = report.skipped.iter().any(|(n, _)| n == "<program>");
    let use_dyn = !shared && (dynamic || (top_failed && target.is_some_and(|t| t.embedded)));
    if use_dyn {
        let p = match tulpa::dyncode::serialize(
            &whole,
            std::path::Path::new(&file),
            opts.sim || target.is_none(),
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("tulpa: {file}: {e}");
                return ExitCode::from(1);
            }
        };
        if !dynamic {
            for (name, why) in report.skipped.iter().filter(|(n, _)| n == "<program>") {
                eprintln!("  {name} is outside the static native subset ({why})");
            }
        }
        eprintln!(
            "  dynamic tier: {} module(s), {} nodes, {} bytes of T-code; runs on the Tulpa VM (tulpa_dyn.c)",
            p.modules,
            p.nodes,
            p.bytes.len()
        );
        report.ir = tulpa::dyncode::emit_ll(&p, &stem);
        report.compiled.clear();
        report.skipped.clear();
        report.has_entry = true;
        report.needs_runtime = true;
    }
    for (name, sig) in &report.compiled {
        let ps: Vec<String> = sig
            .params
            .iter()
            .zip(sig.cparams.iter())
            .map(|((n, _), c)| format!("{n}: {}", c.trim_end()))
            .collect();
        let ret = if sig.void {
            "void".to_string()
        } else {
            sig.cret.trim_end().to_string()
        };
        eprintln!(
            "  native  {}({}) -> {ret}",
            name.replace("__", "."),
            ps.join(", ")
        );
    }
    for (name, why) in &report.skipped {
        eprintln!("  skipped {name}: {why}");
    }
    if exe && !report.has_entry {
        // Hosted targets still get a standalone executable: the program runs on the embedded
        // interpreter instead of as static native code.
        let hosted = target.is_none_or(|t| !t.embedded);
        if build && hosted {
            let out = out.clone().unwrap_or_else(|| stem.clone());
            let hw = target.is_some_and(|t| t.name == "linux");
            return match make_bundle(&file, &out, None, hw) {
                Ok(n) => {
                    eprintln!("built {out}: hosted bundle ({n} source file(s) + the Tulpa runtime), because the program uses features outside the static native subset (see 'skipped' above)");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("tulpa: {e}");
                    ExitCode::from(1)
                }
            };
        }
        eprintln!("tulpa: --exe needs the program's top-level code to compile natively (see 'skipped <program>' above)");
        return ExitCode::from(1);
    }
    if report.compiled.is_empty() && !use_dyn {
        eprintln!("tulpa: nothing in {file} fits the native subset");
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
    let default_out = if exe {
        stem.clone()
    } else if shared {
        format!("{stem}.so")
    } else {
        format!("{stem}.o")
    };
    let out = out.unwrap_or(default_out);
    let base = out
        .strip_suffix(".so")
        .or_else(|| out.strip_suffix(".o"))
        .unwrap_or(&out)
        .to_string();
    let ll = format!("{base}.ll");
    let header = format!("{base}.h");
    let guard = format!(
        "TULPA_{}_H",
        stem.to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    let header_text = if use_dyn {
        format!("#ifndef {guard}\n#define {guard}\n/* This program runs on the Tulpa VM (tulpa_dyn.c); tulpa_entry() starts it. */\n#include \"tulpa_hal.h\"\n#endif\n")
    } else {
        report.header(&guard)
    };
    if let Err(e) =
        std::fs::write(&ll, &report.ir).and_then(|_| std::fs::write(&header, header_text))
    {
        eprintln!("tulpa: {e}");
        return ExitCode::from(2);
    }
    // the header includes tulpa_hal.h when the program uses devices or the runtime
    let out_dir = std::path::Path::new(&header)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    if report.needs_runtime || target.is_some() {
        if let Err(e) = std::fs::write(out_dir.join("tulpa_hal.h"), tulpa::native::HAL_H) {
            eprintln!("tulpa: {e}");
            return ExitCode::from(2);
        }
    }
    let xtensa = target.is_some_and(|t| t.xtensa);
    let (cc, major) = match find_clang(xtensa) {
        Ok(x) => x,
        Err(msg) => {
            eprintln!("tulpa: {msg}; the IR is in {ll} and the header in {header}");
            return ExitCode::from(1);
        }
    };
    let opt = opt.unwrap_or_else(|| {
        if target.is_some_and(|t| t.embedded) {
            "-Os"
        } else {
            "-O2"
        }
        .to_string()
    });
    let mut cmd = std::process::Command::new(&cc);
    cmd.args([opt.as_str(), "-Wno-override-module"]);
    if major == Some(14) {
        // The IR uses opaque `ptr`, which clang 14 only accepts behind this flag.
        cmd.args(["-mllvm", "-opaque-pointers"]);
    }
    if let Some(t) = target {
        for f in t.cflags {
            // clang < 16 rejects the explicit Zicsr/Zifencei extensions; in the ISA 2.0
            // spelling those clangs use, the base `i` already includes them.
            match f.strip_suffix("_zicsr_zifencei") {
                Some(base) if major.is_some_and(|m| m < 16) => cmd.arg(base),
                _ => cmd.arg(f),
            };
        }
        if t.embedded {
            cmd.args(["-ffunction-sections", "-fdata-sections"]);
        }
    }
    let obj = if exe {
        format!("{base}.o")
    } else {
        out.clone()
    };
    if shared {
        cmd.args(["-shared", "-fPIC"]);
    } else {
        cmd.arg("-c");
    }
    cmd.args([&ll, "-o", if shared { &out } else { &obj }]);
    if shared {
        cmd.arg("-lm");
    }
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!("tulpa: {cc} failed compiling {ll}");
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("tulpa: cannot run {cc} ({e}); the IR is in {ll} and the header in {header}");
            return ExitCode::from(1);
        }
    }
    if !exe {
        eprintln!("built {out} (+ {header}, {ll})");
        if let Some(t) = target {
            if t.embedded {
                let hal = t.hal.source().map(|(n, _)| n).unwrap_or("your HAL port");
                eprintln!(
                    "link it with tulpa_rt.c{} and {hal} (`tulpa runtime <dir> --target {}` writes them; `tulpa new` scaffolds a whole project)",
                    if use_dyn { ", tulpa_dyn.c" } else { "" },
                    t.name
                );
            }
        }
        return ExitCode::SUCCESS;
    }
    // --exe: link with the runtime and the target's HAL into a host program
    let t = target.unwrap();
    let rtdir = std::path::PathBuf::from(format!("{base}.runtime"));
    let files = match tulpa::native::write_runtime(&rtdir, t.hal, use_dyn) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("tulpa: {e}");
            return ExitCode::from(2);
        }
    };
    let main_c = rtdir.join("main.c");
    if let Err(e) = std::fs::write(
        &main_c,
        "#include \"tulpa_hal.h\"\nint main(void) { tulpa_entry(); return 0; }\n",
    ) {
        eprintln!("tulpa: {e}");
        return ExitCode::from(2);
    }
    let mut link = std::process::Command::new(&cc);
    link.args([opt.as_str(), "-std=c11", "-I"]).arg(&rtdir);
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "c"))
    {
        link.arg(f);
    }
    link.arg(&main_c).arg(&obj).args(["-o", &out, "-lm"]);
    match link.status() {
        Ok(s) if s.success() => {
            eprintln!(
                "built {out} (target {}; runtime in {})",
                t.name,
                rtdir.display()
            );
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("tulpa: linking {out} failed");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("tulpa: cannot run {cc} ({e})");
            ExitCode::from(1)
        }
    }
}

/// `tulpa new <dir> --board rp2040`
fn new_project(rest: &[String]) -> ExitCode {
    let mut dir = None;
    let mut board = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--board" | "--target" => board = it.next().cloned(),
            b if b.starts_with("--board=") => board = Some(b["--board=".len()..].to_string()),
            d => dir = Some(d.to_string()),
        }
    }
    let (Some(dir), Some(board)) = (dir, board) else {
        eprintln!("usage: tulpa new <dir> --board <rp2040|esp32|esp32s3|esp32c3|esp32c6|linux>");
        return ExitCode::from(2);
    };
    match tulpa::native::new_project(std::path::Path::new(&dir), &board) {
        Ok(files) => {
            for f in &files {
                println!("  created {}", f.display());
            }
            let next = match tulpa::native::target(&board).map(|t| t.hal) {
                Some(tulpa::native::Hal::Pico) => format!("cd {dir} && cmake -B build -DPICO_SDK_PATH=$PICO_SDK_PATH && cmake --build build"),
                Some(tulpa::native::Hal::EspIdf) => format!("cd {dir} && idf.py set-target {board} && idf.py build flash monitor"),
                _ => format!("cd {dir} && make"),
            };
            println!("\ntry it on the simulator: tulpa run --virtual-time {dir}/app.rav\nbuild the firmware:      {next}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tulpa new: {e}");
            ExitCode::from(2)
        }
    }
}

/// `tulpa runtime <dir> [--target T]`
fn write_runtime(rest: &[String]) -> ExitCode {
    let mut dir = None;
    let mut t = "cortex-m0".to_string();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--target" | "--board" => t = it.next().cloned().unwrap_or_default(),
            d => dir = Some(d.to_string()),
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage: tulpa runtime <dir> [--target T]");
        return ExitCode::from(2);
    };
    let Some(target) = tulpa::native::target(&t) else {
        eprintln!("tulpa: unknown target '{t}'");
        return ExitCode::from(2);
    };
    match tulpa::native::write_runtime(std::path::Path::new(&dir), target.hal, true) {
        Ok(files) => {
            for f in files {
                println!("  wrote {}", f.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tulpa: {e}");
            ExitCode::from(2)
        }
    }
}

/// Picks the C compiler for `tulpa build`: `$TULPA_CLANG`, else `clang`, else the newest
/// installed `clang-N`. Returns the command and its major version when it can be read.
/// Xtensa targets (ESP32, ESP32-S2/S3) need Espressif's clang, which is looked up in the
/// ESP-IDF tools directory (`idf_tools.py install esp-clang`).
fn find_clang(xtensa: bool) -> Result<(String, Option<u32>), String> {
    fn major_of(cc: &str) -> Option<Option<u32>> {
        let out = std::process::Command::new(cc)
            .arg("--version")
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let v = text
            .split("version ")
            .nth(1)
            .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse().ok());
        Some(v)
    }
    // upstream LLVM has a generic, experimental Xtensa backend without the ESP32 CPUs;
    // Espressif's build is the one that targets them
    fn has_xtensa(cc: &str) -> bool {
        std::process::Command::new(cc)
            .arg("--version")
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .to_lowercase()
                    .contains("espressif")
            })
            .unwrap_or(false)
    }
    let explicit = std::env::var("TULPA_CLANG").ok();
    let mut candidates: Vec<String> = match &explicit {
        Some(cc) => vec![cc.clone()],
        None => std::iter::once("clang".to_string())
            .chain((14..=22).rev().map(|n| format!("clang-{n}")))
            .collect(),
    };
    if xtensa && explicit.is_none() {
        let mut roots = Vec::new();
        if let Some(p) = std::env::var_os("IDF_TOOLS_PATH") {
            roots.push(std::path::PathBuf::from(p));
        }
        if let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            roots.push(std::path::PathBuf::from(h).join(".espressif"));
        }
        let mut found = Vec::new();
        for r in roots {
            if let Ok(vers) = std::fs::read_dir(r.join("tools/esp-clang")) {
                for v in vers.flatten() {
                    for exe in ["clang", "clang.exe"] {
                        let p = v.path().join("esp-clang/bin").join(exe);
                        if p.is_file() {
                            found.push(p.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        found.sort();
        found.reverse();
        candidates = found.into_iter().chain(candidates).collect();
    }
    for cc in candidates {
        if let Some(major) = major_of(&cc) {
            if let Some(m) = major {
                if m < 14 {
                    return Err(format!(
                        "{cc} is version {m}; tulpa build needs clang 14 or newer"
                    ));
                }
            }
            if xtensa && !has_xtensa(&cc) {
                if explicit.is_some() {
                    return Err(format!("{cc} is not Espressif's clang, which ESP32 / ESP32-S2 / ESP32-S3 (Xtensa) need"));
                }
                continue;
            }
            return Ok((cc, major));
        }
    }
    if xtensa {
        return Err("ESP32 / ESP32-S2 / ESP32-S3 are Xtensa chips and need Espressif's clang: run `python $IDF_PATH/tools/idf_tools.py install esp-clang`, or set TULPA_CLANG".into());
    }
    Err(match explicit {
        Some(cc) => format!("cannot run {cc} (from TULPA_CLANG)"),
        None => "no clang found (install clang 14+ or set TULPA_CLANG)".into(),
    })
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

fn graph_files(files: &[String], _o: &Opts) -> ExitCode {
    // Purely static: diagrams come straight from the AST, so programs that talk to
    // hardware or a co-simulator at the top level can still be graphed.
    for file in files {
        let (_, prog) = match load(file) {
            Ok(x) => x,
            Err(c) => return c,
        };
        for s in &prog {
            if let tulpa::ast::Stmt::Machine(def) = s {
                println!(
                    "%% machine {} ({file})\n{}",
                    def.name,
                    tulpa::ext::mermaid(def)
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
    tulpa::embed::balanced(src)
}

fn repl(o: &Opts) -> ExitCode {
    println!(
        "Tulpa {} REPL — backend: {:?}. Type :quit to exit, :help for help.",
        env!("CARGO_PKG_VERSION"),
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
    // A bundle (see `tulpa bundle`) runs its embedded program; the command line is the
    // program's own arguments.
    if let Some(p) = tulpa::bundle::embedded() {
        match tulpa::bundle::extract(&p) {
            Ok((main, lib)) => {
                let mut path = vec![lib];
                if let Some(old) = std::env::var_os("TULPA_PATH") {
                    path.extend(std::env::split_paths(&old));
                }
                if let Ok(joined) = std::env::join_paths(path) {
                    std::env::set_var("TULPA_PATH", joined);
                }
                args.insert(0, main.to_string_lossy().into_owned());
                if p.hardware {
                    args.insert(0, "--hw".into());
                }
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(70);
            }
        }
    }
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
        let native_cmd = matches!(
            rest.first().map(String::as_str),
            Some(
                "build"
                    | "emit-llvm"
                    | "new"
                    | "runtime"
                    | "targets"
                    | "prove"
                    | "link"
                    | "deploy"
                    | "bundle"
            )
        );
        if seen_file || native_cmd {
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
                print!("{}", USAGE.replace("{VERSION}", env!("CARGO_PKG_VERSION")));
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("tulpa {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            s if s.starts_with("--") => {
                eprintln!(
                    "tulpa: unknown flag {s}\n\n{}",
                    USAGE.replace("{VERSION}", env!("CARGO_PKG_VERSION"))
                );
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
                        | "prove"
                        | "link"
                        | "deploy"
                        | "bundle"
                        | "graph"
                        | "emit-llvm"
                        | "build"
                        | "bytecode"
                        | "lsp"
                        | "new"
                        | "runtime"
                        | "targets"
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
        Some("prove") => {
            let mut trials = 500;
            let (mut strict, mut json) = (false, false);
            let mut files = Vec::new();
            for a in &rest[1..] {
                match a.as_str() {
                    "--strict" => strict = true,
                    "--json" => json = true,
                    s => match s.strip_prefix("--trials=") {
                        Some(n) => trials = n.parse().unwrap_or(500),
                        None => files.push(a.clone()),
                    },
                }
            }
            prove_files(&files, &o, trials, strict, json)
        }
        Some("link") => link_tool(&rest[1..], &o),
        Some("deploy") => deploy(&rest[1..], &o),
        Some("bundle") => bundle(&rest[1..]),
        Some("graph") => graph_files(&rest[1..], &o),
        Some("emit-llvm") | Some("build") => native(&rest, rest[0] == "build"),
        Some("new") => new_project(&rest[1..]),
        Some("runtime") => write_runtime(&rest[1..]),
        Some("targets") => {
            for t in tulpa::native::TARGETS {
                println!("  {:<11} {}", t.name, t.about);
            }
            ExitCode::SUCCESS
        }
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
