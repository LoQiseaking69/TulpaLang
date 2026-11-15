//! Embedding API: everything an IDE or app needs, as plain functions over source text.
//!
//! Used by the Android IDE (`android/native`) through JNI, and usable from any Rust host:
//!
//! * [`run`] — run a program with streamed output, `input()` from the host, and a cancel flag
//! * [`Session`] — an incremental REPL that keeps its state between entries
//! * [`diagnostics`], [`completion`], [`hover`], [`symbols`], [`definition`] — editor intelligence
//!   (the same analysis `tulpa lsp` serves), as JSON strings
//! * [`machines`] — every state machine as a JSON graph (states, transitions, guards)
//! * [`verify`], [`tests`], [`bytecode`] — contract fuzzing, `define_test!` blocks, VM listing
//!
//! All functions are synchronous. [`run`] and [`Session`] must stay on the thread that created
//! them (the interpreter uses `Rc`); run them on a thread with a large stack (see
//! [`RECOMMENDED_STACK`]) because the evaluator recurses on the native stack.

use crate::ast::Stmt;
use crate::interp::{Env, InputHook, Interpreter, Output};
use crate::value::Value;
use serde_json::{json, Value as J};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Stack size for threads that run Tulpa code (deep recursion lives on the native stack).
pub const RECOMMENDED_STACK: usize = 64 * 1024 * 1024;

/// Options for [`run`].
pub struct RunConfig {
    /// Directory `import` paths resolve against (usually the file's folder).
    pub base_dir: PathBuf,
    /// Values for the program's `ARGS` list.
    pub args: Vec<String>,
    /// Simulated clock (`--virtual-time`): sleeps and timing rules advance instantly.
    pub virtual_time: bool,
    /// Receives everything the program prints (including `@@tlm` telemetry lines).
    pub output: Rc<dyn Fn(&str)>,
    /// Supplies lines for `input()`. `None` → reading input is an error.
    pub input: Option<InputHook>,
    /// Raise to stop the program at the next loop iteration / sleep.
    pub cancel: Arc<AtomicBool>,
}

fn new_interpreter(
    cfg_out: Rc<dyn Fn(&str)>,
    input: Option<InputHook>,
    cancel: Arc<AtomicBool>,
) -> Interpreter {
    let mut i = Interpreter::new();
    i.output = Output::Sink(cfg_out);
    i.input_hook = input;
    i.cancel = Some(cancel);
    i
}

/// `file:line:col: message` for a parse error, with the offending line and a caret.
pub fn format_parse_error(file: &str, src: &str, e: &crate::parser::ParseError) -> String {
    let line = src.lines().nth(e.line.saturating_sub(1)).unwrap_or("");
    format!(
        "{file}:{}:{}: {}\n    {line}\n    {}^",
        e.line,
        e.col,
        e.msg,
        " ".repeat(e.col.saturating_sub(1))
    )
}

/// Runs a whole program. `Ok(())` on normal completion; `Err(message)` on a parse or runtime
/// error, or `"interrupted"` after the cancel flag was raised.
pub fn run(src: &str, file: &str, cfg: RunConfig) -> Result<(), String> {
    let prog = crate::parse(src).map_err(|e| format_parse_error(file, src, &e))?;
    let mut i = new_interpreter(cfg.output, cfg.input, cfg.cancel);
    i.base_dirs = vec![cfg.base_dir];
    if cfg.virtual_time {
        i.virtual_time = Some(0.0);
    }
    let argv = Value::list(cfg.args.iter().map(|a| Value::str(a.as_str())).collect());
    crate::interp::define(&i.globals, "ARGS", argv);
    i.run(&prog).map(|_| ()).map_err(|e| e.to_string())
}

/// Result of feeding a line to a [`Session`].
#[derive(Debug, PartialEq)]
pub enum Feed {
    /// Braces aren't balanced yet — keep feeding lines.
    More,
    /// Evaluated. Holds the `repr` of an expression's value (`None` for statements / nil).
    Value(Option<String>),
    /// Parse or runtime error.
    Error(String),
}

/// Incremental REPL: definitions and variables persist between entries.
pub struct Session {
    i: Interpreter,
    env: Env,
    buf: String,
}

impl Session {
    pub fn new(
        output: Rc<dyn Fn(&str)>,
        input: Option<InputHook>,
        cancel: Arc<AtomicBool>,
    ) -> Session {
        let i = new_interpreter(output, input, cancel);
        let env = i.globals.clone();
        Session {
            i,
            env,
            buf: String::new(),
        }
    }

