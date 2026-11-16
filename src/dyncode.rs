//! The dynamic tier for firmware: serialises a whole Tulpa program (with every module it
//! imports) into a compact tree, "T-code", that the C runtime in `runtime/dyn/` executes.
//!
//! The static native compiler (`llvm.rs`) covers typed code. Programs that use dicts,
//! closures, `try`, tasks, signals or the dynamic libraries (`std/plan`, `std/link`, ...) are
//! built this way instead, so every Tulpa program can be deployed to a microcontroller: the
//! firmware links `tulpa_dyn.c` (the Tulpa VM, in portable C) and carries the program as a
//! constant byte array.
//!
//! Layout: `"TDYN" 1 | symbols | units | modules`. Every node is
//! `kind u8 | line | a | b | num-flag u8 [8 bytes] | kid count | kids`, with varint fields.
//! The kinds and their conventions are mirrored by `enum` K_* in `runtime/dyn/dyn.h`.

use crate::ast::*;
use crate::units::Unit;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

macro_rules! kinds {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u8 = $v;)* };
}
kinds! {
    K_NONE = 0, K_INT = 1, K_FLOAT = 2, K_STR = 3, K_FSTR = 4, K_BOOL = 5, K_NIL = 6, K_ID = 7,
    K_PATH = 8, K_LIST = 9, K_REPEAT = 10, K_DICT = 11, K_UNARY = 12, K_BIN = 13, K_LOGIC = 14,
    K_RANGE = 15, K_CALL = 16, K_MCALL = 17, K_INDEX = 18, K_FIELD = 19, K_LAMBDA = 20,
    K_MACRO = 21, K_IF = 22, K_MATCH = 23, K_ARM = 24, K_BLOCK = 25, K_SPAWN = 26, K_AWAIT = 27,
    K_QTY = 28, K_NAMED = 29, K_PWILD = 30, K_PBIND = 31, K_PLIT = 32, K_PRANGE = 33,
    K_PLIST = 34, K_FN = 35, K_PARAM = 36, K_CONTRACT = 37, K_TNAME = 38, K_TUNIT = 39,
    K_LET = 40, K_ASSIGN = 41, K_RETURN = 42, K_WHILE = 43, K_LOOP = 44, K_FOR = 45,
    K_BREAK = 46, K_CONTINUE = 47, K_ASSERT = 48, K_STRUCT = 49, K_IFACE = 50, K_MACHINE = 51,
    K_STATE = 52, K_TRANS = 53, K_WHEN = 54, K_DEADLINE = 55, K_ALWAYS = 56, K_IMPORT = 57,
    K_TRY = 58, K_MACRODEF = 59, K_GROUP = 60, K_SIG = 61,
}

/// Operator codes shared with the C runtime (`enum` OP_*).
fn opcode(op: &str) -> u64 {
    const OPS: &[&str] = &[
        "", "+", "-", "*", "/", "%", "**", "==", "!=", "<", "<=", ">", ">=", "&", "|", "^", "<<",
        ">>", "in", "not in", "!", "~", "not",
    ];
    OPS.iter().position(|o| *o == op).unwrap_or(0) as u64
}

struct Node {
    kind: u8,
    line: usize,
    a: u64,
    b: u64,
    num: Option<[u8; 8]>,
    float: bool,
    kids: Vec<Node>,
}

fn n(kind: u8) -> Node {
    Node {
        kind,
        line: 0,
        a: 0,
        b: 0,
        num: None,
        float: false,
        kids: Vec::new(),
    }
}

impl Node {
    fn a(mut self, a: u64) -> Node {
        self.a = a;
        self
    }
    fn b(mut self, b: u64) -> Node {
        self.b = b;
        self
    }
    fn line(mut self, l: usize) -> Node {
        self.line = l;
        self
    }
    fn kids(mut self, k: Vec<Node>) -> Node {
        self.kids = k;
        self
    }
    fn int(mut self, i: i64) -> Node {
        self.num = Some(i.to_le_bytes());
        self
    }
    fn f64(mut self, f: f64) -> Node {
        self.num = Some(f.to_le_bytes());
        self.float = true;
        self
    }
}

pub struct Program {
    pub bytes: Vec<u8>,
    pub modules: usize,
    pub nodes: usize,
}

