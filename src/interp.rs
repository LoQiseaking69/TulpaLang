//! Tree-walking evaluator.

use crate::ast::*;
use crate::builtins;
use crate::devices::{Backend, SimBus};
use crate::value::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::time::Instant;

// ---------------------------------------------------------------------------- scopes

pub struct Scope {
    vars: HashMap<String, Value>,
    consts: std::collections::HashSet<String>,
    parent: Option<Env>,
}

pub type Env = Rc<RefCell<Scope>>;

pub fn new_env(parent: Option<Env>) -> Env {
    Rc::new(RefCell::new(Scope {
        vars: HashMap::new(),
        consts: Default::default(),
        parent,
    }))
}

/// All variables defined directly in `env` (not parents), sorted by name.
pub(crate) fn env_vars(env: &Env) -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = env
        .borrow()
        .vars
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

pub(crate) fn lookup(env: &Env, name: &str) -> Option<Value> {
    let s = env.borrow();
    match s.vars.get(name) {
        Some(v) => Some(v.clone()),
        None => s.parent.as_ref().and_then(|p| lookup(p, name)),
    }
}

/// Ok(true) assigned, Ok(false) undeclared, Err if the name is a constant.
pub(crate) fn assign_var(env: &Env, name: &str, v: Value) -> Result<bool, String> {
    let mut s = env.borrow_mut();
    if s.vars.contains_key(name) {
        if s.consts.contains(name) {
            return Err(format!("cannot assign to constant '{name}'"));
        }
        s.vars.insert(name.to_string(), v);
        return Ok(true);
    }
    match &s.parent {
        Some(p) => assign_var(p, name, v),
        None => Ok(false),
    }
}

fn define_const(env: &Env, name: &str, v: Value) {
    let mut s = env.borrow_mut();
    s.vars.insert(name.to_string(), v);
    s.consts.insert(name.to_string());
}

pub fn define(env: &Env, name: &str, v: Value) {
    let mut s = env.borrow_mut();
    s.consts.remove(name);
    s.vars.insert(name.to_string(), v);
}

// ---------------------------------------------------------------------------- errors

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub msg: String,
    pub line: Option<usize>,
    pub trace: Vec<String>,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.msg)?,
            None => write!(f, "{}", self.msg)?,
        }
        for t in &self.trace {
            write!(f, "\n    at {t}")?;
        }
        Ok(())
    }
}

pub enum Ctrl {
    Error(RuntimeError),
    Return(Value),
    Break,
    Continue,
}

impl From<&str> for Ctrl {
    fn from(msg: &str) -> Self {
        Ctrl::from(msg.to_string())
    }
}

impl From<String> for Ctrl {
    fn from(msg: String) -> Self {
        Ctrl::Error(RuntimeError {
            msg,
            line: None,
            trace: vec![],
        })
    }
}

pub(crate) type R<T> = Result<T, Ctrl>;

pub(crate) fn rt<T>(msg: impl Into<String>) -> R<T> {
    Err(Ctrl::from(msg.into()))
}

