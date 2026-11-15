//! `tulpa prove`: static verification of contracts and safety obligations.
//!
//! For every function and struct method the prover collects *obligations*:
//!   - each `ensure` clause,
//!   - each `assert` in the body,
//!   - struct invariants being preserved by every method,
//!   - loop invariants (hold on entry, preserved by the body),
//!   - the `require` clauses of every function it calls,
//!   - divisors never being zero,
//!
//! and tries to prove each one for *all* inputs allowed by the `require` clauses.
//!
//! The analysis is a path-sensitive symbolic execution over linear arithmetic: values are
//! affine forms over symbols (parameters, fields, havocked loop variables), each path carries
//! interval bounds per symbol plus the linear facts implied by the branches taken, and an
//! obligation is proved when its negation is infeasible on every path. Non-linear results
//! (products, quotients, `sqrt`, ...) become fresh symbols bounded by interval arithmetic.
//! Quantities are analysed in SI, integers are exact, floats are treated as reals.
//!
//! The answer per obligation is `proved`, `refuted` (the caller confirms by testing and
//! attaches a counterexample) or `unproved`. `proved` is sound for the modelled semantics; the
//! prover never reports `proved` for something it could not establish.

use crate::ast::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

const INF: f64 = f64::INFINITY;
const MAX_STEPS: usize = 400_000;
const MAX_PATHS: usize = 512;
const UNROLL: usize = 64;

#[derive(Clone, Debug, PartialEq)]
struct Aff {
    c: f64,
    t: BTreeMap<usize, f64>,
}

impl Aff {
    fn konst(c: f64) -> Aff {
        Aff {
            c,
            t: BTreeMap::new(),
        }
    }
    fn sym(i: usize) -> Aff {
        Aff {
            c: 0.0,
            t: BTreeMap::from([(i, 1.0)]),
        }
    }
    fn as_const(&self) -> Option<f64> {
        self.t.is_empty().then_some(self.c)
    }
    fn scale(&self, k: f64) -> Aff {
        if k == 0.0 {
            return Aff::konst(0.0);
        }
        Aff {
            c: self.c * k,
            t: self.t.iter().map(|(i, v)| (*i, v * k)).collect(),
        }
    }
    fn add(&self, o: &Aff) -> Aff {
        let mut r = self.clone();
        r.c += o.c;
        for (i, v) in &o.t {
            let e = r.t.entry(*i).or_insert(0.0);
            *e += v;
            if e.abs() < 1e-12 {
                r.t.remove(i);
            }
        }
        r
    }
    fn sub(&self, o: &Aff) -> Aff {
        self.add(&o.scale(-1.0))
    }
}

#[derive(Clone, Debug)]
struct N {
    aff: Aff,
    int: bool,
}

#[derive(Clone, Debug)]
enum V {
    Num(N),
    Bool(Option<bool>),
    Other,
}

#[derive(Clone)]
struct Fact {
    a: Aff,
    strict: bool,
}

#[derive(Clone, Default)]
struct State {
    vars: HashMap<String, V>,
    bounds: Vec<(f64, f64)>,
    facts: Vec<Fact>,
    ret: Option<V>,
    last: Option<V>,
    /// 1 = break, 2 = continue
    brk: u8,
}

impl State {
    fn bound(&self, i: usize) -> (f64, f64) {
        self.bounds.get(i).copied().unwrap_or((-INF, INF))
    }
    fn set_bound(&mut self, i: usize, b: (f64, f64)) {
        if self.bounds.len() <= i {
            self.bounds.resize(i + 1, (-INF, INF));
        }
        self.bounds[i] = b;
    }
    fn live(&self) -> bool {
        self.ret.is_none() && self.brk == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Proved,
    Refuted,
    Unproved,
    Skipped,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Proved => "proved",
            Status::Refuted => "refuted",
            Status::Unproved => "unproved",
            Status::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Obligation {
    /// `f()`, `Struct.method()` or `machine M`.
    pub ctx: String,
    /// ensure | assert | invariant | loop-invariant | precondition | division | machine
    pub kind: &'static str,
    pub text: String,
    pub line: usize,
    pub status: Status,
    pub detail: String,
}

#[derive(Default)]
pub struct Report {
    pub obligations: Vec<Obligation>,
}

impl Report {
    pub fn count(&self, s: Status) -> usize {
        self.obligations.iter().filter(|o| o.status == s).count()
    }

    /// Functions (by `ctx`) that still have an obligation the prover could not settle.
    pub fn open_contexts(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for o in &self.obligations {
            if o.status == Status::Unproved && !seen.contains(&o.ctx) {
                seen.push(o.ctx.clone());
            }
        }
        seen
    }

    /// Records a failure found by testing `ctx`: the obligation whose text appears in `error`
    /// becomes `refuted`; if none matches, the failure is added as its own row.
    pub fn refute(&mut self, ctx: &str, error: &str, counterexample: &str) {
        let detail = format!(
            "counterexample: {ctx_name}({counterexample})",
            ctx_name = ctx.trim_end_matches("()")
        );
        let mut hit = false;
        for o in self.obligations.iter_mut().filter(|o| o.ctx == ctx) {
            let matches = match o.kind {
                "division" => {
                    error.contains("division by zero") || error.contains("modulo by zero")
                }
                _ => error.contains(o.text.as_str()),
            };
            if matches && o.status != Status::Proved {
                o.status = Status::Refuted;
                o.detail = detail.clone();
                hit = true;
            }
        }
        if !hit {
            self.obligations.push(Obligation {
                ctx: ctx.to_string(),
                kind: "runtime",
                text: error.to_string(),
                line: 0,
                status: Status::Refuted,
                detail,
            });
        }
    }

    /// Marks what testing could not break either.
    pub fn tested(&mut self, ctx: &str, trials: usize) {
        for o in self.obligations.iter_mut().filter(|o| o.ctx == ctx) {
            if o.status == Status::Unproved && o.detail.is_empty() {
                o.detail = format!("no counterexample in {trials} tests");
            }
        }
    }

    pub fn to_json(&self, file: &str) -> serde_json::Value {
        serde_json::json!({
            "file": file,
            "proved": self.count(Status::Proved),
            "refuted": self.count(Status::Refuted),
            "unproved": self.count(Status::Unproved),
            "skipped": self.count(Status::Skipped),
            "obligations": self.obligations.iter().map(|o| serde_json::json!({
                "context": o.ctx, "kind": o.kind, "text": o.text, "line": o.line,
                "status": o.status.word(), "detail": o.detail,
            })).collect::<Vec<_>>(),
        })
    }
}

struct Prover {
    syms: Vec<bool>,
    fns: HashMap<String, Rc<FnDef>>,
    consts: HashMap<String, V>,
    obs: Vec<Obligation>,
    index: HashMap<(String, &'static str, String, usize), usize>,
    ctx: String,
    quiet: usize,
    stack: Vec<String>,
    steps: usize,
    unsupported: Option<String>,
    invariants: Vec<Contract>,
    fields: Vec<(String, V)>,
}

fn is_cmp(op: &str) -> bool {
    matches!(op, "<" | "<=" | ">" | ">=" | "==" | "!=")
}

fn negate(op: &str) -> &'static str {
    match op {
        "<" => ">=",
        "<=" => ">",
        ">" => "<=",
        ">=" => "<",
        "==" => "!=",
        _ => "==",
    }
}

fn integral(x: f64) -> bool {
    x.is_finite() && x.fract() == 0.0
}

/// Names assigned anywhere in `stmts` (`x = ..`, `self.f = ..` as "self.f"), and whether each
/// could receive a non-integer.
fn assigned(stmts: &[Stmt], out: &mut HashMap<String, bool>) {
    fn floaty(e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Ident(..) => false,
            Expr::Binary(a, op, b, _) => op == "**" || floaty(a) || floaty(b),
            Expr::Unary(_, a, _) => floaty(a),
            _ => true,
        }
    }
    fn blocks(e: &Expr, out: &mut HashMap<String, bool>) {
        match e {
            Expr::MacroCall(_, args, _) => {
                for a in args {
                    match a {
                        MacroArg::Block(b) => assigned(b, out),
                        MacroArg::Expr(e) => blocks(e, out),
                    }
                }
            }
            Expr::IfExpr(_, a, b) => {
                assigned(a, out);
                if let Some(b) = b {
                    assigned(b, out);
                }
            }
            Expr::Block(b) => assigned(b, out),
            Expr::Match(_, arms, _) => {
                for arm in arms {
                    assigned(&arm.body, out);
                }
            }
            _ => {}
        }
    }
    for s in stmts {
        match s {
            Stmt::Assign(t, _, e, _) => {
                let name = match t {
                    Target::Name(n) => Some(n.clone()),
                    Target::Field(Expr::Ident(o, _), f) if o == "self" => Some(format!("self.{f}")),
                    Target::Index(Expr::Ident(n, _), _) | Target::Field(Expr::Ident(n, _), _) => {
                        Some(n.clone())
                    }
                    _ => None,
                };
                if let Some(n) = name {
                    let f = out.entry(n).or_insert(false);
                    *f = *f || floaty(e);
                }
                blocks(e, out);
            }
            Stmt::Let(names, e, _, _) => {
                for n in names {
                    let f = out.entry(n.clone()).or_insert(false);
                    *f = *f || floaty(e);
                }
                blocks(e, out);
            }
            Stmt::Expr(e, _) => blocks(e, out),
            Stmt::If(_, a, b) => {
                assigned(a, out);
                if let Some(b) = b {
                    assigned(b, out);
                }
            }
            Stmt::While(_, _, b, _) | Stmt::Loop(b) => assigned(b, out),
            Stmt::For(names, _, b, _) => {
                for n in names {
                    out.insert(n.clone(), true);
                }
                assigned(b, out);
            }
            Stmt::Try(a, _, b) => {
                assigned(a, out);
                assigned(b, out);
            }
            _ => {}
        }
    }
}

impl Prover {
    fn fresh(&mut self, st: &mut State, lo: f64, hi: f64, int: bool) -> N {
        let i = self.syms.len();
        self.syms.push(int);
        let (lo, hi) = if int {
            (lo.ceil(), hi.floor())
        } else {
            (lo, hi)
        };
        st.set_bound(i, (lo, hi));
        N {
            aff: Aff::sym(i),
            int,
        }
    }