struct Ser {
    /// Whether the build is for the simulated board (where `sim::` hooks exist).
    sim: bool,
    syms: Vec<String>,
    sym_ix: HashMap<String, u64>,
    units: Vec<(u64, f64, [i8; 8], u64)>,
    unit_ix: HashMap<String, u64>,
    modules: Vec<Option<Node>>,
    module_ix: HashMap<PathBuf, u64>,
    dirs: Vec<PathBuf>,
    nodes: usize,
    line: usize,
}

/// Modules implemented in Rust that the firmware runtime does not provide.
const HOST_ONLY: &[&str] = &["nn", "cosim", "phone"];

impl Ser {
    fn sym(&mut self, s: &str) -> u64 {
        if let Some(i) = self.sym_ix.get(s) {
            return *i;
        }
        let i = self.syms.len() as u64;
        self.syms.push(s.to_string());
        self.sym_ix.insert(s.to_string(), i);
        i
    }

    fn unit(&mut self, written: &str, u: &Unit) -> u64 {
        if let Some(i) = self.unit_ix.get(written) {
            return *i;
        }
        let i = self.units.len() as u64;
        let (w, c) = (self.sym(written), self.sym(&u.name));
        self.units.push((w, u.scale, u.dim, c));
        self.unit_ix.insert(written.to_string(), i);
        i
    }

    /// String literals that are unit expressions go in the unit table, so `.to("deg")`,
    /// `quantity(v, "m/s")` and link schemas like `"[m/s]"` resolve in firmware.
    fn note_unit_text(&mut self, s: &str) {
        let inner = s
            .strip_prefix('[')
            .and_then(|x| x.strip_suffix(']'))
            .unwrap_or(s);
        if inner.is_empty() || inner.len() > 24 || inner.contains(' ') {
            return;
        }
        if let Ok(u) = crate::units::parse(inner) {
            self.unit(inner, &u);
        }
    }

    fn ty(&mut self, t: &Option<TypeAnn>) -> Node {
        match t {
            None => n(K_NONE),
            Some(TypeAnn::Name(name, opt)) => n(K_TNAME).a(self.sym(name)).b(*opt as u64),
            Some(TypeAnn::Unit(u, opt)) => {
                n(K_TUNIT).a(self.unit(&u.name.clone(), u)).b(*opt as u64)
            }
        }
    }

    fn names(&mut self, names: &[String]) -> Node {
        let kids = names.iter().map(|x| n(K_ID).a(self.sym(x))).collect();
        n(K_GROUP).kids(kids)
    }

    fn contracts(&mut self, cs: &[Contract]) -> Result<Node, String> {
        let mut kids = Vec::new();
        for c in cs {
            kids.push(
                n(K_CONTRACT)
                    .a(self.sym(&c.text))
                    .kids(vec![self.expr(&c.expr)?]),
            );
        }
        Ok(n(K_GROUP).kids(kids))
    }

    fn params(&mut self, ps: &[Param]) -> Result<Node, String> {
        let mut kids = Vec::new();
        for p in ps {
            let d = match &p.default {
                Some(e) => self.expr(e)?,
                None => n(K_NONE),
            };
            kids.push(
                n(K_PARAM)
                    .a(self.sym(&p.name))
                    .kids(vec![d, self.ty(&p.ty)]),
            );
        }
        Ok(n(K_GROUP).kids(kids))
    }

    fn func(&mut self, d: &FnDef) -> Result<Node, String> {
        // prime names used by `ensure` clauses
        for p in &d.params {
            self.sym(&format!("{}'", p.name));
        }
        self.sym("result'");
        let kids = vec![
            self.params(&d.params)?,
            self.contracts(&d.requires)?,
            self.contracts(&d.ensures)?,
            self.ty(&d.ret),
            self.block(&d.body)?,
        ];
        Ok(n(K_FN).a(self.sym(&d.name)).b(d.is_expr as u64).kids(kids))
    }

    fn opt_block(&mut self, b: &Option<Vec<Stmt>>) -> Result<Node, String> {
        match b {
            Some(b) => self.block(b),
            None => Ok(n(K_NONE)),
        }
    }

    fn block(&mut self, stmts: &[Stmt]) -> Result<Node, String> {
        let mut kids = Vec::new();
        for s in stmts {
            if let Some(k) = self.stmt(s)? {
                kids.push(k);
            }
        }
        Ok(n(K_BLOCK).kids(kids))
    }