pub(crate) fn at_line(c: Ctrl, line: usize) -> Ctrl {
    match c {
        Ctrl::Error(mut e) if e.line.is_none() => {
            e.line = Some(line);
            Ctrl::Error(e)
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------- output

#[derive(Clone)]
pub enum Output {
    Stdout,
    Buffer(Rc<RefCell<String>>),
}

/// A user macro: parameter names and body.
type MacroDef = (Vec<String>, Vec<Stmt>);

pub struct TestCase {
    pub name: String,
    pub body: Vec<Stmt>,
    pub env: Env,
}

// ---------------------------------------------------------------------------- interpreter

pub struct Interpreter {
    pub globals: Env,
    /// Builtins live here; `globals` and every module scope are children of it.
    pub builtins_env: Env,
    pub(crate) rules: Vec<Rc<RefCell<crate::ext::Rule>>>,
    pub(crate) monitors: Vec<Rc<RefCell<crate::ext::Monitor>>>,
    pub(crate) deadlines: Vec<Rc<RefCell<crate::ext::Deadline>>>,
    /// Simulated clock in seconds (`--virtual-time` / `sim::virtual_time(true)`); None = wall clock.
    pub virtual_time: Option<f64>,
    pub(crate) reacting: bool,
    /// Directory stack used to resolve `import` paths.
    pub base_dirs: Vec<std::path::PathBuf>,
    pub(crate) modules: HashMap<std::path::PathBuf, Value>,
    pub(crate) loading: Vec<std::path::PathBuf>,
    /// Device record/replay tape (`--record` / `--replay`).
    pub tape: Option<Rc<RefCell<crate::devices::Tape>>>,
    macros: HashMap<String, Rc<MacroDef>>,
    pending: Vec<Rc<RefCell<Task>>>,
    pub tests: Vec<TestCase>,
    pub backend: Backend,
    pub sim: Rc<RefCell<SimBus>>,
    pub trace_devices: bool,
    pub output: Output,
    pub start: Instant,
    pub(crate) depth: usize,
    /// Run eligible functions on the bytecode VM (`--no-vm` / `TULPA_NO_VM=1` disables).
    pub use_vm: bool,
    pub(crate) compiled: HashMap<usize, (Rc<FnDef>, Option<Rc<crate::vm::Chunk>>)>,
}

const MAX_DEPTH: usize = 2500;

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        let builtins_env = new_env(None);
        builtins::install(&builtins_env);
        let globals = new_env(Some(builtins_env.clone()));
        Interpreter {
            globals,
            builtins_env,
            rules: Vec::new(),
            monitors: Vec::new(),
            deadlines: Vec::new(),
            virtual_time: None,
            reacting: false,
            base_dirs: vec![std::path::PathBuf::from(".")],
            modules: HashMap::new(),
            loading: Vec::new(),
            tape: None,
            macros: HashMap::new(),
            pending: Vec::new(),
            tests: Vec::new(),
            backend: Backend::Sim,
            sim: Rc::new(RefCell::new(SimBus::default())),
            trace_devices: false,
            output: Output::Stdout,
            start: Instant::now(),
            depth: 0,
            use_vm: std::env::var("TULPA_NO_VM")
                .map(|v| v.is_empty() || v == "0")
                .unwrap_or(true),
            compiled: HashMap::new(),
        }
    }

    /// Creates an interpreter whose `print` output is captured into the returned buffer.
    pub fn captured() -> (Self, Rc<RefCell<String>>) {
        let buf = Rc::new(RefCell::new(String::new()));
        let mut i = Self::new();
        i.output = Output::Buffer(buf.clone());
        (i, buf)
    }

    pub fn write_out(&self, s: &str) {
        match &self.output {
            Output::Stdout => {
                use std::io::Write;
                let mut o = std::io::stdout().lock();
                let _ = o.write_all(s.as_bytes());
                let _ = o.flush();
            }
            Output::Buffer(b) => b.borrow_mut().push_str(s),
        }
    }

    /// Runs a whole program, then drains any tasks that were spawned but never awaited.
    pub fn run(&mut self, program: &[Stmt]) -> Result<Value, RuntimeError> {
        let env = self.globals.clone();
        let result = self.run_in(program, &env);
        let drained = self.drain_tasks();
        let v = result?;
        drained?;
        Ok(v)
    }

    /// Executes statements in an existing scope (used by the REPL).
    pub fn run_in(&mut self, program: &[Stmt], env: &Env) -> Result<Value, RuntimeError> {
        self.hoist(program, env);
        match self.exec_block_in(program, env) {
            Ok(v) => Ok(v),
            Err(Ctrl::Error(e)) => Err(e),
            Err(Ctrl::Return(v)) => Ok(v),
            Err(Ctrl::Break) | Err(Ctrl::Continue) => Err(RuntimeError {
                msg: "'break' or 'continue' outside of a loop".into(),
                line: None,
                trace: vec![],
            }),
        }
    }

    pub fn drain_tasks(&mut self) -> Result<(), RuntimeError> {
        while !self.pending.is_empty() {
            let batch = std::mem::take(&mut self.pending);
            for t in batch {
                if let Err(Ctrl::Error(e)) = self.force_task(&t) {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Function declarations are visible before their definition line.
    pub(crate) fn hoist(&mut self, stmts: &[Stmt], env: &Env) {
        for s in stmts {
            match s {
                Stmt::Fn(def) => define(env, &def.name, self.closure(def, env)),
                Stmt::MacroDef(name, params, body) => {
                    self.macros
                        .insert(name.clone(), Rc::new((params.clone(), body.clone())));
                }
                _ => {}
            }
        }
    }

    pub(crate) fn closure(&self, def: &Rc<FnDef>, env: &Env) -> Value {
        Value::Func(Rc::new(Closure {
            def: def.clone(),
            env: env.clone(),
        }))
    }

    // ------------------------------------------------------------------ statements

    pub(crate) fn exec_block(&mut self, stmts: &[Stmt], parent: &Env) -> R<Value> {
        let env = new_env(Some(parent.clone()));
        self.hoist(stmts, &env);
        self.exec_block_in(stmts, &env)
    }

    pub(crate) fn exec_block_in(&mut self, stmts: &[Stmt], env: &Env) -> R<Value> {
        let mut last = Value::Nil;
        for s in stmts {
            last = self.exec(s, env)?;
        }
        Ok(last)
    }

    fn exec(&mut self, s: &Stmt, env: &Env) -> R<Value> {
        match s {
            Stmt::Expr(e, line) => self.eval(e, env).map_err(|c| at_line(c, *line)),
            Stmt::Let(names, e, is_const, line) => {
                let v = self.eval(e, env).map_err(|c| at_line(c, *line))?;
                let bind = |n: &str, x: Value| {
                    if *is_const {
                        define_const(env, n, x)
                    } else {
                        define(env, n, x)
                    }
                };
                if names.len() == 1 {
                    bind(&names[0], v);
                } else {
                    let items =
                        destructure(&v, names.len()).map_err(|m| at_line(m.into(), *line))?;
                    for (n, x) in names.iter().zip(items) {
                        bind(n, x);
                    }
                }
                Ok(Value::Nil)
            }
            Stmt::Assign(target, op, e, line) => {
                self.assign(target, op.as_deref(), e, env)
                    .map_err(|c| at_line(c, *line))?;
                if self.has_reactive() {
                    self.react().map_err(|c| at_line(c, *line))?;
                }
                Ok(Value::Nil)
            }
            Stmt::Struct(def) => {
                let t = crate::value::StructType {
                    def: def.clone(),
                    env: env.clone(),
                };
                define(env, &def.name, Value::StructType(Rc::new(t)));
                Ok(Value::Nil)
            }
            Stmt::Machine(def) => {
                let t = crate::value::MachineType {
                    def: def.clone(),
                    env: env.clone(),
                };
                define(env, &def.name, Value::MachineType(Rc::new(t)));
                Ok(Value::Nil)
            }
            Stmt::When(cond, hold, body, els, line) => self
                .register_when(cond, hold.as_ref(), body, els.as_ref(), env)
                .map_err(|c| at_line(c, *line)),
            Stmt::Deadline(trigger, within, response, handler, line) => self
                .register_deadline(trigger, within, response, handler.as_ref(), env)
                .map_err(|c| at_line(c, *line)),
            Stmt::Always(c, handler, line) => self
                .register_always(c, handler.as_ref(), env)
                .map_err(|c| at_line(c, *line)),
            Stmt::Import(path, alias, names, line) => self
                .import(path, alias.as_deref(), names, env)
                .map_err(|c| at_line(c, *line)),
            Stmt::Fn(def) => {
                define(env, &def.name, self.closure(def, env));
                Ok(Value::Nil)
            }
            Stmt::MacroDef(name, params, body) => {
                self.macros
                    .insert(name.clone(), Rc::new((params.clone(), body.clone())));
                Ok(Value::Nil)
            }
            Stmt::Return(e, line) => {
                let v = match e {
                    Some(e) => self.eval(e, env).map_err(|c| at_line(c, *line))?,
                    None => Value::Nil,
                };
                Err(Ctrl::Return(v))
            }
            Stmt::If(c, a, b) => {
                if self.eval(c, env)?.truthy() {
                    self.exec_block(a, env)
                } else if let Some(b) = b {
                    self.exec_block(b, env)
                } else {
                    Ok(Value::Nil)
                }
            }
            Stmt::While(c, invariants, body, line) => {
                loop {
                    self.check_invariants(invariants, env, *line)?;
                    if !self.eval(c, env).map_err(|x| at_line(x, *line))?.truthy() {
                        break;
                    }
                    match self.exec_block(body, env) {
                        Ok(_) | Err(Ctrl::Continue) => {}
                        Err(Ctrl::Break) => return Ok(Value::Nil),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Nil)
            }
            Stmt::Loop(body) => loop {
                match self.exec_block(body, env) {
                    Ok(_) | Err(Ctrl::Continue) => {}
                    Err(Ctrl::Break) => return Ok(Value::Nil),
                    Err(e) => return Err(e),
                }
            },
            Stmt::For(names, it, body, line) => {
                let iterable = self.eval(it, env).map_err(|c| at_line(c, *line))?;
                let items = self
                    .iterate(&iterable, names.len())
                    .map_err(|c| at_line(c, *line))?;
                for item in items {
                    let scope = new_env(Some(env.clone()));
                    if names.len() == 1 {
                        define(&scope, &names[0], item);
                    } else {
                        let parts = destructure(&item, names.len())
                            .map_err(|m| at_line(m.into(), *line))?;
                        for (n, x) in names.iter().zip(parts) {
                            define(&scope, n, x);
                        }
                    }
                    self.hoist(body, &scope);
                    match self.exec_block_in(body, &scope) {
                        Ok(_) | Err(Ctrl::Continue) => {}
                        Err(Ctrl::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Nil)
            }
            Stmt::Break => Err(Ctrl::Break),
            Stmt::Continue => Err(Ctrl::Continue),
            Stmt::Use(path) => {
                if !builtins::module_exists(&path[0]) {
                    return rt(format!("unknown module '{}'", path.join("::")));
                }
                Ok(Value::Nil)
            }
            Stmt::Assert(e, msg, text, line) => {
                if !self.eval(e, env).map_err(|c| at_line(c, *line))?.truthy() {
                    let extra = match msg {
                        Some(m) => format!(": {}", self.eval(m, env)?),
                        None => String::new(),
                    };
                    return Err(at_line(
                        format!("assertion failed: {text}{extra}").into(),
                        *line,
                    ));
                }
                Ok(Value::Nil)
            }
            Stmt::Try(body, name, handler) => match self.exec_block(body, env) {
                Err(Ctrl::Error(e)) => {
                    let scope = new_env(Some(env.clone()));
                    if let Some(n) = name {
                        define(&scope, n, Value::str(e.msg.as_str()));
                    }
                    self.exec_block(handler, &scope)
                }
                other => other,
            },
        }
    }

    fn check_invariants(&mut self, invs: &[Contract], env: &Env, line: usize) -> R<()> {
        for inv in invs {
            if !self
                .eval(&inv.expr, env)
                .map_err(|c| at_line(c, line))?
                .truthy()
            {
                return Err(at_line(
                    format!("loop invariant violated: {}", inv.text).into(),
                    line,
                ));
            }
        }
        Ok(())
    }

    fn assign(&mut self, target: &Target, op: Option<&str>, e: &Expr, env: &Env) -> R<()> {
        let rhs = self.eval(e, env)?;
        match target {
            Target::Name(n) => {
                let v = match op {
                    Some(op) => {
                        let cur =
                            lookup(env, n).ok_or_else(|| format!("undefined variable '{n}'"))?;
                        self.binary_op(&cur, op, &rhs)?
                    }
                    None => rhs,
                };
                if !assign_var(env, n, v)? {
                    return rt(format!(
                        "assignment to undeclared variable '{n}' (use 'let {n} = ...')"
                    ));
                }
            }
            Target::Index(obj, idx) => {
                let o = self.eval(obj, env)?;
                let i = self.eval(idx, env)?;
                let v = match op {
                    Some(op) => self.binary_op(&index(&o, &i)?, op, &rhs)?,
                    None => rhs,
                };
                set_index(&o, &i, v)?;
            }
            Target::Field(obj, f) => {
                let o = self.eval(obj, env)?;
                let v = match op {
                    Some(op) => {
                        let cur = self.get_field(&o, f)?;
                        self.binary_op(&cur, op, &rhs)?
                    }
                    None => rhs,
                };
                self.set_field(&o, f, v)?;
            }
        }
        Ok(())
    }

    pub fn iterate(&mut self, v: &Value, arity: usize) -> R<Vec<Value>> {
        Ok(match v {
            Value::Range(a, b, incl) => {
                let end = if *incl { *b + 1 } else { *b };
                (*a..end).map(Value::Int).collect()
            }
            Value::List(l) => l.borrow().clone(),
            Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
            Value::Dict(d) => d
                .borrow()
                .entries
                .iter()
                .map(|(k, v)| {
                    if arity > 1 {
                        Value::list(vec![k.clone(), v.clone()])
                    } else {
                        k.clone()
                    }
                })
                .collect(),
            other => return rt(format!("cannot iterate over {}", other.type_name())),
        })
    }

    // ------------------------------------------------------------------ expressions

    pub fn eval(&mut self, e: &Expr, env: &Env) -> R<Value> {
        match e {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(n) => Ok(Value::Float(*n)),
            Expr::Str(s) => Ok(Value::str(s.as_str())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::FStr(parts) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        FPart::Lit(l) => s.push_str(l),
                        FPart::Expr(e) => {
                            let v = self.eval(e, env)?;
                            s.push_str(&self.stringify(&v)?)
                        }
                    }
                }
                Ok(Value::str(s))
            }
            Expr::Ident(name, line) => match lookup(env, name) {
                Some(Value::Thunk(t)) => self.force_thunk(&t),
                Some(v) => Ok(v),
                None => Err(at_line(
                    format!("undefined variable '{name}'").into(),
                    *line,
                )),
            },
            Expr::Path(m, name, line) => builtins::path(m, name)
                .ok_or_else(|| at_line(format!("unknown item '{m}::{name}'").into(), *line)),
            Expr::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.eval(i, env)?);
                }
                Ok(Value::list(out))
            }
            Expr::Repeat(v, n) => {
                let v = self.eval(v, env)?;
                let n = self
                    .eval(n, env)?
                    .as_i64()
                    .ok_or("repeat count must be an integer")?;
                Ok(Value::list(vec![v; n.max(0) as usize]))
            }
            Expr::Dict(pairs) => {
                let mut d = Dict::default();
                for (k, v) in pairs {
                    let k = self.eval(k, env)?;
                    let v = self.eval(v, env)?;
                    d.set(k, v)?;
                }
                Ok(Value::dict(d))
            }
            Expr::Unary(op, x, line) => {
                let v = self.eval(x, env)?;
                unary(op, &v).map_err(|m| at_line(m.into(), *line))
            }
            Expr::Binary(l, op, r, line) => {
                let a = self.eval(l, env)?;
                let b = self.eval(r, env)?;
                self.binary_op(&a, op, &b).map_err(|c| at_line(c, *line))
            }
            Expr::Quantity(v, u) => Ok(Value::Quantity(crate::units::Quantity::new(*v, u.clone()))),
            Expr::Named(n, _) => rt(format!(
                "keyword argument '{n}:' is only allowed inside a call"
            )),
            Expr::Logic(l, is_and, r) => {
                let a = self.eval(l, env)?;
                if a.truthy() != *is_and {
                    Ok(a)
                } else {
                    self.eval(r, env)
                }
            }
            Expr::Range(a, b, incl) => {
                let a = self
                    .eval(a, env)?
                    .as_i64()
                    .ok_or("range bounds must be integers")?;
                let b = self
                    .eval(b, env)?
                    .as_i64()
                    .ok_or("range bounds must be integers")?;
                Ok(Value::Range(a, b, *incl))
            }
            Expr::Call(f, args, line) => {
                let fv = self.eval(f, env)?;
                let (av, kw) = self.eval_args(args, env)?;
                self.call_kw(&fv, av, kw).map_err(|c| at_line(c, *line))
            }
            Expr::MethodCall(obj, name, args, line) => {
                let o = self.eval(obj, env)?;
                let (av, kw) = self.eval_args(args, env)?;
                if !kw.is_empty() {
                    return self
                        .call_method_kw(&o, name, av, kw)
                        .map_err(|c| at_line(c, *line));
                }
                self.call_method(&o, name, av)
                    .map_err(|c| at_line(c, *line))
            }
            Expr::Index(o, i, line) => {
                let o = self.eval(o, env)?;
                let i = self.eval(i, env)?;
                index(&o, &i).map_err(|m| at_line(m.into(), *line))
            }
            Expr::Field(o, f, line) => {
                let o = self.eval(o, env)?;
                self.get_field(&o, f).map_err(|c| at_line(c, *line))
            }
            Expr::Lambda(def) => Ok(self.closure(def, env)),
            Expr::MacroCall(name, args, line) => self
                .macro_call(name, args, env)
                .map_err(|c| at_line(c, *line)),
            Expr::IfExpr(c, a, b) => {
                if self.eval(c, env)?.truthy() {
                    self.exec_block(a, env)
                } else if let Some(b) = b {
                    self.exec_block(b, env)
                } else {
                    Ok(Value::Nil)
                }
            }
            Expr::Match(subject, arms, line) => {
                let v = self.eval(subject, env)?;
                for arm in arms {
                    for p in &arm.patterns {
                        let scope = new_env(Some(env.clone()));
                        if self.matches(p, &v, &scope)? {
                            if let Some(g) = &arm.guard {
                                if !self.eval(g, &scope)?.truthy() {
                                    continue;
                                }
                            }
                            return self.exec_block_in(&arm.body, &scope);
                        }
                    }
                }
                Err(at_line(
                    format!("no match arm matched value {}", v.repr()).into(),
                    *line,
                ))
            }
            Expr::Block(stmts) => self.exec_block(stmts, env),
            Expr::Spawn(x, line) => {
                let (f, args) = match &**x {
                    Expr::Call(f, args, _) => {
                        let f = self.eval(f, env)?;
                        let mut av = Vec::new();
                        for a in args {
                            av.push(self.eval(a, env)?);
                        }
                        (f, av)
                    }
                    other => (self.eval(other, env)?, vec![]),
                };
                if !matches!(f, Value::Func(_) | Value::Builtin(..)) {
                    return Err(at_line(
                        format!("cannot spawn a {}", f.type_name()).into(),
                        *line,
                    ));
                }
                let t = Rc::new(RefCell::new(Task::Pending(f, args)));
                self.pending.push(t.clone());
                Ok(Value::Task(t))
            }
            Expr::Await(x, line) => {
                let v = self.eval(x, env)?;
                self.await_value(v).map_err(|c| at_line(c, *line))
            }
        }
    }

    pub fn await_value(&mut self, v: Value) -> R<Value> {
        match v {
            Value::Task(t) => self.force_task(&t),
            Value::List(l) => {
                let items = l.borrow().clone();
                let mut out = Vec::new();
                for i in items {
                    out.push(self.await_value(i)?);
                }
                Ok(Value::list(out))
            }
            other => Ok(other),
        }
    }

    fn force_task(&mut self, t: &Rc<RefCell<Task>>) -> R<Value> {
        let state = std::mem::replace(&mut *t.borrow_mut(), Task::Running);
        match state {
            Task::Pending(f, args) => {
                let r = self.call(&f, args);
                *t.borrow_mut() = match &r {
                    Ok(v) => Task::Done(v.clone()),
                    Err(Ctrl::Error(e)) => Task::Failed(e.msg.clone()),
                    Err(_) => Task::Done(Value::Nil),
                };
                r
            }
            Task::Done(v) => {
                *t.borrow_mut() = Task::Done(v.clone());
                Ok(v)
            }
            Task::Failed(m) => {
                *t.borrow_mut() = Task::Failed(m.clone());
                rt(format!("awaited task failed: {m}"))
            }
            Task::Running => rt("task awaited itself (cycle)"),
        }
    }

    pub(crate) fn force_thunk(&mut self, t: &Rc<(MacroArg, Env)>) -> R<Value> {
        let (arg, env) = &**t;
        match arg {
            MacroArg::Expr(e) => self.eval(e, env),
            MacroArg::Block(b) => self.exec_block(b, env),
        }
    }

    fn matches(&mut self, p: &Pattern, v: &Value, scope: &Env) -> R<bool> {
        Ok(match p {
            Pattern::Wildcard => true,
            Pattern::Bind(n) => {
                define(scope, n, v.clone());
                true
            }
            Pattern::Literal(e) => self.eval(e, scope)?.equals(v),
            Pattern::Range(a, b, incl) => {
                let (Some(x), Some(lo), Some(hi)) = (
                    v.as_f64(),
                    self.eval(a, scope)?.as_f64(),
                    self.eval(b, scope)?.as_f64(),
                ) else {
                    return Ok(false);
                };
                x >= lo && if *incl { x <= hi } else { x < hi }
            }
            Pattern::List(items, rest) => {
                let Value::List(l) = v else { return Ok(false) };
                let l = l.borrow().clone();
                if (rest.is_none() && l.len() != items.len()) || l.len() < items.len() {
                    return Ok(false);
                }
                for (p, x) in items.iter().zip(l.iter()) {
                    if !self.matches(p, x, scope)? {
                        return Ok(false);
                    }
                }
                if let Some(r) = rest {
                    if r != "_" {
                        define(scope, r, Value::list(l[items.len()..].to_vec()));
                    }
                }
                true
            }
        })
    }

    // ------------------------------------------------------------------ calls

    pub fn call(&mut self, f: &Value, args: Vec<Value>) -> R<Value> {
        match f {
            Value::Builtin(_, func) => func(self, args).map_err(Ctrl::from),
            Value::Func(c) => self.call_closure(c, args),
            _ => self.call_kw(f, args, vec![]),
        }
    }

    pub(crate) fn call_closure(&mut self, c: &Rc<Closure>, args: Vec<Value>) -> R<Value> {
        self.call_closure_kw(c, args, vec![])
    }

    pub(crate) fn call_closure_kw(
        &mut self,
        c: &Rc<Closure>,
        mut args: Vec<Value>,
        kw: Vec<(String, Value)>,
    ) -> R<Value> {
        let def = &c.def;
        // Fast path: compiled body, exact positional arity, no contracts. Locals live in VM
        // slots, so no scope object is allocated for the call.
        if kw.is_empty()
            && args.len() == def.params.len()
            && def.requires.is_empty()
            && def.ensures.is_empty()
            && self.depth < MAX_DEPTH
        {
            if let Some(chunk) = self.chunk_for(def) {
                for (p, v) in def.params.iter().zip(args.iter()) {
                    if let Some(ty) = &p.ty {
                        if !self.type_matches(ty, v) {
                            self.check_type(
                                ty,
                                v,
                                &format!("{}() parameter '{}'", def.name, p.name),
                            )?;
                        }
                    }
                }
                self.depth += 1;
                let r = self.run_chunk(&chunk, &c.env, args);
                self.depth -= 1;
                let value = match r {
                    Ok((v, _)) => v,
                    Err(Ctrl::Error(mut e)) => {
                        if e.trace.len() < 12 {
                            let at = e.line.map(|l| format!(" (line {l})")).unwrap_or_default();
                            e.trace.push(format!("{}(){at}", def.name));
                        }
                        return Err(Ctrl::Error(e));
                    }
                    Err(other) => return Err(other),
                };
                if let Some(ty) = &def.ret {
                    if !self.type_matches(ty, &value) {
                        self.check_type(ty, &value, &format!("{}() return value", def.name))?;
                    }
                }
                return Ok(value);
            }
        }
        let required = def.params.iter().filter(|p| p.default.is_none()).count();
        let kw_used = !kw.is_empty();
        if kw_used {
            // Gaps left by keyword arguments are filled with `Value::Thunk` placeholders.
            args = self.merge_kwargs(&def.name, &def.params, args, kw)?;
        }
        if (!kw_used && args.len() < required) || args.len() > def.params.len() {
            return rt(format!(
                "{}() takes {} argument{}, got {}",
                def.name,
                if required == def.params.len() {
                    required.to_string()
                } else {
                    format!("{required}..{}", def.params.len())
                },
                if def.params.len() == 1 { "" } else { "s" },
                args.len()
            ));
        }
        if self.depth >= MAX_DEPTH {
            return rt(format!("maximum recursion depth ({MAX_DEPTH}) exceeded"));
        }
        let env = new_env(Some(c.env.clone()));
        let n_given = args.len();
        args.reverse();
        for (i, p) in def.params.iter().enumerate() {
            let given = if i < n_given { args.pop() } else { None };
            let v = match given {
                Some(Value::Thunk(_)) | None => match &p.default {
                    Some(d) => self.eval(d, &env)?,
                    None => return rt(format!("{}() missing argument '{}'", def.name, p.name)),
                },
                Some(v) => v,
            };
            if let Some(ty) = &p.ty {
                self.check_type(ty, &v, &format!("{}() parameter '{}'", def.name, p.name))?;
            }
            define(&env, &p.name, v);
        }
        let pre: Vec<(String, Value)> = if def.ensures.is_empty() {
            vec![]
        } else {
            def.params
                .iter()
                .map(|p| (p.name.clone(), lookup(&env, &p.name).unwrap()))
                .collect()
        };
        for r in &def.requires {
            if !self.eval(&r.expr, &env)?.truthy() {
                return rt(format!(
                    "precondition violated in {}(): require {}",
                    def.name, r.text
                ));
            }
        }

        self.depth += 1;
        let result = match self.chunk_for(def) {
            Some(chunk) => {
                let locals: Vec<Value> = def
                    .params
                    .iter()
                    .map(|p| lookup(&env, &p.name).unwrap_or(Value::Nil))
                    .collect();
                match self.run_chunk(&chunk, &env, locals) {
                    Ok((v, finals)) => {
                        if !def.ensures.is_empty() {
                            // expose final parameter values for `x'` in postconditions
                            for (p, v) in def.params.iter().zip(finals) {
                                define(&env, &p.name, v);
                            }
                        }
                        Ok(v)
                    }
                    Err(e) => Err(e),
                }
            }
            None => {
                self.hoist(&def.body, &env);
                self.exec_block_in(&def.body, &env)
            }
        };
        self.depth -= 1;

        let value = match result {
            Ok(v) => v, // last expression is the implicit return value
            Err(Ctrl::Return(v)) => v,
            Err(Ctrl::Error(mut e)) => {
                if e.trace.len() < 12 {
                    let at = e.line.map(|l| format!(" (line {l})")).unwrap_or_default();
                    e.trace.push(format!("{}(){at}", def.name));
                }
                // Keep the innermost line as the error location.
                return Err(Ctrl::Error(e));
            }
            Err(Ctrl::Break) | Err(Ctrl::Continue) => {
                return rt("'break' or 'continue' outside of a loop");
            }
        };

        if let Some(ty) = &def.ret {
            self.check_type(ty, &value, &format!("{}() return value", def.name))?;
        }
        if !def.ensures.is_empty() {
            // Hehner-style post-state: `x` = initial value, `x'` = final value, `result'` = return value.
            let post = new_env(Some(c.env.clone()));
            for (name, v) in pre {
                let final_v = lookup(&env, &name).unwrap_or(Value::Nil);
                define(&post, &name, v);
                define(&post, &format!("{name}'"), final_v);
            }
            define(&post, "result'", value.clone());
            for en in &def.ensures {
                if !self.eval(&en.expr, &post)?.truthy() {
                    return rt(format!(
                        "postcondition violated in {}(): ensure {} (result' = {})",
                        def.name,
                        en.text,
                        value.repr()
                    ));
                }
            }
        }
        Ok(value)
    }

    pub fn call_method(&mut self, o: &Value, name: &str, args: Vec<Value>) -> R<Value> {
        match o {
            Value::Native(n) => {
                let r = n.borrow_mut().call_method(name, &args);
                if self.trace_devices {
                    let desc = n.borrow().describe();
                    let a: Vec<String> = args.iter().map(|a| a.repr()).collect();
                    match &r {
                        Ok(v) => {
                            eprintln!("[trace] {desc}.{name}({}) -> {}", a.join(", "), v.repr())
                        }
                        Err(e) => eprintln!("[trace] {desc}.{name}({}) !! {e}", a.join(", ")),
                    }
                }
                let v = r.map_err(Ctrl::from)?;
                if self.has_reactive() {
                    self.react()?;
                }
                Ok(v)
            }
            Value::Object(_)
            | Value::Machine(_)
            | Value::Signal(_)
            | Value::StructType(_)
            | Value::MachineType(_)
            | Value::Quantity(_) => self.call_method_kw(o, name, args, vec![]),
            Value::Dict(d) => {
                let member = d.borrow().get(&Value::str(name))?;
                match member {
                    Some(f @ (Value::Func(_) | Value::Builtin(..))) => self.call(&f, args),
                    _ => builtins::method(self, o, name, args).map_err(Ctrl::from),
                }
            }
            _ => builtins::method(self, o, name, args).map_err(Ctrl::from),
        }
    }

    // ------------------------------------------------------------------ macros

    fn macro_arg(&mut self, a: &MacroArg, env: &Env) -> R<Value> {
        match a {
            MacroArg::Expr(e) => self.eval(e, env),
            MacroArg::Block(b) => self.exec_block(b, env),
        }
    }

    fn macro_call(&mut self, name: &str, args: &[MacroArg], env: &Env) -> R<Value> {
        let argc = |n: usize| -> R<()> {
            if args.len() != n {
                rt(format!(
                    "{name}! expects {n} argument(s), got {}",
                    args.len()
                ))
            } else {
                Ok(())
            }
        };
        match name {
            "simple_loop" | "repeat" => {
                argc(2)?;
                let n = self
                    .macro_arg(&args[0], env)?
                    .as_i64()
                    .ok_or(format!("{name}! count must be an integer"))?;
                for _ in 0..n.max(0) {
                    match self.macro_arg(&args[1], env) {
                        Ok(_) | Err(Ctrl::Continue) => {}
                        Err(Ctrl::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Nil)
            }
            "every" => {
                // every!(period_ms, { ... })  or  every!(period_ms, count, { ... })
                if args.len() != 2 && args.len() != 3 {
                    return rt("every! expects (period_ms, body) or (period_ms, count, body)");
                }
                let pv = self.macro_arg(&args[0], env)?;
                let period = crate::ext::millis_of(&pv)
                    .ok_or("every! period must be milliseconds or a time quantity like 20[ms]")?;
                let count = if args.len() == 3 {
                    Some(
                        self.macro_arg(&args[1], env)?
                            .as_i64()
                            .ok_or("every! count must be an integer")?,
                    )
                } else {
                    None
                };
                let body = args.last().unwrap();
                let dt = period.max(0.0) / 1000.0;
                let mut next = self.now();
                let mut i = 0i64;
                while count.is_none_or(|c| i < c) {
                    match self.macro_arg(body, env) {
                        Ok(_) | Err(Ctrl::Continue) => {}
                        Err(Ctrl::Break) => break,
                        Err(e) => return Err(e),
                    }
                    i += 1;
                    if self.has_reactive() {
                        self.react()?;
                    }
                    next += dt;
                    let now = self.now();
                    if next > now {
                        self.sleep_secs(next - now)?;
                    } else {
                        next = now; // overran the period; don't try to catch up
                    }
                }
                Ok(Value::Int(i))
            }
            "time_execution" => {
                argc(1)?;
                let t = Instant::now();
                let v = self.macro_arg(&args[0], env)?;
                self.write_out(&format!("Execution time: {:?}\n", t.elapsed()));
                Ok(v)
            }
            "log_message" => {
                argc(1)?;
                let m = self.macro_arg(&args[0], env)?;
                self.write_out(&format!("[{}] {}\n", builtins::timestamp(), m));
                Ok(Value::Nil)
            }
            "println" | "print" | "format" => {
                let mut vals = Vec::new();
                for a in args {
                    vals.push(self.macro_arg(a, env)?);
                }
                let s = if vals.is_empty() {
                    String::new()
                } else {
                    let fmt = match &vals[0] {
                        Value::Str(s) => s.to_string(),
                        other => {
                            return rt(format!(
                                "{name}! first argument must be a format string, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    builtins::rust_format(&fmt, &vals[1..])?
                };
                match name {
                    "format" => Ok(Value::str(s)),
                    "println" => {
                        self.write_out(&format!("{s}\n"));
                        Ok(Value::Nil)
                    }
                    _ => {
                        self.write_out(&s);
                        Ok(Value::Nil)
                    }
                }
            }
            "vec" => {
                let mut vals = Vec::new();
                for a in args {
                    vals.push(self.macro_arg(a, env)?);
                }
                Ok(Value::list(vals))
            }
            "assert_eq" => {
                if args.len() < 2 {
                    return rt("assert_eq! expects two values");
                }
                let a = self.macro_arg(&args[0], env)?;
                let b = self.macro_arg(&args[1], env)?;
                if !a.equals(&b) {
                    return rt(format!("assertion failed: {} != {}", a.repr(), b.repr()));
                }
                Ok(Value::Nil)
            }
            "assert" => {
                argc(1)?;
                if !self.macro_arg(&args[0], env)?.truthy() {
                    return rt("assertion failed");
                }
                Ok(Value::Nil)
            }
            "dbg" => {
                argc(1)?;
                let v = self.macro_arg(&args[0], env)?;
                eprintln!("[dbg] {}", v.repr());
                Ok(v)
            }
            "unwrap_or_default" => {
                argc(2)?;
                match self.macro_arg(&args[0], env) {
                    Ok(Value::Nil) | Err(Ctrl::Error(_)) => self.macro_arg(&args[1], env),
                    other => other,
                }
            }
            "define_test" => {
                argc(2)?;
                let name = match &args[0] {
                    MacroArg::Expr(Expr::Ident(n, _)) => n.clone(),
                    a => self.macro_arg(a, env)?.to_string(),
                };
                let body = match &args[1] {
                    MacroArg::Block(b) => b.clone(),
                    _ => return rt("define_test! body must be a { block }"),
                };
                self.tests.push(TestCase {
                    name,
                    body,
                    env: env.clone(),
                });
                Ok(Value::Nil)
            }
            _ => {
                let Some(m) = self.macros.get(name).cloned() else {
                    return rt(format!("unknown macro '{name}!'"));
                };
                let (params, body) = &*m;
                if params.len() != args.len() {
                    return rt(format!(
                        "{name}! expects {} argument(s), got {}",
                        params.len(),
                        args.len()
                    ));
                }
                let scope = new_env(Some(env.clone()));
                for (p, a) in params.iter().zip(args) {
                    define(&scope, p, Value::Thunk(Rc::new((a.clone(), env.clone()))));
                }
                self.hoist(body, &scope);
                self.exec_block_in(body, &scope)
            }
        }
    }

    /// User-defined global values, sorted by name.
    pub fn global_values(&self) -> Vec<(String, Value)> {
        env_vars(&self.globals)
    }

    /// User-defined global functions, sorted by name.
    pub fn global_functions(&self) -> Vec<(String, Value)> {
        self.global_values()
            .into_iter()
            .filter(|(_, v)| matches!(v, Value::Func(_)))
            .collect()
    }

    /// Runs every `define_test!` registered by the program. Returns (passed, failed).
    pub fn run_tests(&mut self) -> (usize, Vec<(String, String)>) {
        let tests = std::mem::take(&mut self.tests);
        let mut passed = 0;
        let mut failed = Vec::new();
        for t in &tests {
            match self.exec_block(&t.body, &t.env) {
                Ok(_) | Err(Ctrl::Return(_)) => passed += 1,
                Err(Ctrl::Error(e)) => failed.push((t.name.clone(), e.to_string())),
                Err(_) => failed.push((t.name.clone(), "break/continue outside loop".into())),
            }
        }
        (passed, failed)
    }
}

// ---------------------------------------------------------------------------- operators

fn destructure(v: &Value, n: usize) -> Result<Vec<Value>, String> {
    match v {
        Value::List(l) if l.borrow().len() == n => Ok(l.borrow().clone()),
        Value::List(l) => Err(format!(
            "cannot unpack {} values into {n} names",
            l.borrow().len()
        )),
        other => Err(format!(
            "cannot unpack {} into {n} names",
            other.type_name()
        )),
    }
}

pub fn unary(op: &str, v: &Value) -> Result<Value, String> {
    match (op, v) {
        ("-", Value::Int(i)) => i
            .checked_neg()
            .map(Value::Int)
            .ok_or_else(|| "integer overflow".into()),
        ("-", Value::Float(f)) => Ok(Value::Float(-f)),
        ("-", Value::Quantity(q)) => Ok(Value::Quantity(crate::units::Quantity {
            si: -q.si,
            ..q.clone()
        })),
        ("!", v) => Ok(Value::Bool(!v.truthy())),
        ("~", Value::Int(i)) => Ok(Value::Int(!i)),
        _ => Err(format!(
            "bad operand type for unary {op}: {}",
            v.type_name()
        )),
    }
}

pub fn binary(a: &Value, op: &str, b: &Value) -> Result<Value, String> {
    use Value::*;
    if matches!(a, Quantity(_)) || matches!(b, Quantity(_)) {
        return crate::ext::quantity_binary(a, op, b);
    }
    let overflow = || format!("integer overflow in {a} {op} {b}");
    Ok(match (op, a, b) {
        ("==", _, _) => Bool(a.equals(b)),
        ("!=", _, _) => Bool(!a.equals(b)),
        ("+", Int(x), Int(y)) => Int(x.checked_add(*y).ok_or_else(overflow)?),
        ("-", Int(x), Int(y)) => Int(x.checked_sub(*y).ok_or_else(overflow)?),
        ("*", Int(x), Int(y)) => Int(x.checked_mul(*y).ok_or_else(overflow)?),
        ("/", Int(_), Int(0)) | ("%", Int(_), Int(0)) => return Err("division by zero".into()),
        ("/", Int(x), Int(y)) => Int(x.div_euclid(*y)),
        ("%", Int(x), Int(y)) => Int(x.rem_euclid(*y)),
        ("**", Int(x), Int(y)) if *y >= 0 => Int(x
            .checked_pow(u32::try_from(*y).map_err(|_| overflow())?)
            .ok_or_else(overflow)?),
        ("&", Int(x), Int(y)) => Int(x & y),
        ("|", Int(x), Int(y)) => Int(x | y),
        ("^", Int(x), Int(y)) => Int(x ^ y),
        ("<<", Int(x), Int(y)) => Int(x
            .checked_shl(u32::try_from(*y).map_err(|_| overflow())?)
            .ok_or_else(overflow)?),
        (">>", Int(x), Int(y)) => Int(x
            .checked_shr(u32::try_from(*y).map_err(|_| overflow())?)
            .ok_or_else(overflow)?),
        ("+" | "-" | "*" | "/" | "%" | "**", Int(_) | Float(_), Int(_) | Float(_)) => {
            let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            Float(match op {
                "+" => x + y,
                "-" => x - y,
                "*" => x * y,
                "/" => {
                    if y == 0.0 {
                        return Err("division by zero".into());
                    }
                    x / y
                }
                "%" => x.rem_euclid(y),
                _ => x.powf(y),
            })
        }
        ("+", Str(x), y) => Value::str(format!("{x}{y}")),
        ("+", x, Str(y)) => Value::str(format!("{x}{y}")),
        ("+", List(x), List(y)) => {
            let mut v = x.borrow().clone();
            v.extend(y.borrow().iter().cloned());
            Value::list(v)
        }
        ("*", Str(s), Int(n)) | ("*", Int(n), Str(s)) => Value::str(s.repeat((*n).max(0) as usize)),
        ("*", List(l), Int(n)) | ("*", Int(n), List(l)) => {
            let src = l.borrow();
            let mut v = Vec::new();
            for _ in 0..(*n).max(0) {
                v.extend(src.iter().cloned());
            }
            Value::list(v)
        }
        ("<" | "<=" | ">" | ">=", _, _) => {
            let ord = compare(a, b)?;
            Bool(match op {
                "<" => ord.is_lt(),
                "<=" => ord.is_le(),
                ">" => ord.is_gt(),
                _ => ord.is_ge(),
            })
        }
        ("in", x, coll) => Bool(contains(coll, x)?),
        ("not in", x, coll) => Bool(!contains(coll, x)?),
        _ => {
            return Err(format!(
                "unsupported operand types for {op}: {} and {}",
                a.type_name(),
                b.type_name()
            ))
        }
    })
}

pub fn compare(a: &Value, b: &Value) -> Result<std::cmp::Ordering, String> {
    use std::cmp::Ordering;
    if matches!(a, Value::Quantity(_)) || matches!(b, Value::Quantity(_)) {
        return crate::ext::quantity_compare(a, b);
    }
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        (Value::List(x), Value::List(y)) => {
            let (x, y) = (x.borrow(), y.borrow());
            for (p, q) in x.iter().zip(y.iter()) {
                let o = compare(p, q)?;
                if o != Ordering::Equal {
                    return Ok(o);
                }
            }
            Ok(x.len().cmp(&y.len()))
        }
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).ok_or_else(|| "cannot compare NaN".into()),
            _ => Err(format!(
                "cannot compare {} with {}",
                a.type_name(),
                b.type_name()
            )),
        },
    }
}

fn contains(coll: &Value, x: &Value) -> Result<bool, String> {
    Ok(match coll {
        Value::List(l) => l.borrow().iter().any(|v| v.equals(x)),
        Value::Str(s) => match x {
            Value::Str(sub) => s.contains(sub.as_ref()),
            _ => return Err("'in <str>' requires a string on the left".into()),
        },
        Value::Dict(d) => d.borrow().get(x)?.is_some(),
        Value::Range(a, b, incl) => match x.as_i64() {
            Some(i) => i >= *a && if *incl { i <= *b } else { i < *b },
            None => false,
        },
        other => return Err(format!("'in' not supported for {}", other.type_name())),
    })
}

fn norm_index(i: &Value, len: usize) -> Result<usize, String> {
    let i = i
        .as_i64()
        .ok_or_else(|| format!("index must be an integer, got {}", i.type_name()))?;
    let idx = if i < 0 { len as i64 + i } else { i };
    if idx < 0 || idx as usize >= len {
        return Err(format!("index {i} out of range for length {len}"));
    }
    Ok(idx as usize)
}

fn slice_bounds(a: i64, b: i64, incl: bool, len: usize) -> (usize, usize) {
    let fix = |x: i64| -> usize {
        let x = if x < 0 { len as i64 + x } else { x };
        x.clamp(0, len as i64) as usize
    };
    let start = fix(a);
    let end = fix(if incl { b + 1 } else { b });
    (start, end.max(start))
}

pub fn index(o: &Value, i: &Value) -> Result<Value, String> {
    match (o, i) {
        (Value::List(l), Value::Range(a, b, incl)) => {
            let l = l.borrow();
            let (s, e) = slice_bounds(*a, *b, *incl, l.len());
            Ok(Value::list(l[s..e].to_vec()))
        }
        (Value::Str(st), Value::Range(a, b, incl)) => {
            let chars: Vec<char> = st.chars().collect();
            let (s, e) = slice_bounds(*a, *b, *incl, chars.len());
            Ok(Value::str(chars[s..e].iter().collect::<String>()))
        }
        (Value::List(l), _) => {
            let l = l.borrow();
            Ok(l[norm_index(i, l.len())?].clone())
        }
        (Value::Str(s), _) => {
            let chars: Vec<char> = s.chars().collect();
            Ok(Value::str(chars[norm_index(i, chars.len())?].to_string()))
        }
        (Value::Dict(d), _) => d
            .borrow()
            .get(i)?
            .ok_or_else(|| format!("key {} not found", i.repr())),
        (Value::Range(a, b, incl), _) => {
            let len = if *incl { b - a + 1 } else { b - a }.max(0) as usize;
            Ok(Value::Int(a + norm_index(i, len)? as i64))
        }
        _ => Err(format!("{} is not indexable", o.type_name())),
    }
}

pub(crate) fn set_index(o: &Value, i: &Value, v: Value) -> Result<(), String> {
    match o {
        Value::List(l) => {
            let mut l = l.borrow_mut();
            let idx = norm_index(i, l.len())?;
            l[idx] = v;
            Ok(())
        }
        Value::Dict(d) => d.borrow_mut().set(i.clone(), v),
        _ => Err(format!(
            "{} does not support item assignment",
            o.type_name()
        )),
    }
}

pub(crate) fn field(o: &Value, f: &str) -> Result<Value, String> {
    match o {
        Value::Dict(d) => d
            .borrow()
            .get(&Value::str(f))?
            .ok_or_else(|| format!("dict has no field '{f}'")),
        Value::List(l) if f.chars().all(|c| c.is_ascii_digit()) => {
            index(&Value::List(l.clone()), &Value::Int(f.parse().unwrap()))
        }
        _ => Err(format!(
            "{} has no field '{f}' (did you mean .{f}()?)",
            o.type_name()
        )),
    }
}