    fn range(&self, st: &State, a: &Aff) -> (f64, f64) {
        let (mut lo, mut hi) = (a.c, a.c);
        for (i, k) in &a.t {
            let (l, h) = st.bound(*i);
            let (x, y) = if *k > 0.0 {
                (k * l, k * h)
            } else {
                (k * h, k * l)
            };
            lo += x;
            hi += y;
        }
        if lo.is_nan() {
            lo = -INF;
        }
        if hi.is_nan() {
            hi = INF;
        }
        (lo, hi)
    }

    /// Is `a >= 0` (or `a > 0`) guaranteed in `st`?
    fn implied(&self, st: &State, a: &Aff, strict: bool) -> bool {
        let pos = |lo: f64, have_strict: bool| lo > 0.0 || (lo >= 0.0 && (!strict || have_strict));
        if pos(self.range(st, a).0, false) {
            return true;
        }
        for f in &st.facts {
            let mut ks = vec![1.0];
            for (i, v) in &a.t {
                if let Some(fv) = f.a.t.get(i) {
                    let k = v / fv;
                    if k > 0.0 && k != 1.0 {
                        ks.push(k);
                    }
                }
            }
            for k in ks {
                if pos(self.range(st, &a.sub(&f.a.scale(k))).0, f.strict) {
                    return true;
                }
            }
        }
        if st.facts.len() <= 48 {
            for (i, f1) in st.facts.iter().enumerate() {
                let r1 = a.sub(&f1.a);
                for f2 in &st.facts[i + 1..] {
                    if pos(self.range(st, &r1.sub(&f2.a)).0, f1.strict || f2.strict) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Adds `a >= 0` (or `a > 0`) to the path. Returns false if the path becomes infeasible.
    fn add_fact(&mut self, st: &mut State, a: Aff, strict: bool) -> bool {
        if let Some(c) = a.as_const() {
            return c > 0.0 || (!strict && c >= 0.0);
        }
        let all_int = a.t.iter().all(|(i, k)| self.syms[*i] && integral(*k)) && integral(a.c);
        let (a, strict) = if all_int && strict {
            (a.add(&Aff::konst(-1.0)), false)
        } else {
            (a, strict)
        };
        // contradiction with what is already known?
        if self.implied(st, &a.scale(-1.0), !strict) {
            return false;
        }
        // tighten the bound of each symbol: k*x >= -(rest), using the largest possible rest
        for (i, k) in a.t.clone() {
            let mut rest = a.clone();
            rest.t.remove(&i);
            let rh = self.range(st, &rest).1;
            if !rh.is_finite() {
                continue;
            }
            let (mut lo, mut hi) = st.bound(i);
            let limit = -rh / k;
            if k > 0.0 {
                lo = lo.max(if self.syms[i] {
                    (limit - 1e-9).ceil()
                } else {
                    limit
                });
            } else {
                hi = hi.min(if self.syms[i] {
                    (limit + 1e-9).floor()
                } else {
                    limit
                });
            }
            if lo > hi {
                return false;
            }
            st.set_bound(i, (lo, hi));
        }
        if !st.facts.iter().any(|f| f.a == a && f.strict == strict) {
            st.facts.push(Fact { a, strict });
        }
        true
    }

    fn assume_cmp(&mut self, st: State, l: &N, op: &str, r: &N, truth: bool) -> Vec<State> {
        let op = if truth {
            op.to_string()
        } else {
            negate(op).to_string()
        };
        let d = l.aff.sub(&r.aff);
        let neg = d.scale(-1.0);
        let mut out = Vec::new();
        let mut one = |me: &mut Self, mut s: State, facts: &[(Aff, bool)]| {
            if facts
                .iter()
                .all(|(a, strict)| me.add_fact(&mut s, a.clone(), *strict))
            {
                out.push(s);
            }
        };
        match op.as_str() {
            "<" => one(self, st, &[(neg, true)]),
            "<=" => one(self, st, &[(neg, false)]),
            ">" => one(self, st, &[(d, true)]),
            ">=" => one(self, st, &[(d, false)]),
            "==" => one(self, st, &[(d, false), (neg, false)]),
            _ => {
                one(self, st.clone(), &[(d, true)]);
                one(self, st, &[(neg, true)]);
            }
        }
        out
    }

    /// States in which `e` evaluates to `truth`. Empty = impossible.
    fn assume(&mut self, st: State, e: &Expr, truth: bool) -> Vec<State> {
        self.steps += 1;
        match e {
            Expr::Bool(b) => {
                if *b == truth {
                    vec![st]
                } else {
                    vec![]
                }
            }
            Expr::Unary(op, x, _) if op == "not" || op == "!" => self.assume(st, x, !truth),
            Expr::Logic(a, is_and, b) => {
                let mut out = Vec::new();
                if *is_and == truth {
                    // (a and b) is true  /  (a or b) is false: both sides take `truth`
                    for s in self.assume(st, a, truth) {
                        out.extend(self.assume(s, b, truth));
                    }
                } else {
                    out.extend(self.assume(st.clone(), a, truth));
                    for s in self.assume(st, a, !truth) {
                        out.extend(self.assume(s, b, truth));
                    }
                }
                out
            }
            Expr::Binary(l, op, r, _) if is_cmp(op) => {
                let mut out = Vec::new();
                for (s1, a) in self.eval(st, l) {
                    for (s2, b) in self.eval(s1, r) {
                        match (&a, &b) {
                            (V::Num(x), V::Num(y)) => {
                                out.extend(self.assume_cmp(s2, x, op, y, truth))
                            }
                            (V::Bool(Some(x)), V::Bool(Some(y))) if op == "==" || op == "!=" => {
                                if ((x == y) == (op == "==")) == truth {
                                    out.push(s2);
                                }
                            }
                            _ => out.push(s2),
                        }
                    }
                }
                out
            }
            _ => {
                let mut out = Vec::new();
                for (s, v) in self.eval(st, e) {
                    match v {
                        V::Bool(Some(b)) => {
                            if b == truth {
                                out.push(s);
                            }
                        }
                        V::Num(n) => {
                            let zero = N {
                                aff: Aff::konst(0.0),
                                int: true,
                            };
                            out.extend(self.assume_cmp(s, &n, "!=", &zero, truth));
                        }
                        _ => out.push(s),
                    }
                }
                out
            }
        }
    }

    fn decide(&mut self, st: State, e: &Expr) -> Vec<(State, V)> {
        let t = self.assume(st.clone(), e, true);
        let f = self.assume(st.clone(), e, false);
        let v = match (t.is_empty(), f.is_empty()) {
            (false, true) => Some(true),
            (true, false) => Some(false),
            _ => None,
        };
        vec![(st, V::Bool(v))]
    }

    fn record(&mut self, kind: &'static str, text: &str, line: usize, proved: bool) {
        if self.quiet > 0 && kind != "precondition" {
            return;
        }
        let key = (self.ctx.clone(), kind, text.to_string(), line);
        match self.index.get(&key) {
            Some(i) => {
                if !proved {
                    self.obs[*i].status = Status::Unproved;
                }
            }
            None => {
                self.index.insert(key, self.obs.len());
                self.obs.push(Obligation {
                    ctx: self.ctx.clone(),
                    kind,
                    text: text.to_string(),
                    line,
                    status: if proved {
                        Status::Proved
                    } else {
                        Status::Unproved
                    },
                    detail: String::new(),
                });
            }
        }
    }

    fn prove(&mut self, st: &State, e: &Expr) -> bool {
        self.assume(st.clone(), e, false).is_empty()
    }

    fn nonzero(&mut self, st: &State, n: &N, line: usize) {
        let ok = self.implied(st, &n.aff, true) || self.implied(st, &n.aff.scale(-1.0), true);
        self.record("division", "divisor is never zero", line, ok);
    }

    fn arith(&mut self, mut st: State, a: N, op: &str, b: N, line: usize) -> (State, V) {
        let int = a.int && b.int;
        let (ac, bc) = (a.aff.as_const(), b.aff.as_const());
        let n = match op {
            "+" => N {
                aff: a.aff.add(&b.aff),
                int,
            },
            "-" => N {
                aff: a.aff.sub(&b.aff),
                int,
            },
            "*" => match (ac, bc) {
                (Some(k), _) => N {
                    aff: b.aff.scale(k),
                    int,
                },
                (_, Some(k)) => N {
                    aff: a.aff.scale(k),
                    int,
                },
                _ => {
                    let (al, ah) = self.range(&st, &a.aff);
                    let (bl, bh) = self.range(&st, &b.aff);
                    let ps = [al * bl, al * bh, ah * bl, ah * bh];
                    let lo = ps
                        .iter()
                        .cloned()
                        .fold(INF, |m, x| if x.is_nan() { -INF } else { m.min(x) });
                    let hi = ps
                        .iter()
                        .cloned()
                        .fold(-INF, |m, x| if x.is_nan() { INF } else { m.max(x) });
                    let same = a.aff == b.aff;
                    self.fresh(&mut st, if same { lo.max(0.0) } else { lo }, hi, int)
                }
            },
            "/" | "%" => {
                self.nonzero(&st, &b, line);
                let (al, ah) = self.range(&st, &a.aff);
                let (bl, bh) = self.range(&st, &b.aff);
                if op == "%" {
                    let m = bl.abs().max(bh.abs());
                    match (ac, bc) {
                        (Some(x), Some(y)) if y != 0.0 => N {
                            aff: Aff::konst(x.rem_euclid(y)),
                            int,
                        },
                        _ => self.fresh(&mut st, 0.0, if int { m - 1.0 } else { m }, int),
                    }
                } else if let (false, Some(k)) = (int, bc.filter(|k| *k != 0.0)) {
                    N {
                        aff: a.aff.scale(1.0 / k),
                        int: false,
                    }
                } else if let (Some(x), Some(y)) = (ac, bc.filter(|k| *k != 0.0)) {
                    let q = if int { (x / y).floor() } else { x / y };
                    let q = if int && y < 0.0 {
                        (x as i64).div_euclid(y as i64) as f64
                    } else {
                        q
                    };
                    N {
                        aff: Aff::konst(q),
                        int,
                    }
                } else if let (true, Some(k)) = (int, bc.filter(|k| *k > 0.0)) {
                    // floor division by a positive constant: k*q <= a <= k*q + k - 1
                    let q = self.fresh(&mut st, (al / k).floor(), (ah / k).floor(), true);
                    let kq = q.aff.scale(k);
                    let _ = self.add_fact(&mut st, a.aff.sub(&kq), false);
                    let _ = self.add_fact(&mut st, kq.add(&Aff::konst(k - 1.0)).sub(&a.aff), false);
                    q
                } else if bl > 0.0 || bh < 0.0 {
                    let qs = [al / bl, al / bh, ah / bl, ah / bh];
                    let lo = qs
                        .iter()
                        .cloned()
                        .fold(INF, |m, x| if x.is_nan() { -INF } else { m.min(x) });
                    let hi = qs
                        .iter()
                        .cloned()
                        .fold(-INF, |m, x| if x.is_nan() { INF } else { m.max(x) });
                    if int && bl > 0.0 {
                        self.fresh(&mut st, lo.floor(), hi.floor(), true)
                    } else if int {
                        self.fresh(&mut st, lo.floor() - 1.0, hi.ceil() + 1.0, true)
                    } else {
                        self.fresh(&mut st, lo, hi, false)
                    }
                } else {
                    self.fresh(&mut st, -INF, INF, int)
                }
            }
            "**" => match (ac, bc) {
                (Some(x), Some(y)) => N {
                    aff: Aff::konst(x.powf(y)),
                    int: int && y >= 0.0,
                },
                (_, Some(2.0)) => {
                    let (al, ah) = self.range(&st, &a.aff);
                    let hi = (al * al).max(ah * ah);
                    let lo = if al <= 0.0 && ah >= 0.0 {
                        0.0
                    } else {
                        (al * al).min(ah * ah)
                    };
                    self.fresh(&mut st, lo, hi, int)
                }
                _ => self.fresh(&mut st, -INF, INF, false),
            },
            _ => return (st, V::Other),
        };
        (st, V::Num(n))
    }

    fn havoc(&mut self, st: &mut State, names: &HashMap<String, bool>) {
        for (name, floaty) in names {
            let Some(v) = st.vars.get(name).cloned() else {
                continue;
            };
            let nv = match v {
                V::Num(n) => V::Num(self.fresh(st, -INF, INF, n.int && !*floaty)),
                V::Bool(_) => V::Bool(None),
                V::Other => V::Other,
            };
            st.vars.insert(name.clone(), nv);
        }
    }

    fn havoc_self(&mut self, st: &mut State) -> Vec<State> {
        let names: HashMap<String, bool> = st
            .vars
            .keys()
            .filter(|k| k.starts_with("self."))
            .map(|k| (k.clone(), false))
            .collect();
        // keep each field's int-ness: a method cannot change a typed field's type
        for name in names.keys() {
            if let Some(V::Num(n)) = st.vars.get(name).cloned() {
                let f = self.fresh(st, -INF, INF, n.int);
                st.vars.insert(name.clone(), V::Num(f));
            }
        }
        let mut states = vec![st.clone()];
        for inv in self.invariants.clone() {
            let mut next = Vec::new();
            for s in states {
                next.extend(self.assume(s, &inv.expr, true));
            }
            states = next;
        }
        states
    }

    fn fork2(&mut self, st: State, a: &N, op: &str, b: &N, yes: V, no: V) -> Vec<(State, V)> {
        let mut out = Vec::new();
        for s in self.assume_cmp(st.clone(), a, op, b, true) {
            out.push((s, yes.clone()));
        }
        for s in self.assume_cmp(st, a, op, b, false) {
            out.push((s, no.clone()));
        }
        out
    }

    #[allow(clippy::type_complexity)]
    fn eval_args(&mut self, st: State, args: &[Expr]) -> Vec<(State, Vec<(Option<String>, V)>)> {
        let mut acc = vec![(st, Vec::new())];
        for a in args {
            let (name, e) = match a {
                Expr::Named(n, e) => (Some(n.clone()), &**e),
                e => (None, e),
            };
            let mut next = Vec::new();
            for (s, vs) in acc {
                for (s2, v) in self.eval(s, e) {
                    let mut vs2: Vec<(Option<String>, V)> = vs.clone();
                    vs2.push((name.clone(), v));
                    next.push((s2, vs2));
                }
            }
            acc = next;
            if acc.len() > MAX_PATHS {
                self.unsupported.get_or_insert("too many paths".into());
                acc.truncate(1);
            }
        }
        acc
    }

    fn builtin(&mut self, mut st: State, name: &str, vs: &[V]) -> Option<Vec<(State, V)>> {
        let num = |i: usize| match vs.get(i) {
            Some(V::Num(n)) => Some(n.clone()),
            _ => None,
        };
        let zero = N {
            aff: Aff::konst(0.0),
            int: true,
        };
        Some(match (name, vs.len()) {
            ("abs", 1) => {
                let x = num(0)?;
                let neg = N {
                    aff: x.aff.scale(-1.0),
                    int: x.int,
                };
                self.fork2(st, &x, ">=", &zero, V::Num(x.clone()), V::Num(neg))
            }
            ("min", 2) | ("max", 2) => {
                let (a, b) = (num(0)?, num(1)?);
                let op = if name == "min" { "<=" } else { ">=" };
                self.fork2(st, &a, op, &b, V::Num(a.clone()), V::Num(b.clone()))
            }
            ("clamp", 3) => {
                let (x, lo, hi) = (num(0)?, num(1)?, num(2)?);
                let mut out = Vec::new();
                for s in self.assume_cmp(st.clone(), &x, "<", &lo, true) {
                    out.push((s, V::Num(lo.clone())));
                }
                for s in self.assume_cmp(st, &x, "<", &lo, false) {
                    out.extend(self.fork2(s, &x, ">", &hi, V::Num(hi.clone()), V::Num(x.clone())));
                }
                out
            }
            ("float", 1) => {
                let x = num(0)?;
                vec![(
                    st,
                    V::Num(N {
                        aff: x.aff,
                        int: false,
                    }),
                )]
            }
            ("int", 1) | ("floor", 1) | ("ceil", 1) | ("round", 1) => {
                let x = num(0)?;
                if x.int {
                    return Some(vec![(st, V::Num(x))]);
                }
                let (lo, hi) = self.range(&st, &x.aff);
                let n = self.fresh(&mut st, lo.floor(), hi.ceil(), true);
                vec![(st, V::Num(n))]
            }
            ("sqrt", 1) => {
                let (lo, hi) = self.range(&st, &num(0)?.aff);
                let n = self.fresh(&mut st, lo.max(0.0).sqrt(), hi.max(0.0).sqrt(), false);
                vec![(st, V::Num(n))]
            }
            ("sin", 1) | ("cos", 1) => {
                let n = self.fresh(&mut st, -1.0, 1.0, false);
                vec![(st, V::Num(n))]
            }
            ("hypot", 2) => {
                let n = self.fresh(&mut st, 0.0, INF, false);
                vec![(st, V::Num(n))]
            }
            ("len", 1) | ("millis", 0) | ("micros", 0) => {
                let n = self.fresh(&mut st, 0.0, INF, true);
                vec![(st, V::Num(n))]
            }
            _ => return None,
        })
    }

    fn call_user(
        &mut self,
        st: State,
        def: &Rc<FnDef>,
        args: Vec<(Option<String>, V)>,
        line: usize,
    ) -> Vec<(State, V)> {
        // bind parameters
        let mut bound: HashMap<String, V> = self.consts.clone();
        let mut pos = args
            .iter()
            .filter(|(n, _)| n.is_none())
            .map(|(_, v)| v.clone());
        let mut st = st;
        for p in &def.params {
            let named = args
                .iter()
                .find(|(n, _)| n.as_deref() == Some(&p.name))
                .map(|(_, v)| v.clone());
            let v = match named.or_else(|| pos.next()) {
                Some(v) => v,
                None => match &p.default {
                    Some(d) => {
                        let saved = std::mem::replace(&mut st.vars, bound.clone());
                        let mut r = self.eval(st.clone(), d);
                        st.vars = saved;
                        if r.len() == 1 {
                            let (s, v) = r.remove(0);
                            let vars = std::mem::take(&mut st.vars);
                            st = s;
                            st.vars = vars;
                            v
                        } else {
                            V::Other
                        }
                    }
                    None => V::Other,
                },
            };
            bound.insert(p.name.clone(), v);
        }
        let saved = std::mem::replace(&mut st.vars, bound);
        let (saved_ret, saved_last) = (st.ret.take(), st.last.take());
        // the caller must establish the callee's preconditions
        for r in &def.requires {
            let ok = self.prove(&st, &r.expr);
            self.record(
                "precondition",
                &format!("{}(): require {}", def.name, r.text),
                line,
                ok,
            );
        }
        let restore = |mut s: State, saved: &HashMap<String, V>| {
            s.vars = saved.clone();
            s.ret = saved_ret.clone();
            s.last = saved_last.clone();
            s.brk = 0;
            s
        };
        let inline = self.stack.len() < 4 && !self.stack.contains(&def.name);
        if inline {
            self.stack.push(def.name.clone());
            self.quiet += 1;
            let before = self.unsupported.take();
            let mut states = vec![st.clone()];
            for r in &def.requires {
                let mut next = Vec::new();
                for s in states {
                    next.extend(self.assume(s, &r.expr, true));
                }
                states = next;
            }
            let outs = self.exec(states, &def.body);
            self.quiet -= 1;
            self.stack.pop();
            let failed = self.unsupported.take().is_some();
            self.unsupported = before;
            if !failed {
                return outs
                    .into_iter()
                    .map(|s| {
                        let v = s.ret.clone().or(s.last.clone()).unwrap_or(V::Other);
                        (restore(s, &saved), v)
                    })
                    .collect();
            }
        }
        // modular: all we know about the result is the callee's `ensure` clauses
        let numeric = matches!(&def.ret, Some(TypeAnn::Name(n, _)) if n == "int" || n == "float")
            || matches!(&def.ret, Some(TypeAnn::Unit(..)));
        if !numeric && def.ensures.is_empty() {
            return vec![(restore(st, &saved), V::Other)];
        }
        let int = matches!(&def.ret, Some(TypeAnn::Name(n, _)) if n == "int");
        let res = V::Num(self.fresh(&mut st, -INF, INF, int));
        st.vars.insert("result'".into(), res.clone());
        for p in &def.params {
            if let Some(v) = st.vars.get(&p.name).cloned() {
                st.vars.insert(format!("{}'", p.name), v);
            }
        }
        let mut states = vec![st];
        for e in &def.ensures {
            let mut next = Vec::new();
            for s in states {
                next.extend(self.assume(s, &e.expr, true));
            }
            states = next;
        }
        states
            .into_iter()
            .map(|s| (restore(s, &saved), res.clone()))
            .collect()
    }

    fn eval(&mut self, st: State, e: &Expr) -> Vec<(State, V)> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            self.unsupported
                .get_or_insert("analysis budget exceeded".into());
            return vec![(st, V::Other)];
        }
        let konst = |c: f64, int: bool| {
            V::Num(N {
                aff: Aff::konst(c),
                int,
            })
        };
        match e {
            Expr::Int(i) => vec![(st, konst(*i as f64, true))],
            Expr::Float(f) => vec![(st, konst(*f, false))],
            Expr::Quantity(v, u) => vec![(st, konst(v * u.scale, false))],
            Expr::Bool(b) => vec![(st, V::Bool(Some(*b)))],
            Expr::Ident(name, _) => {
                let v = st.vars.get(name).cloned().unwrap_or(V::Other);
                vec![(st, v)]
            }
            Expr::Path(m, name, _) if m == "math" => {
                let v = match name.as_str() {
                    "pi" => konst(std::f64::consts::PI, false),
                    "tau" => konst(std::f64::consts::TAU, false),
                    "e" => konst(std::f64::consts::E, false),
                    _ => V::Other,
                };
                vec![(st, v)]
            }
            Expr::Named(_, x) => self.eval(st, x),
            Expr::Unary(op, x, _) if op == "-" => self
                .eval(st, x)
                .into_iter()
                .map(|(s, v)| match v {
                    V::Num(n) => (
                        s,
                        V::Num(N {
                            aff: n.aff.scale(-1.0),
                            int: n.int,
                        }),
                    ),
                    _ => (s, V::Other),
                })
                .collect(),
            Expr::Unary(op, _, _) if op == "not" || op == "!" => self.decide(st, e),
            Expr::Logic(..) => self.decide(st, e),
            Expr::Binary(_, op, _, _) if is_cmp(op) => self.decide(st, e),
            Expr::Binary(l, op, r, line) => {
                let mut out = Vec::new();
                for (s1, a) in self.eval(st, l) {
                    for (s2, b) in self.eval(s1, r) {
                        match (&a, &b) {
                            (V::Num(x), V::Num(y)) => {
                                out.push(self.arith(s2, x.clone(), op, y.clone(), *line))
                            }
                            _ => out.push((s2, V::Other)),
                        }
                    }
                }
                out
            }
            Expr::IfExpr(c, a, b) => {
                let mut out = Vec::new();
                let t = self.assume(st.clone(), c, true);
                for s in self.exec(t, a) {
                    let v = s.last.clone().unwrap_or(V::Other);
                    out.push((s, v));
                }
                let f = self.assume(st, c, false);
                match b {
                    Some(b) => {
                        for s in self.exec(f, b) {
                            let v = s.last.clone().unwrap_or(V::Other);
                            out.push((s, v));
                        }
                    }
                    None => out.extend(f.into_iter().map(|s| (s, V::Other))),
                }
                out
            }
            Expr::Field(o, f, _) => match &**o {
                Expr::Ident(n, _) if n == "self" => {
                    let v = st
                        .vars
                        .get(&format!("self.{f}"))
                        .cloned()
                        .unwrap_or(V::Other);
                    vec![(st, v)]
                }
                _ if f == "si" => self.eval(st, o),
                _ => vec![(st, V::Other)],
            },
            Expr::MethodCall(o, m, args, _) => {
                if matches!(&**o, Expr::Ident(n, _) if n == "self") {
                    let mut s = st;
                    return self
                        .havoc_self(&mut s)
                        .into_iter()
                        .map(|s| (s, V::Other))
                        .collect();
                }
                match (m.as_str(), args.as_slice()) {
                    ("to", [Expr::Str(_)]) => self.eval(st, o),
                    ("in", [Expr::Str(u)]) => match crate::units::parse(u) {
                        Ok(unit) => self
                            .eval(st, o)
                            .into_iter()
                            .map(|(s, v)| match v {
                                V::Num(n) => (
                                    s,
                                    V::Num(N {
                                        aff: n.aff.scale(1.0 / unit.scale),
                                        int: false,
                                    }),
                                ),
                                _ => (s, V::Other),
                            })
                            .collect(),
                        Err(_) => vec![(st, V::Other)],
                    },
                    _ => {
                        // a method on a local object may mutate it
                        let mut s = st;
                        if let Expr::Ident(n, _) = &**o {
                            if s.vars.contains_key(n) {
                                s.vars.insert(n.clone(), V::Other);
                            }
                        }
                        vec![(s, V::Other)]
                    }
                }
            }
            Expr::Call(f, args, line) => {
                let Expr::Ident(name, _) = &**f else {
                    return vec![(st, V::Other)];
                };
                let mut out = Vec::new();
                for (s, vs) in self.eval_args(st, args) {
                    if !s.vars.contains_key(name) {
                        if let Some(def) = self.fns.get(name).cloned() {
                            out.extend(self.call_user(s, &def, vs, *line));
                            continue;
                        }
                        let plain: Vec<V> = vs.iter().map(|(_, v)| v.clone()).collect();
                        if let Some(r) = self.builtin(s.clone(), name, &plain) {
                            out.extend(r);
                            continue;
                        }
                    }
                    out.push((s, V::Other));
                }
                out
            }
            Expr::MacroCall(name, args, line) => {
                let mut s = st;
                if name == "assert" {
                    if let Some(MacroArg::Expr(c)) = args.first() {
                        let ok = self.prove(&s, c);
                        self.record("assert", "assert!(...)", *line, ok);
                        return self
                            .assume(s, c, true)
                            .into_iter()
                            .map(|s| (s, V::Other))
                            .collect();
                    }
                }
                let mut names = HashMap::new();
                for a in args {
                    if let MacroArg::Block(b) = a {
                        assigned(b, &mut names);
                    }
                }
                self.havoc(&mut s, &names);
                vec![(s, V::Other)]
            }
            Expr::Match(_, arms, _) => {
                let mut names = HashMap::new();
                for a in arms {
                    assigned(&a.body, &mut names);
                }
                let mut s = st;
                self.havoc(&mut s, &names);
                vec![(s, V::Other)]
            }
            Expr::Spawn(..) | Expr::Await(..) => {
                self.unsupported.get_or_insert("tasks".into());
                vec![(st, V::Other)]
            }
            _ => vec![(st, V::Other)],
        }
    }

    fn assign(
        &mut self,
        st: State,
        name: String,
        op: &Option<String>,
        e: &Expr,
        line: usize,
    ) -> Vec<State> {
        let mut out = Vec::new();
        for (mut s, v) in self.eval(st, e) {
            let v = match op.as_deref().map(|o| o.trim_end_matches('=')) {
                Some(o) if !o.is_empty() => match (s.vars.get(&name).cloned(), v) {
                    (Some(V::Num(a)), V::Num(b)) => {
                        let (s2, r) = self.arith(s, a, o, b, line);
                        s = s2;
                        r
                    }
                    _ => V::Other,
                },
                _ => v,
            };
            s.vars.insert(name.clone(), v);
            s.last = None;
            out.push(s);
        }
        out
    }

    fn cap(&mut self, mut states: Vec<State>) -> Vec<State> {
        if states.len() > MAX_PATHS {
            self.unsupported.get_or_insert("too many paths".into());
            states.truncate(MAX_PATHS);
        }
        states
    }

    fn exec(&mut self, mut states: Vec<State>, stmts: &[Stmt]) -> Vec<State> {
        for stmt in stmts {
            let (live, mut done): (Vec<State>, Vec<State>) =
                states.into_iter().partition(|s| s.live());
            let mut next = Vec::new();
            for st in live {
                next.extend(self.step(st, stmt));
            }
            done.extend(next);
            states = self.cap(done);
        }
        states
    }

    fn all(&mut self, states: Vec<State>, cs: &[Contract], truth: bool) -> Vec<State> {
        let mut states = states;
        for c in cs {
            let mut next = Vec::new();
            for s in states {
                next.extend(self.assume(s, &c.expr, truth));
            }
            states = next;
        }
        states
    }

    fn while_loop(
        &mut self,
        st: State,
        cond: &Expr,
        invs: &[Contract],
        body: &[Stmt],
        line: usize,
    ) -> Vec<State> {
        // 1. unroll while the condition is decidable
        if invs.is_empty() {
            let mut active = vec![st.clone()];
            let mut done = Vec::new();
            let mut ok = true;
            let mut iters = 0;
            'outer: while !active.is_empty() {
                if iters >= UNROLL || active.len() + done.len() > 128 {
                    ok = false;
                    break;
                }
                let mut next = Vec::new();
                for s in active {
                    let t = self.assume(s.clone(), cond, true);
                    let f = self.assume(s, cond, false);
                    if !t.is_empty() && !f.is_empty() {
                        ok = false;
                        break 'outer;
                    }
                    done.extend(f);
                    for mut b in self.exec(t, body) {
                        if b.ret.is_some() || b.brk == 1 {
                            b.brk = 0;
                            done.push(b);
                        } else {
                            b.brk = 0;
                            next.push(b);
                        }
                    }
                }
                active = next;
                iters += 1;
            }
            if ok {
                return done;
            }
        }
        // 2. inductive: havoc what the loop assigns, assume the invariants
        for inv in invs {
            let ok = self.prove(&st, &inv.expr);
            self.record(
                "loop-invariant",
                &format!("holds on entry: {}", inv.text),
                line,
                ok,
            );
        }
        let mut names = HashMap::new();
        assigned(body, &mut names);
        let mut h = st;
        self.havoc(&mut h, &names);
        let hs = self.all(vec![h], invs, true);
        let mut out = Vec::new();
        for s in hs {
            let t = self.assume(s.clone(), cond, true);
            for mut b in self.exec(t, body) {
                if b.ret.is_some() {
                    out.push(b);
                    continue;
                }
                if b.brk == 1 {
                    b.brk = 0;
                    out.push(b);
                    continue;
                }
                b.brk = 0;
                for inv in invs {
                    let ok = self.prove(&b, &inv.expr);
                    self.record(
                        "loop-invariant",
                        &format!("preserved: {}", inv.text),
                        line,
                        ok,
                    );
                }
            }
            out.extend(self.assume(s, cond, false));
        }
        out
    }

    fn step(&mut self, mut st: State, stmt: &Stmt) -> Vec<State> {
        self.steps += 1;
        match stmt {
            Stmt::Let(names, e, _, _) => {
                if let [name] = names.as_slice() {
                    self.eval(st, e)
                        .into_iter()
                        .map(|(mut s, v)| {
                            s.vars.insert(name.clone(), v);
                            s.last = None;
                            s
                        })
                        .collect()
                } else {
                    self.eval(st, e)
                        .into_iter()
                        .map(|(mut s, _)| {
                            for n in names {
                                s.vars.insert(n.clone(), V::Other);
                            }
                            s.last = None;
                            s
                        })
                        .collect()
                }
            }
            Stmt::Assign(Target::Name(n), op, e, line) => self.assign(st, n.clone(), op, e, *line),
            Stmt::Assign(Target::Field(Expr::Ident(o, _), f), op, e, line) if o == "self" => {
                self.assign(st, format!("self.{f}"), op, e, *line)
            }
            Stmt::Assign(t, _, e, _) => {
                let base = match t {
                    Target::Index(Expr::Ident(n, _), _) | Target::Field(Expr::Ident(n, _), _) => {
                        Some(n.clone())
                    }
                    _ => None,
                };
                self.eval(st, e)
                    .into_iter()
                    .map(|(mut s, _)| {
                        if let Some(b) = &base {
                            if s.vars.contains_key(b) {
                                s.vars.insert(b.clone(), V::Other);
                            }
                        }
                        s.last = None;
                        s
                    })
                    .collect()
            }
            Stmt::Expr(e, _) => self
                .eval(st, e)
                .into_iter()
                .map(|(mut s, v)| {
                    s.last = Some(v);
                    s
                })
                .collect(),
            Stmt::Return(e, _) => match e {
                Some(e) => self
                    .eval(st, e)
                    .into_iter()
                    .map(|(mut s, v)| {
                        s.ret = Some(v);
                        s
                    })
                    .collect(),
                None => {
                    st.ret = Some(V::Other);
                    vec![st]
                }
            },
            Stmt::If(c, a, b) => {
                let t = self.assume(st.clone(), c, true);
                let mut out = self.exec(t, a);
                let f = self.assume(st, c, false);
                match b {
                    Some(b) => out.extend(self.exec(f, b)),
                    None => out.extend(f.into_iter().map(|mut s| {
                        s.last = None;
                        s
                    })),
                }
                out
            }
            Stmt::While(c, invs, body, line) => {
                let mut out = self.while_loop(st, c, invs, body, *line);
                out.iter_mut().for_each(|s| s.last = None);
                out
            }
            Stmt::Loop(body) => {
                let mut out = self.while_loop(st, &Expr::Bool(true), &[], body, 0);
                out.iter_mut().for_each(|s| s.last = None);
                out
            }
            Stmt::For(names, iter, body, _) => {
                // constant integer ranges are unrolled; anything else is havocked
                if let (Expr::Range(a, b, incl), [name]) = (iter, names.as_slice()) {
                    let lo = self.eval(st.clone(), a);
                    let hi = self.eval(st.clone(), b);
                    if let ([(_, V::Num(x))], [(_, V::Num(y))]) = (lo.as_slice(), hi.as_slice()) {
                        if let (Some(x), Some(y)) = (x.aff.as_const(), y.aff.as_const()) {
                            let end = if *incl { y + 1.0 } else { y };
                            if integral(x) && integral(end) && end - x <= 256.0 {
                                let mut active = vec![st];
                                let mut done = Vec::new();
                                let mut i = x;
                                while i < end {
                                    for s in active.iter_mut() {
                                        s.vars.insert(
                                            name.clone(),
                                            V::Num(N {
                                                aff: Aff::konst(i),
                                                int: true,
                                            }),
                                        );
                                    }
                                    let mut next = Vec::new();
                                    for mut b in self.exec(active, body) {
                                        if b.ret.is_some() || b.brk == 1 {
                                            b.brk = 0;
                                            done.push(b);
                                        } else {
                                            b.brk = 0;
                                            next.push(b);
                                        }
                                    }
                                    active = self.cap(next);
                                    i += 1.0;
                                }
                                done.extend(active);
                                done.iter_mut().for_each(|s| s.last = None);
                                return done;
                            }
                        }
                    }
                }
                let mut names_set = HashMap::new();
                assigned(body, &mut names_set);
                self.havoc(&mut st, &names_set);
                let mut inner = st.clone();
                for n in names {
                    inner.vars.insert(n.clone(), V::Other);
                }
                if let (Expr::Range(a, b, incl), [name]) = (iter, names.as_slice()) {
                    let lo = self.eval(inner.clone(), a);
                    let hi = self.eval(inner.clone(), b);
                    if let ([(_, V::Num(x))], [(_, V::Num(y))]) = (lo.as_slice(), hi.as_slice()) {
                        let i = self.fresh(&mut inner, -INF, INF, true);
                        let _ = self.add_fact(&mut inner, i.aff.sub(&x.aff), false);
                        let _ = self.add_fact(&mut inner, y.aff.sub(&i.aff), !*incl);
                        inner.vars.insert(name.clone(), V::Num(i));
                    }
                }
                let mut out: Vec<State> = self
                    .exec(vec![inner], body)
                    .into_iter()
                    .filter(|s| s.ret.is_some())
                    .collect();
                st.last = None;
                out.push(st);
                out
            }
            Stmt::Break => {
                st.brk = 1;
                vec![st]
            }
            Stmt::Continue => {
                st.brk = 2;
                vec![st]
            }
            Stmt::Assert(e, _, text, line) => {
                let ok = self.prove(&st, e);
                self.record("assert", text, *line, ok);
                let mut out = self.assume(st, e, true);
                out.iter_mut().for_each(|s| s.last = None);
                out
            }
            Stmt::Try(a, _, b) => {
                self.unsupported.get_or_insert("try/catch".into());
                let mut names = HashMap::new();
                assigned(a, &mut names);
                assigned(b, &mut names);
                self.havoc(&mut st, &names);
                vec![st]
            }
            Stmt::When(..) | Stmt::Deadline(..) | Stmt::Always(..) => {
                self.unsupported
                    .get_or_insert("reactive rules inside a function".into());
                vec![st]
            }
            _ => {
                st.last = None;
                vec![st]
            }
        }
    }

    fn param_value(&mut self, st: &mut State, ty: &Option<TypeAnn>, default: &Option<Expr>) -> V {
        match ty {
            Some(TypeAnn::Name(n, false)) if n == "int" => V::Num(self.fresh(st, -INF, INF, true)),
            Some(TypeAnn::Name(n, false)) if n == "float" => {
                V::Num(self.fresh(st, -INF, INF, false))
            }
            Some(TypeAnn::Unit(_, false)) => V::Num(self.fresh(st, -INF, INF, false)),
            Some(TypeAnn::Name(n, false)) if n == "bool" => V::Bool(None),
            Some(_) => V::Other,
            None => match default {
                Some(Expr::Int(_)) => V::Num(self.fresh(st, -INF, INF, true)),
                Some(Expr::Float(_)) | Some(Expr::Quantity(..)) | None => {
                    V::Num(self.fresh(st, -INF, INF, false))
                }
                Some(Expr::Bool(_)) => V::Bool(None),
                Some(_) => V::Other,
            },
        }
    }

    fn function(&mut self, def: &Rc<FnDef>, owner: Option<&StructDef>) {
        self.ctx = match owner {
            Some(s) => format!("{}.{}()", s.name, def.name),
            None => format!("{}()", def.name),
        };
        self.steps = 0;
        self.unsupported = None;
        self.stack = vec![def.name.clone()];
        let first = self.obs.len();
        let mut st = State {
            vars: self.consts.clone(),
            ..State::default()
        };
        self.invariants = owner.map(|s| s.invariants.clone()).unwrap_or_default();
        if let Some(sd) = owner {
            for f in &sd.fields {
                let v = self.param_value(&mut st, &f.ty, &f.default);
                st.vars.insert(format!("self.{}", f.name), v);
            }
        }
        for p in def
            .params
            .iter()
            .filter(|p| !(owner.is_some() && p.name == "self"))
        {
            let v = self.param_value(&mut st, &p.ty, &p.default);
            st.vars.insert(p.name.clone(), v);
        }
        let initial = st.vars.clone();
        let mut states = vec![st];
        if def.name != "init" {
            let invs = self.invariants.clone();
            states = self.all(states, &invs, true);
        }
        states = self.all(states, &def.requires, true);
        if states.is_empty() && !def.requires.is_empty() {
            self.obs.push(Obligation {
                ctx: self.ctx.clone(),
                kind: "precondition",
                text: "the require clauses can be satisfied".into(),
                line: 0,
                status: Status::Refuted,
                detail: "no input satisfies every require".into(),
            });
            return;
        }
        let outs = self.exec(states, &def.body);
        for s in outs {
            let result = s.ret.clone().or(s.last.clone()).unwrap_or(V::Other);
            let mut post = s.clone();
            for p in &def.params {
                if let Some(v) = s.vars.get(&p.name) {
                    post.vars.insert(format!("{}'", p.name), v.clone());
                }
                if let Some(v) = initial.get(&p.name) {
                    post.vars.insert(p.name.clone(), v.clone());
                }
            }
            post.vars.insert("result'".into(), result);
            for e in &def.ensures {
                let ok = self.prove(&post, &e.expr);
                self.record("ensure", &format!("ensure {}", e.text), 0, ok);
            }
            for inv in self.invariants.clone() {
                let ok = self.prove(&s, &inv.expr);
                self.record(
                    "invariant",
                    &format!("preserves invariant {}", inv.text),
                    0,
                    ok,
                );
            }
        }
        if let Some(why) = self.unsupported.take() {
            for o in &mut self.obs[first..] {
                o.status = Status::Skipped;
                o.detail = format!("not analysed: uses {why}");
            }
        }
    }

    fn defaults(&mut self, sd: &StructDef) {
        if sd.invariants.is_empty() {
            return;
        }
        self.ctx = format!("{}()", sd.name);
        self.invariants.clear();
        let mut st = State {
            vars: self.consts.clone(),
            ..State::default()
        };
        // fields without a default are constructor arguments: any value
        for f in &sd.fields {
            let v = match &f.default {
                Some(d) => {
                    let mut r = self.eval(st.clone(), d);
                    if r.len() == 1 {
                        let (s, v) = r.remove(0);
                        st = s;
                        v
                    } else {
                        V::Other
                    }
                }
                None => self.param_value(&mut st, &f.ty, &None),
            };
            st.vars.insert(format!("self.{}", f.name), v);
        }
        let all_defaulted = sd.fields.iter().all(|f| f.default.is_some());
        for inv in &sd.invariants {
            let ok = self.prove(&st, &inv.expr);
            if all_defaulted || ok {
                self.record(
                    "invariant",
                    &format!("default fields satisfy invariant {}", inv.text),
                    0,
                    ok,
                );
            }
        }
    }

    fn machine(&mut self, m: &MachineDef) {
        let ctx = format!("machine {}", m.name);
        let Some(first) = m.states.first() else {
            return;
        };
        // reachability, treating every guard as possibly true
        let mut seen: HashSet<&str> = HashSet::from([first.name.as_str()]);
        let mut work = vec![first.name.as_str()];
        while let Some(s) = work.pop() {
            let st = m.states.iter().find(|x| x.name == s);
            let outs = st
                .iter()
                .flat_map(|x| x.transitions.iter())
                .chain(m.global.iter());
            for t in outs {
                if let Some(target) = &t.target {
                    if seen.insert(target.as_str()) {
                        work.push(target.as_str());
                    }
                }
            }
        }
        let unreachable: Vec<&str> = m
            .states
            .iter()
            .map(|s| s.name.as_str())
            .filter(|s| !seen.contains(s))
            .collect();
        self.obs.push(Obligation {
            ctx: ctx.clone(),
            kind: "machine",
            text: format!("every state is reachable from {}", first.name),
            line: first.line,
            status: if unreachable.is_empty() {
                Status::Proved
            } else {
                Status::Refuted
            },
            detail: if unreachable.is_empty() {
                String::new()
            } else {
                format!("unreachable: {}", unreachable.join(", "))
            },
        });
        // an unguarded transition hides every later transition on the same event
        let mut shadowed = Vec::new();
        for s in &m.states {
            let mut open: HashSet<&str> = HashSet::new();
            for t in &s.transitions {
                if open.contains(t.event.as_str()) {
                    shadowed.push(format!("{}.{} (line {})", s.name, t.event, t.line));
                }
                if t.guard.is_none() {
                    open.insert(t.event.as_str());
                }
            }
        }
        self.obs.push(Obligation {
            ctx: ctx.clone(),
            kind: "machine",
            text: "no transition is shadowed by an earlier unguarded one".into(),
            line: first.line,
            status: if shadowed.is_empty() {
                Status::Proved
            } else {
                Status::Refuted
            },
            detail: if shadowed.is_empty() {
                String::new()
            } else {
                format!("never taken: {}", shadowed.join(", "))
            },
        });
        // every non-terminal state can be left; terminal states are listed, not failed
        let terminal: Vec<&str> = m
            .states
            .iter()
            .filter(|s| {
                !s.transitions
                    .iter()
                    .chain(m.global.iter())
                    .any(|t| t.target.as_deref().is_some_and(|x| x != s.name))
            })
            .map(|s| s.name.as_str())
            .collect();
        self.obs.push(Obligation {
            ctx,
            kind: "machine",
            text: "terminal states".into(),
            line: first.line,
            status: Status::Proved,
            detail: if terminal.is_empty() {
                "none: every state has a way out".into()
            } else {
                format!("no way out of: {}", terminal.join(", "))
            },
        });
    }
}

/// Statically analyses every function, method and machine defined at the top level of `prog`.
pub fn prove_program(prog: &[Stmt]) -> Report {
    let mut p = Prover {
        syms: Vec::new(),
        fns: HashMap::new(),
        consts: HashMap::new(),
        obs: Vec::new(),
        index: HashMap::new(),
        ctx: String::new(),
        quiet: 0,
        stack: Vec::new(),
        steps: 0,
        unsupported: None,
        invariants: Vec::new(),
        fields: Vec::new(),
    };
    let _ = &p.fields;
    for s in prog {
        match s {
            Stmt::Fn(d) => {
                p.fns.insert(d.name.clone(), d.clone());
            }
            Stmt::Let(names, e, true, _) if names.len() == 1 => {
                let v = match e {
                    Expr::Int(i) => Some((*i as f64, true)),
                    Expr::Float(f) => Some((*f, false)),
                    Expr::Quantity(v, u) => Some((v * u.scale, false)),
                    Expr::Unary(op, x, _) if op == "-" => match &**x {
                        Expr::Int(i) => Some((-(*i as f64), true)),
                        Expr::Float(f) => Some((-*f, false)),
                        Expr::Quantity(v, u) => Some((-v * u.scale, false)),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some((c, int)) = v {
                    p.consts.insert(
                        names[0].clone(),
                        V::Num(N {
                            aff: Aff::konst(c),
                            int,
                        }),
                    );
                }
            }
            _ => {}
        }
    }
    for s in prog {
        match s {
            Stmt::Fn(d) => p.function(d, None),
            Stmt::Struct(sd) => {
                p.defaults(sd);
                for m in &sd.methods {
                    p.function(m, Some(sd));
                }
            }
            Stmt::Machine(m) => p.machine(m),
            _ => {}
        }
    }
    Report { obligations: p.obs }
}