    fn pattern(&mut self, p: &Pattern) -> Result<Node, String> {
        Ok(match p {
            Pattern::Wildcard => n(K_PWILD),
            Pattern::Bind(name) => n(K_PBIND).a(self.sym(name)),
            Pattern::Literal(e) => n(K_PLIT).kids(vec![self.expr(e)?]),
            Pattern::Range(a, b, incl) => n(K_PRANGE)
                .a(*incl as u64)
                .kids(vec![self.expr(a)?, self.expr(b)?]),
            Pattern::List(items, rest) => {
                let mut kids = Vec::new();
                for i in items {
                    kids.push(self.pattern(i)?);
                }
                let (a, b) = match rest {
                    Some(r) if r == "_" => (0, 1),
                    Some(r) => (self.sym(r), 2),
                    None => (0, 0),
                };
                n(K_PLIST).a(a).b(b).kids(kids)
            }
        })
    }

    fn exprs(&mut self, es: &[Expr]) -> Result<Vec<Node>, String> {
        es.iter().map(|e| self.expr(e)).collect()
    }

    fn expr(&mut self, e: &Expr) -> Result<Node, String> {
        self.nodes += 1;
        let line = self.line;
        let node = match e {
            Expr::Int(i) => n(K_INT).int(*i),
            Expr::Float(f) => n(K_FLOAT).f64(*f),
            Expr::Str(s) => {
                self.note_unit_text(s);
                n(K_STR).a(self.sym(s))
            }
            Expr::FStr(parts) => {
                let mut kids = Vec::new();
                for p in parts {
                    kids.push(match p {
                        FPart::Lit(l) => n(K_STR).a(self.sym(l)),
                        FPart::Expr(e) => self.expr(e)?,
                    });
                }
                n(K_FSTR).kids(kids)
            }
            Expr::Bool(b) => n(K_BOOL).a(*b as u64),
            Expr::Nil => n(K_NIL),
            Expr::Ident(name, l) => n(K_ID).a(self.sym(name)).line(*l),
            Expr::Path(m, name, l) => {
                if HOST_ONLY.contains(&m.as_str()) {
                    return Err(format!("line {l}: {m}::{name} is not available in firmware (the {m} module runs on desktop and Linux targets)"));
                }
                if m == "sim"
                    && !self.sim
                    && !["virtual_time", "advance", "backend"].contains(&name.as_str())
                {
                    return Err(format!("line {l}: sim::{name} drives the simulated board, which this target does not have (build with --sim or --target sim to test, and keep sim:: calls out of the firmware)"));
                }
                if !firmware_has(m, name) {
                    return Err(format!("line {l}: {m}::{name} is not available in firmware (it runs on the host; see docs/embedded.md for what the firmware VM provides)"));
                }
                n(K_PATH).a(self.sym(m)).b(self.sym(name)).line(*l)
            }
            Expr::List(items) => n(K_LIST).kids(self.exprs(items)?),
            Expr::Repeat(v, c) => n(K_REPEAT).kids(vec![self.expr(v)?, self.expr(c)?]),
            Expr::Dict(pairs) => {
                let mut kids = Vec::new();
                for (k, v) in pairs {
                    kids.push(self.expr(k)?);
                    kids.push(self.expr(v)?);
                }
                n(K_DICT).kids(kids)
            }
            Expr::Unary(op, x, l) => n(K_UNARY).a(opcode(op)).line(*l).kids(vec![self.expr(x)?]),
            Expr::Binary(a, op, b, l) => n(K_BIN)
                .a(opcode(op))
                .line(*l)
                .kids(vec![self.expr(a)?, self.expr(b)?]),
            Expr::Logic(a, is_and, b) => n(K_LOGIC)
                .a(*is_and as u64)
                .kids(vec![self.expr(a)?, self.expr(b)?]),
            Expr::Range(a, b, incl) => n(K_RANGE)
                .a(*incl as u64)
                .kids(vec![self.expr(a)?, self.expr(b)?]),
            Expr::Call(f, args, l) => {
                let mut kids = vec![self.expr(f)?];
                kids.extend(self.exprs(args)?);
                n(K_CALL).line(*l).kids(kids)
            }
            Expr::MethodCall(o, m, args, l) => {
                let mut kids = vec![self.expr(o)?];
                kids.extend(self.exprs(args)?);
                n(K_MCALL).a(self.sym(m)).line(*l).kids(kids)
            }
            Expr::Index(o, i, l) => n(K_INDEX).line(*l).kids(vec![self.expr(o)?, self.expr(i)?]),
            Expr::Field(o, f, l) => n(K_FIELD).a(self.sym(f)).line(*l).kids(vec![self.expr(o)?]),
            Expr::Lambda(d) => n(K_LAMBDA).kids(vec![self.func(d)?]),
            Expr::MacroCall(name, args, l) => {
                let mut kids = Vec::new();
                for a in args {
                    kids.push(match a {
                        MacroArg::Expr(e) => self.expr(e)?,
                        MacroArg::Block(b) => self.block(b)?,
                    });
                }
                n(K_MACRO).a(self.sym(name)).line(*l).kids(kids)
            }
            Expr::IfExpr(c, a, b) => {
                n(K_IF).kids(vec![self.expr(c)?, self.block(a)?, self.opt_block(b)?])
            }
            Expr::Match(s, arms, l) => {
                let mut kids = vec![self.expr(s)?];
                for arm in arms {
                    let mut pats = Vec::new();
                    for p in &arm.patterns {
                        pats.push(self.pattern(p)?);
                    }
                    let guard = match &arm.guard {
                        Some(g) => self.expr(g)?,
                        None => n(K_NONE),
                    };
                    kids.push(n(K_ARM).kids(vec![
                        n(K_GROUP).kids(pats),
                        guard,
                        self.block(&arm.body)?,
                    ]));
                }
                n(K_MATCH).line(*l).kids(kids)
            }
            Expr::Block(b) => self.block(b)?,
            Expr::Spawn(x, l) => n(K_SPAWN).line(*l).kids(vec![self.expr(x)?]),
            Expr::Await(x, l) => n(K_AWAIT).line(*l).kids(vec![self.expr(x)?]),
            Expr::Quantity(v, u) => n(K_QTY).a(self.unit(&u.name.clone(), u)).f64(*v),
            Expr::Named(name, x) => n(K_NAMED).a(self.sym(name)).kids(vec![self.expr(x)?]),
        };
        Ok(if node.line == 0 {
            node.line(line)
        } else {
            node
        })
    }