    /// True while a multi-line entry is open.
    pub fn pending(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Discards a half-typed multi-line entry.
    pub fn cancel_entry(&mut self) {
        self.buf.clear();
    }

    pub fn set_base_dir(&mut self, dir: PathBuf) {
        self.i.base_dirs = vec![dir];
    }

    /// Feeds one line (or a whole pasted block).
    pub fn feed(&mut self, line: &str) -> Feed {
        self.buf.push_str(line);
        self.buf.push('\n');
        if !balanced(&self.buf) {
            return Feed::More;
        }
        let src = std::mem::take(&mut self.buf);
        if src.trim().is_empty() {
            return Feed::Value(None);
        }
        let prog = match crate::parse(&src) {
            Ok(p) => p,
            Err(e) => return Feed::Error(format_parse_error("<repl>", &src, &e)),
        };
        let is_expr = matches!(prog.last(), Some(Stmt::Expr(..)));
        let r = self.i.run_in(&prog, &self.env);
        let drained = self.i.drain_tasks();
        match (r, drained) {
            (Err(e), _) | (_, Err(e)) => Feed::Error(e.to_string()),
            (Ok(v), _) if is_expr && !matches!(v, Value::Nil) => Feed::Value(Some(v.repr())),
            _ => Feed::Value(None),
        }
    }

    /// Names defined so far (for the REPL's completion).
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.env.borrow().var_names();
        v.sort();
        v
    }
}