    fn transition(&mut self, t: &Transition) -> Result<Node, String> {
        let guard = match &t.guard {
            Some(g) => self.expr(g)?,
            None => n(K_NONE),
        };
        let (target, has) = match &t.target {
            Some(s) => (self.sym(s), 1),
            None => (0, 0),
        };
        let kids = vec![
            self.names(&t.params),
            guard,
            self.opt_block(&t.action)?,
            n(K_INT).int(has),
        ];
        Ok(n(K_TRANS)
            .a(self.sym(&t.event))
            .b(target)
            .line(t.line)
            .kids(kids))
    }

    fn import(&mut self, path: &str, line: usize) -> Result<u64, String> {
        let base = self.dirs.last().cloned().unwrap_or_default();
        let mut full = base.join(path);
        if !full.exists() && full.extension().is_none() {
            full.set_extension("rav");
        }
        if !full.exists() {
            full = crate::stdx_path::resolve(path)
                .ok_or_else(|| format!("line {line}: import \"{path}\" was not found"))?;
        }
        let canon = std::fs::canonicalize(&full).map_err(|e| format!("import \"{path}\": {e}"))?;
        if let Some(i) = self.module_ix.get(&canon) {
            return Ok(*i);
        }
        let src = std::fs::read_to_string(&canon).map_err(|e| format!("import \"{path}\": {e}"))?;
        let prog = crate::parser::parse(&src)
            .map_err(|e| format!("{}:{}:{}: {}", canon.display(), e.line, e.col, e.msg))?;
        let i = self.modules.len() as u64;
        self.module_ix.insert(canon.clone(), i);
        self.modules.push(None);
        self.dirs
            .push(canon.parent().map(Path::to_path_buf).unwrap_or_default());
        let body = self
            .block(&prog)
            .map_err(|e| format!("in {}: {e}", canon.display()));
        self.dirs.pop();
        let stem = canon
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "module".into());
        self.modules[i as usize] = Some(body?.a(self.sym(&stem)).b(self.sym(path)));
        Ok(i)
    }

    fn stmt(&mut self, s: &Stmt) -> Result<Option<Node>, String> {
        self.nodes += 1;
        Ok(Some(match s {
            Stmt::Expr(e, l) => {
                self.line = *l;
                self.expr(e)?.line(*l)
            }
            Stmt::Let(names, e, c, l) => {
                self.line = *l;
                let mut kids = vec![self.expr(e)?];
                kids.extend(names.iter().map(|x| n(K_ID).a(self.sym(x))));
                n(K_LET).a(*c as u64).line(*l).kids(kids)
            }
            Stmt::Assign(t, op, e, l) => {
                self.line = *l;
                let target = match t {
                    Target::Name(name) => n(K_ID).a(self.sym(name)),
                    Target::Index(o, i) => n(K_INDEX).kids(vec![self.expr(o)?, self.expr(i)?]),
                    Target::Field(o, f) => n(K_FIELD).a(self.sym(f)).kids(vec![self.expr(o)?]),
                };
                let op = op
                    .as_deref()
                    .map(|o| opcode(o.trim_end_matches('=')))
                    .unwrap_or(0);
                n(K_ASSIGN).a(op).line(*l).kids(vec![target, self.expr(e)?])
            }
            Stmt::Fn(d) => self.func(d)?,
            Stmt::Return(e, l) => {
                self.line = *l;
                let kids = match e {
                    Some(e) => vec![self.expr(e)?],
                    None => vec![],
                };
                n(K_RETURN).line(*l).kids(kids)
            }
            Stmt::If(c, a, b) => {
                n(K_IF).kids(vec![self.expr(c)?, self.block(a)?, self.opt_block(b)?])
            }
            Stmt::While(c, invs, body, l) => {
                self.line = *l;
                n(K_WHILE).line(*l).kids(vec![
                    self.expr(c)?,
                    self.contracts(invs)?,
                    self.block(body)?,
                ])
            }
            Stmt::Loop(b) => n(K_LOOP).kids(vec![self.block(b)?]),
            Stmt::For(names, it, body, l) => {
                self.line = *l;
                let mut kids = vec![self.expr(it)?, self.block(body)?];
                kids.extend(names.iter().map(|x| n(K_ID).a(self.sym(x))));
                n(K_FOR).line(*l).kids(kids)
            }
            Stmt::Break => n(K_BREAK),
            Stmt::Continue => n(K_CONTINUE),
            Stmt::Use(_) => return Ok(None),
            Stmt::Assert(e, msg, text, l) => {
                self.line = *l;
                let m = match msg {
                    Some(m) => self.expr(m)?,
                    None => n(K_NONE),
                };
                n(K_ASSERT)
                    .a(self.sym(text))
                    .line(*l)
                    .kids(vec![self.expr(e)?, m])
            }
            Stmt::Struct(sd) => {
                let mut methods = Vec::new();
                for m in &sd.methods {
                    methods.push(self.func(m)?);
                }
                let kids = vec![
                    self.params(&sd.fields)?,
                    n(K_GROUP).kids(methods),
                    self.contracts(&sd.invariants)?,
                    self.names(&sd.implements),
                ];
                n(K_STRUCT).a(self.sym(&sd.name)).kids(kids)
            }
            Stmt::Interface(d) => {
                let mut sigs = Vec::new();
                for m in &d.methods {
                    let names: Vec<String> = m.params.iter().map(|p| p.name.clone()).collect();
                    let kids = vec![self.ty(&m.ret), self.names(&names)];
                    sigs.push(
                        n(K_SIG)
                            .a(self.sym(&m.name))
                            .b(m.params.len() as u64)
                            .kids(kids),
                    );
                }
                n(K_IFACE)
                    .a(self.sym(&d.name))
                    .line(d.line)
                    .kids(vec![self.names(&d.extends), n(K_GROUP).kids(sigs)])
            }
            Stmt::Machine(m) => {
                let mut states = Vec::new();
                for st in &m.states {
                    let mut kids = vec![self.opt_block(&st.enter)?, self.opt_block(&st.exit)?];
                    for t in &st.transitions {
                        kids.push(self.transition(t)?);
                    }
                    states.push(n(K_STATE).a(self.sym(&st.name)).line(st.line).kids(kids));
                }
                let mut global = Vec::new();
                for t in &m.global {
                    global.push(self.transition(t)?);
                }
                n(K_MACHINE).a(self.sym(&m.name)).kids(vec![
                    self.params(&m.params)?,
                    n(K_GROUP).kids(states),
                    n(K_GROUP).kids(global),
                ])
            }
            Stmt::When(c, hold, body, els, l) => {
                self.line = *l;
                let h = match hold {
                    Some(h) => self.expr(h)?,
                    None => n(K_NONE),
                };
                n(K_WHEN).line(*l).kids(vec![
                    self.expr(c)?,
                    h,
                    self.block(body)?,
                    self.opt_block(els)?,
                ])
            }
            Stmt::Deadline(trig, within, resp, handler, l) => {
                self.line = *l;
                let kids = vec![
                    self.expr(trig)?,
                    self.expr(within)?,
                    self.expr(&resp.expr)?,
                    self.opt_block(handler)?,
                ];
                n(K_DEADLINE).a(self.sym(&resp.text)).line(*l).kids(kids)
            }
            Stmt::Always(c, handler, l) => {
                self.line = *l;
                n(K_ALWAYS)
                    .a(self.sym(&c.text))
                    .line(*l)
                    .kids(vec![self.expr(&c.expr)?, self.opt_block(handler)?])
            }
            Stmt::Import(path, alias, names, l) => {
                let m = self.import(path, *l)?;
                let alias = match alias {
                    Some(a) => self.sym(a),
                    None => u64::MAX,
                };
                n(K_IMPORT)
                    .a(m)
                    .b(alias)
                    .line(*l)
                    .kids(vec![self.names(names)])
            }
            Stmt::Try(body, name, handler) => {
                let name = match name {
                    Some(x) => self.sym(x),
                    None => u64::MAX,
                };
                n(K_TRY)
                    .a(name)
                    .kids(vec![self.block(body)?, self.block(handler)?])
            }
            Stmt::MacroDef(name, params, body) => n(K_MACRODEF)
                .a(self.sym(name))
                .kids(vec![self.names(params), self.block(body)?]),
        }))
    }
}