/// True when brackets/braces/parens are balanced outside strings and comments.
pub fn balanced(src: &str) -> bool {
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
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

// ---------------------------------------------------------------------------- editor intelligence

/// LSP-style diagnostics: `[{range:{start:{line,character},end:…}, severity, message, …}]`.
pub fn diagnostics(src: &str) -> String {
    J::Array(crate::lsp::diagnostics(src)).to_string()
}

/// Completion items at a zero-based `line`/`col` (the cursor): `[{label, kind, detail}]`.
pub fn completion(src: &str, line: usize, col: usize) -> String {
    J::Array(crate::lsp::completion(src, line + 1, col + 1)).to_string()
}

/// Hover text (markdown) for the character at zero-based `line`/`col` (a tap position).
pub fn hover(src: &str, line: usize, col: usize) -> Option<String> {
    // lsp columns are 1-based cursor positions *after* the character
    crate::lsp::hover(src, line + 1, col + 2)
}

/// Document symbols (functions, structs, machines, constants) for an outline.
pub fn symbols(src: &str) -> String {
    J::Array(crate::lsp::symbols(src)).to_string()
}

/// Definition of the identifier at zero-based `line`/`col` (a tap position):
/// zero-based `{line, col, len}` of its declaration, or `null`.
pub fn definition(src: &str, line: usize, col: usize) -> String {
    match crate::lsp::definition(src, line + 1, col + 2) {
        Some((l, c, n)) => {
            json!({"line": l.saturating_sub(1), "col": c.saturating_sub(1), "len": n}).to_string()
        }
        None => "null".into(),
    }
}

/// Every top-level `machine` as a graph:
/// `[{name, line, initial, states:[{name, line, enter, exit}],
///    transitions:[{from, to, event, params, guard, internal, global, line}]}]`.
pub fn machines(src: &str) -> String {
    let Ok(prog) = crate::parse(src) else {
        return "[]".into();
    };
    let lines: Vec<&str> = src.lines().collect();
    let guard_text = |line: usize| -> Option<String> {
        let l = lines.get(line.checked_sub(1)?)?;
        let start = l.find(" if ")? + 4;
        let end = l[start..].find("=>").map(|e| start + e).unwrap_or(l.len());
        Some(
            l[start..end]
                .trim()
                .trim_end_matches('{')
                .trim()
                .to_string(),
        )
    };
    let mut out = Vec::new();
    for s in &prog {
        let Stmt::Machine(def) = s else { continue };
        if def.states.is_empty() {
            continue;
        }
        let mut trans = Vec::new();
        let tj = |from: &str, t: &crate::ast::Transition, global: bool| {
            json!({
                "from": from,
                "to": t.target.clone().unwrap_or_else(|| from.to_string()),
                "event": t.event,
                "params": t.params,
                "guard": if t.guard.is_some() { guard_text(t.line).map(J::String).unwrap_or(J::String("…".into())) } else { J::Null },
                "internal": t.target.is_none(),
                "global": global,
                "line": t.line,
            })
        };
        for st in &def.states {
            for t in &st.transitions {
                trans.push(tj(&st.name, t, false));
            }
        }
        for t in &def.global {
            trans.push(tj("*", t, true));
        }
        out.push(json!({
            "name": def.name,
            "initial": def.states[0].name,
            "states": def.states.iter().map(|st| json!({
                "name": st.name, "line": st.line,
                "enter": st.enter.is_some(), "exit": st.exit.is_some(),
            })).collect::<Vec<_>>(),
            "transitions": trans,
            "mermaid": crate::ext::mermaid(def),
        }));
    }
    J::Array(out).to_string()
}

/// Bytecode listing for every function (or why it stays interpreted).
pub fn bytecode(src: &str) -> Result<String, String> {
    let prog = crate::parse(src).map_err(|e| format_parse_error("<source>", src, &e))?;
    let mut out = String::new();
    for s in &prog {
        if let Stmt::Fn(def) = s {
            match crate::vm::compile(def) {
                Some(c) => out.push_str(&crate::vm::disassemble(&def.name, &c)),
                None => out.push_str(&format!(
                    "fn {}: interpreted (uses features outside the bytecode tier)\n",
                    def.name
                )),
            }
            out.push('\n');
        }
    }
    if out.is_empty() {
        out.push_str("no top-level functions\n");
    }
    Ok(out)
}

fn load_definitions(
    src: &str,
    base_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<Interpreter, String> {
    let prog = crate::parse(src).map_err(|e| format_parse_error("<source>", src, &e))?;
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
    let mut i = new_interpreter(Rc::new(|_| {}), None, cancel);
    i.base_dirs = vec![base_dir];
    i.run(&defs)
        .map_err(|e| format!("error while loading definitions: {e}"))?;
    Ok(i)
}

/// Contract-fuzzes every function with an `ensure` or return type (like `tulpa verify`):
/// `[{name, ok, passed, discarded, counterexample:[repr…], error}]`.
pub fn verify(
    src: &str,
    base_dir: PathBuf,
    trials: usize,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let mut i = load_definitions(src, base_dir, cancel)?;
    let mut out = Vec::new();
    for (name, v) in i.global_functions() {
        let Value::Func(c) = &v else { continue };
        if c.def.ensures.is_empty() && c.def.ret.is_none() {
            continue;
        }
        let r = match i.check_fn(&v, trials) {
            Ok(r) => r,
            Err(crate::interp::Ctrl::Error(e)) if e.to_string().contains("interrupted") => {
                return Err("interrupted".into());
            }
            Err(_) => continue,
        };
        let get = |k: &str| match &r {
            Value::Dict(d) => d
                .borrow()
                .get(&Value::str(k))
                .ok()
                .flatten()
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        let cex: Vec<String> = match get("counterexample") {
            Value::List(l) => l.borrow().iter().map(|v| v.repr()).collect(),
            _ => vec![],
        };
        out.push(json!({
            "name": name,
            "ok": get("ok").truthy(),
            "passed": get("passed").as_i64().unwrap_or(0),
            "discarded": get("discarded").as_i64().unwrap_or(0),
            "counterexample": cex,
            "error": match get("error") { Value::Nil => J::Null, e => J::String(e.to_string()) },
        }));
    }
    Ok(J::Array(out).to_string())
}

/// Runs the program's `define_test!` blocks: `{passed, failed:[{name, error}]}`.
pub fn tests(src: &str, file: &str, cfg: RunConfig) -> Result<String, String> {
    let prog = crate::parse(src).map_err(|e| format_parse_error(file, src, &e))?;
    let mut i = new_interpreter(cfg.output, cfg.input, cfg.cancel);
    i.base_dirs = vec![cfg.base_dir];
    if cfg.virtual_time {
        i.virtual_time = Some(0.0);
    }
    i.run(&prog)
        .map_err(|e| format!("error while loading tests: {e}"))?;
    let (passed, failed) = i.run_tests();
    Ok(json!({
        "passed": passed,
        "failed": failed.into_iter().map(|(n, e)| json!({"name": n, "error": e})).collect::<Vec<_>>(),
    })
    .to_string())
}

/// Library version (`CARGO_PKG_VERSION`).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Native and host module names with their functions, for completion lists and docs.
pub fn modules() -> String {
    let mut m = serde_json::Map::new();
    for name in crate::builtins::module_names() {
        m.insert(
            name.to_string(),
            J::Array(
                crate::builtins::module_members(name)
                    .into_iter()
                    .map(|s| J::String(s.into()))
                    .collect(),
            ),
        );
    }
    for name in crate::host::modules() {
        m.insert(
            name.to_string(),
            J::Array(
                crate::host::names(name)
                    .into_iter()
                    .map(|s| J::String(s.into()))
                    .collect(),
            ),
        );
    }
    J::Object(m).to_string()
}