/// Whether the firmware VM provides `m::name`. The C builtin table is the single source of truth.
fn firmware_has(m: &str, name: &str) -> bool {
    let m = match m {
        "kinematics" => "kin",
        "gpio" => "GPIO",
        "i2c" => "I2C",
        "spi" => "SPI",
        "uart" | "serial" => "UART",
        "pwm" => "PWM",
        "servo" => "Servo",
        "control" => "PID",
        other => other,
    };
    if m == "math" && ["pi", "tau", "e", "inf", "nan"].contains(&name) {
        return true;
    }
    let full = if ["math", "std", "io", "time"].contains(&m) {
        name.to_string()
    } else if (name == "open" || name == "pid")
        && !["sim", "tlink", "kin", "pack", "json", "tlm"].contains(&m)
    {
        format!("{m}::new")
    } else {
        format!("{m}::{name}")
    };
    crate::native::DYN_C.contains(&format!("{{\"{full}\", b_"))
}

/// Lays the tree out as flat tables the VM reads in place (from flash, on a microcontroller).
struct Flat {
    nodes: Vec<[u8; 16]>,
    kids: Vec<u32>,
    nums: Vec<[u8; 8]>,
    paths: u32,
}

impl Flat {
    fn add(&mut self, node: &Node) -> u32 {
        let ix = self.nodes.len() as u32;
        self.nodes.push([0; 16]);
        let mut numt = 0u8;
        let mut u = 0u32;
        if let Some(b) = &node.num {
            numt = if node.float { 2 } else { 1 };
            u = self.nums.len() as u32;
            self.nums.push(*b);
        } else if node.kind == K_PATH {
            u = self.paths; // index of this path's slot in the VM's lookup cache
            self.paths += 1;
        } else if node.kind == K_BLOCK {
            // whether the block declares names of its own (and so needs a scope at run time)
            let declares = node.kids.iter().any(|k| {
                matches!(
                    k.kind,
                    K_LET | K_FN | K_STRUCT | K_IFACE | K_MACHINE | K_IMPORT
                )
            });
            numt = if declares { 1 } else { 2 };
        }
        if !node.kids.is_empty() {
            let ids: Vec<u32> = node.kids.iter().map(|k| self.add(k)).collect();
            u = self.kids.len() as u32;
            self.kids.extend(ids);
        }
        let mut rec = [0u8; 16];
        rec[0] = node.kind;
        rec[1] = numt;
        rec[2..4].copy_from_slice(&(node.kids.len().min(65535) as u16).to_le_bytes());
        rec[4..8].copy_from_slice(&(node.a as u32).to_le_bytes()); // u64::MAX ("none") is -1
        rec[8..12].copy_from_slice(&(node.b as u32).to_le_bytes());
        rec[12..16].copy_from_slice(&u.to_le_bytes());
        self.nodes[ix as usize] = rec;
        ix
    }
}

/// Serialises `prog` (the main file `file`) and everything it imports.
pub fn serialize(prog: &[Stmt], file: &Path, sim: bool) -> Result<Program, String> {
    let mut s = Ser {
        syms: Vec::new(),
        sym_ix: HashMap::new(),
        units: Vec::new(),
        unit_ix: HashMap::new(),
        modules: vec![None],
        module_ix: HashMap::new(),
        dirs: vec![file.parent().map(Path::to_path_buf).unwrap_or_default()],
        nodes: 0,
        line: 0,
        sim,
    };
    s.sym(""); // symbol 0 is the empty string
               // units the runtime itself hands back (Servo.angle(), time helpers) are always known
    for u in ["deg", "rad", "s", "ms", "us", "Hz", "m"] {
        s.note_unit_text(u);
    }
    let main = s.block(prog)?;
    s.modules[0] = Some(main);
    let mut flat = Flat {
        nodes: Vec::new(),
        kids: Vec::new(),
        nums: Vec::new(),
        paths: 0,
    };
    let mut lines: Vec<u16> = Vec::new();
    let mut roots = Vec::new();
    for m in &s.modules {
        roots.push(flat.add(m.as_ref().expect("module serialised")));
    }
    fn collect_lines(n: &Node, out: &mut Vec<u16>) {
        out.push(n.line.min(65535) as u16);
        for k in &n.kids {
            collect_lines(k, out);
        }
    }
    for m in &s.modules {
        collect_lines(m.as_ref().unwrap(), &mut lines);
    }
    let mut strs = Vec::new();
    let mut sym_off = Vec::new();
    for name in &s.syms {
        sym_off.push(strs.len() as u32);
        strs.extend(name.as_bytes());
        strs.push(0);
    }
    // Image: magic, counts, then tables; every table starts on a 4-byte boundary and the
    // numbers on an 8-byte one, so the VM can read them where they lie.
    let mut out = b"TDYN\x02\0\0\0".to_vec();
    let counts = [
        s.syms.len(),
        s.units.len(),
        s.modules.len(),
        flat.nodes.len(),
        flat.kids.len(),
        flat.nums.len(),
        flat.paths as usize,
        strs.len(),
    ];
    for c in counts {
        out.extend((c as u32).to_le_bytes());
    }
    for n in &flat.nums {
        out.extend(n);
    }
    for (written, scale, dim, canon) in &s.units {
        out.extend(scale.to_le_bytes());
        out.extend((*written as u32).to_le_bytes());
        out.extend((*canon as u32).to_le_bytes());
        out.extend(dim.iter().map(|d| *d as u8));
    }
    for r in &roots {
        out.extend(r.to_le_bytes());
    }
    for o in &sym_off {
        out.extend(o.to_le_bytes());
    }
    for n in &flat.nodes {
        out.extend(n);
    }
    for k in &flat.kids {
        out.extend(k.to_le_bytes());
    }
    for l in &lines {
        out.extend(l.to_le_bytes());
    }
    out.extend(&strs);
    Ok(Program {
        bytes: out,
        modules: s.modules.len(),
        nodes: flat.nodes.len(),
    })
}

/// The program as an LLVM module: the T-code as a constant (it stays in flash) and the
/// `tulpa_entry()` that hands it to the VM.
pub fn emit_ll(p: &Program, module: &str) -> String {
    let mut data = String::with_capacity(p.bytes.len() * 3 + 64);
    for b in &p.bytes {
        data.push_str(&format!("\\{b:02X}"));
    }
    format!(
        "; Tulpa dynamic tier: module {module}, {} bytes of T-code ({} modules)\n\
         @tulpa_program = constant [{n} x i8] c\"{data}\", align 8\n\
         @tulpa_program_len = constant i64 {n}\n\
         declare void @tulpa_dyn_run(ptr, i64)\n\
         declare void @tulpa_dyn_violation(ptr)\n\
         ; device and runtime failures are routed into the VM, where `try` can catch them\n\
         define weak void @tulpa_violation(ptr %msg) {{\n  call void @tulpa_dyn_violation(ptr %msg)\n  ret void\n}}\n\
         define void @tulpa_entry() {{\n  call void @tulpa_dyn_run(ptr @tulpa_program, i64 {n})\n  ret void\n}}\n",
        p.bytes.len(),
        p.modules,
        n = p.bytes.len()
    )
}
