//! Ahead-of-time native code generation: emits LLVM IR for Tulpa's typed numeric subset.
//!
//! Eligible functions use numbers and booleans, locals, arithmetic, comparisons, `if`/`while`/
//! `for i in a..b`, `return`, `assert`, calls to other eligible functions and math builtins.
//! Semantics match the interpreter: integer `/` and `%` are Euclidean (remainder never negative), integer overflow and
//! division by zero are caught, and `require`/`ensure`/loop `invariant`/`assert` become runtime
//! checks that report the violated clause and abort. Each function is exported as
//! `tulpa_<name>` with C-compatible types (int → int64_t, float → double, bool → _Bool),
//! so the result links straight into C, C++ or Rust firmware.

use crate::ast::*;
use crate::units;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::rc::Rc;

const DIM_S: units::Dim = [0, 0, 1, 0, 0, 0, 0, 0];
const DIM_HZ: units::Dim = [0, 0, -1, 0, 0, 0, 0, 0];
const DIM_RAD: units::Dim = [0, 0, 0, 0, 0, 0, 0, 1];

/// Device classes with a native representation. Each is a small C struct declared in
/// `tulpa_hal.h` (`tulpa_gpio_t`, ...) and driven through the `tulpa_<dev>_*` runtime ABI.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Dev {
    Gpio,
    I2c,
    Spi,
    Uart,
    Pwm,
    Servo,
}

impl Dev {
    pub fn name(self) -> &'static str {
        match self {
            Dev::Gpio => "gpio",
            Dev::I2c => "i2c",
            Dev::Spi => "spi",
            Dev::Uart => "uart",
            Dev::Pwm => "pwm",
            Dev::Servo => "servo",
        }
    }
    pub fn display(self) -> &'static str {
        match self {
            Dev::Gpio => "GPIO",
            Dev::I2c => "I2C",
            Dev::Spi => "SPI",
            Dev::Uart => "UART",
            Dev::Pwm => "PWM",
            Dev::Servo => "Servo",
        }
    }
    fn from_name(n: &str) -> Option<Dev> {
        Some(match n {
            "GPIO" | "gpio" => Dev::Gpio,
            "I2C" | "i2c" => Dev::I2c,
            "SPI" | "spi" => Dev::Spi,
            "UART" | "uart" => Dev::Uart,
            "PWM" | "pwm" => Dev::Pwm,
            "Servo" | "servo" => Dev::Servo,
            _ => return None,
        })
    }
    /// LLVM layout; must match the C struct in tulpa_hal.h field for field.
    fn ll_def(self) -> &'static str {
        match self {
            Dev::Gpio => "{ i32, i32, ptr }",
            Dev::I2c => "{ i32, i32, ptr }",
            Dev::Spi => "{ i32, i32, i32, i32, ptr }",
            Dev::Uart => "{ ptr, i32, ptr }",
            Dev::Pwm => "{ i32, i32, i64, i64, i32, ptr }",
            Dev::Servo => "{ %tulpa.pwm, double, double, double, double, i32 }",
        }
    }
    const ALL: [Dev; 6] = [
        Dev::Gpio,
        Dev::I2c,
        Dev::Spi,
        Dev::Uart,
        Dev::Pwm,
        Dev::Servo,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    I64,
    F64,
    Bool,
    /// A physical quantity, held as an SI double; its dimension is checked at compile time.
    Q(units::Dim),
    /// A string literal (C string).
    Str,
    /// An instance of a struct (index into the module's struct table), passed by reference.
    Obj(u16),
    /// A device handle, passed by reference.
    Dev(Dev),
    /// A fixed-length array of integers (list literals, bus reads), passed by reference.
    Arr(u16),
    /// A state machine instance, passed by reference.
    Mach(u16),
    /// The current state of a machine (an index).
    State(u16),
}

fn qty(d: units::Dim) -> Ty {
    if d == units::DIMLESS {
        Ty::F64
    } else {
        Ty::Q(d)
    }
}

fn dim_of(t: Ty) -> units::Dim {
    match t {
        Ty::Q(d) => d,
        _ => units::DIMLESS,
    }
}

impl Ty {
    fn ll(self) -> &'static str {
        match self {
            Ty::I64 => "i64",
            Ty::F64 | Ty::Q(_) => "double",
            Ty::Bool => "i1",
            Ty::State(_) => "i32",
            Ty::Str | Ty::Obj(_) | Ty::Dev(_) | Ty::Arr(_) | Ty::Mach(_) => "ptr",
        }
    }
    /// Values held by reference: variables store a pointer, struct fields store the object inline.
    fn is_ref(self) -> bool {
        matches!(self, Ty::Obj(_) | Ty::Dev(_) | Ty::Arr(_) | Ty::Mach(_))
    }
    pub fn c(self) -> &'static str {
        match self {
            Ty::I64 => "int64_t",
            Ty::F64 | Ty::Q(_) => "double",
            Ty::Bool => "_Bool",
            Ty::Str => "const char *",
            Ty::State(_) => "int32_t",
            _ => "void *",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sig {
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    ret_annotated: bool,
    /// Declared unit (scale to SI, name) per parameter; C callers pass values in this unit.
    pub units: Vec<Option<(f64, String)>>,
    pub ret_unit: Option<(f64, String)>,
    /// Returns nothing (`void` in C): the program entry, machine constructors, checks.
    pub void: bool,
    /// C spelling of each parameter type and of the return type (filled in for the header).
    pub cparams: Vec<String>,
    pub cret: String,
    /// Parameters without annotations: typed from the first native call site.
    inferred: Vec<bool>,
    /// Default values (evaluated at the call site, as in the interpreter's semantics for
    /// constant defaults).
    defaults: Vec<Option<Expr>>,
}

pub struct Report {
    pub ir: String,
    pub compiled: Vec<(String, Sig)>,
    pub skipped: Vec<(String, String)>,
    /// C typedefs for the structs and machines in the module.
    pub c_types: String,
    /// The module uses devices or the runtime (print, sleep, ...): link tulpa_rt.c + a HAL.
    pub needs_runtime: bool,
    /// A program entry `void tulpa_entry(void)` was generated from the top-level statements.
    pub has_entry: bool,
}

impl Report {
    /// C header with prototypes for the compiled functions.
    pub fn header(&self, guard: &str) -> String {
        let mut h = format!("/* generated by tulpa — do not edit */\n#ifndef {guard}\n#define {guard}\n#include <stdint.h>\n#include <stdbool.h>\n");
        if self.needs_runtime {
            h.push_str("#include \"tulpa_hal.h\"\n");
        }
        h.push('\n');
        h.push_str(&self.c_types);
        for (name, sig) in &self.compiled {
            let ps: Vec<String> = sig
                .params
                .iter()
                .zip(sig.units.iter())
                .zip(sig.cparams.iter())
                .map(|(((n, t), u), c)| {
                    let sep = if c.ends_with('*') { "" } else { " " };
                    match (u, t) {
                        (Some((_, name)), _) => format!("{c}{sep}{n} /* [{name}] */"),
                        (None, Ty::Q(d)) => {
                            format!("{c}{sep}{n} /* [{}] SI */", units::dim_name(d))
                        }
                        _ => format!("{c}{sep}{n}"),
                    }
                })
                .collect();
            let ps = if ps.is_empty() {
                "void".to_string()
            } else {
                ps.join(", ")
            };
            let unit = match (&sig.ret_unit, sig.ret) {
                (Some((_, u)), _) if !sig.void => format!(" /* returns [{u}] */"),
                (None, Ty::Q(d)) if !sig.void => {
                    format!(" /* returns [{}] SI */", units::dim_name(&d))
                }
                _ => String::new(),
            };
            let ret = if sig.void {
                "void".to_string()
            } else {
                sig.cret.clone()
            };
            let sep = if ret.ends_with('*') { "" } else { " " };
            let _ = writeln!(h, "{ret}{sep}tulpa_{name}({ps});{unit}");
        }
        h.push_str(&format!("\n#endif /* {guard} */\n"));
        h
    }
}

type UnitInfo = Option<(f64, String)>;

/// A struct with a native layout.
struct SInfo {
    name: String,
    def: Rc<StructDef>,
    /// Declared field types (None = inferred from construction sites).
    fields: Vec<(String, Option<Ty>, UnitInfo)>,
}

/// A machine with a native layout: `{ i32 state, params... }`.
struct MInfo {
    name: String,
    def: Rc<MachineDef>,
    params: Vec<(String, Option<Ty>, UnitInfo)>,
}

/// What the code is compiled for.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Freestanding target (microcontroller): no libc `write`/`abort`; violations go to
    /// `tulpa_hal_panic`.
    pub embedded: bool,
    /// Allow `sim::*` calls (they link against the simulation HAL).
    pub sim: bool,
    /// Names bound to imported modules (`import "x" as m`): `m.f()` calls the merged `f`.
    pub module_aliases: Vec<String>,
}

/// Module-wide knowledge shared by every function being compiled.
struct Ctx {
    structs: Vec<SInfo>,
    machines: Vec<MInfo>,
    interfaces: HashSet<String>,
    /// interface -> the interfaces it extends
    iface_bases: HashMap<String, Vec<String>>,
    /// Field / machine-parameter types learned from construction sites.
    field_tys: RefCell<HashMap<(u16, usize), Ty>>,
    mparam_tys: RefCell<HashMap<(u16, usize), Ty>>,
    /// Event parameter types learned from call sites.
    event_tys: RefCell<HashMap<(u16, String), Vec<Ty>>>,
    /// Unannotated function / method parameter types learned from call sites.
    param_tys: RefCell<HashMap<(String, usize), Ty>>,
    /// Every function name in the program (to tell "not native" from "unknown").
    fn_names: HashSet<String>,
    /// Module globals, declared by top-level `let`s in the program entry.
    globals: RefCell<HashMap<String, Ty>>,
    /// Something new was learned during this pass (another pass may compile more).
    learned: RefCell<bool>,
    /// Top-level `always` monitors, in program order.
    monitors: Vec<(Contract, Option<Vec<Stmt>>)>,
    /// Struct names with invariants.
    opts: Options,
    /// Why a unit was dropped (for diagnostics in callers).
    dropped: RefCell<HashMap<String, String>>,
}

impl Ctx {
    fn sidx(&self, n: &str) -> Option<u16> {
        self.structs
            .iter()
            .position(|s| s.name == n)
            .map(|i| i as u16)
    }
    fn midx(&self, n: &str) -> Option<u16> {
        self.machines
            .iter()
            .position(|m| m.name == n)
            .map(|i| i as u16)
    }
    fn field_ty(&self, si: u16, k: usize) -> Option<Ty> {
        self.structs[si as usize].fields[k]
            .1
            .or_else(|| self.field_tys.borrow().get(&(si, k)).copied())
    }
    fn mparam_ty(&self, mi: u16, k: usize) -> Option<Ty> {
        self.machines[mi as usize].params[k]
            .1
            .or_else(|| self.mparam_tys.borrow().get(&(mi, k)).copied())
    }
    fn learn_field(&self, si: u16, k: usize, t: Ty) {
        if self.structs[si as usize].fields[k].1.is_none() {
            let mut m = self.field_tys.borrow_mut();
            if let std::collections::hash_map::Entry::Vacant(e) = m.entry((si, k)) {
                e.insert(t);
                *self.learned.borrow_mut() = true;
            }
        }
    }
    fn learn_mparam(&self, mi: u16, k: usize, t: Ty) {
        if self.machines[mi as usize].params[k].1.is_none() {
            let mut m = self.mparam_tys.borrow_mut();
            if let std::collections::hash_map::Entry::Vacant(e) = m.entry((mi, k)) {
                e.insert(t);
                *self.learned.borrow_mut() = true;
            }
        }
    }
    fn learn_global(&self, n: &str, t: Ty) -> Result<(), String> {
        let mut g = self.globals.borrow_mut();
        match g.get(n) {
            Some(old) if *old != t => Err(format!(
                "global '{n}' changes type ({} then {})",
                self.ty_name(*old),
                self.ty_name(t)
            )),
            Some(_) => Ok(()),
            None => {
                g.insert(n.to_string(), t);
                *self.learned.borrow_mut() = true;
                Ok(())
            }
        }
    }
    fn invariants(&self, si: u16) -> bool {
        !self.structs[si as usize].def.invariants.is_empty()
    }
    /// LLVM storage type of an object (inline in struct fields and in object slots).
    fn store_ty(&self, t: Ty) -> String {
        match t {
            Ty::Obj(i) => format!("%S.{}", self.structs[i as usize].name),
            Ty::Mach(i) => format!("%M.{}", self.machines[i as usize].name),
            Ty::Dev(d) => format!("%tulpa.{}", d.name()),
            Ty::Arr(n) => format!("[{n} x i64]"),
            other => other.ll().to_string(),
        }
    }
    fn c_ty(&self, t: Ty) -> String {
        match t {
            Ty::Obj(i) => format!("tulpa_{} *", self.structs[i as usize].name),
            Ty::Mach(i) => format!("tulpa_{} *", self.machines[i as usize].name),
            Ty::Dev(d) => format!("tulpa_{}_t *", d.name()),
            Ty::Arr(_) => "int64_t *".into(),
            other => other.c().to_string(),
        }
    }
    fn ty_name(&self, t: Ty) -> String {
        match t {
            Ty::I64 => "int".into(),
            Ty::F64 => "float".into(),
            Ty::Bool => "bool".into(),
            Ty::Q(d) => units::describe(&d),
            Ty::Str => "str".into(),
            Ty::Obj(i) => self.structs[i as usize].name.clone(),
            Ty::Mach(i) => format!("machine {}", self.machines[i as usize].name),
            Ty::Dev(d) => d.display().into(),
            Ty::Arr(n) => format!("list of {n} ints"),
            Ty::State(i) => format!("{}.state", self.machines[i as usize].name),
        }
    }
    /// Native type of an annotation.
    fn ann_ty(&self, t: &Option<TypeAnn>) -> Result<Option<(Ty, UnitInfo)>, String> {
        match t {
            None => Ok(None),
            Some(TypeAnn::Name(n, false)) => match n.as_str() {
                "int" => Ok(Some((Ty::I64, None))),
                "float" | "num" | "number" => Ok(Some((Ty::F64, None))),
                "bool" => Ok(Some((Ty::Bool, None))),
                "str" => Ok(Some((Ty::Str, None))),
                other => {
                    if let Some(d) = Dev::from_name(other)
                        .filter(|_| other.chars().next().is_some_and(|c| c.is_uppercase()))
                    {
                        Ok(Some((Ty::Dev(d), None)))
                    } else if let Some(i) = self.sidx(other) {
                        Ok(Some((Ty::Obj(i), None)))
                    } else if let Some(i) = self.midx(other) {
                        Ok(Some((Ty::Mach(i), None)))
                    } else if self.interfaces.contains(other) {
                        Err(format!("interface type '{other}' dispatches at run time; natively, declare the concrete struct type"))
                    } else {
                        Err(format!("type '{other}' has no native representation"))
                    }
                }
            },
            Some(TypeAnn::Name(_, true)) | Some(TypeAnn::Unit(_, true)) => {
                Err("optional types have no native representation".into())
            }
            Some(TypeAnn::Unit(u, false)) => {
                Ok(Some((qty(u.dim), Some((u.scale, u.name.clone())))))
            }
        }
    }
}

/// One function-like thing to compile.
#[derive(Clone)]
enum Unit {
    Fn(Rc<FnDef>),
    Method(u16, Rc<FnDef>),
    /// The program entry, from the top-level statements.
    Entry(Rc<Vec<Stmt>>),
    MachInit(u16),
    Event(u16, String),
    /// Struct invariant check.
    Inv(u16),
    /// Re-checks every armed `always` monitor.
    Always,
}

fn unit_sym(ctx: &Ctx, u: &Unit) -> String {
    match u {
        Unit::Fn(d) => d.name.clone(),
        Unit::Method(si, d) => format!("{}_{}", ctx.structs[*si as usize].name, d.name),
        Unit::Entry(_) => "entry".into(),
        Unit::MachInit(mi) => format!("{}__init", ctx.machines[*mi as usize].name),
        Unit::Event(mi, e) => format!("{}__on_{e}", ctx.machines[*mi as usize].name),
        Unit::Inv(si) => format!("{}__invariant", ctx.structs[*si as usize].name),
        Unit::Always => "always__check".into(),
    }
}

fn unit_label(ctx: &Ctx, u: &Unit) -> String {
    match u {
        Unit::Fn(d) => d.name.clone(),
        Unit::Method(si, d) => format!("{}.{}", ctx.structs[*si as usize].name, d.name),
        Unit::Entry(_) => "<program>".into(),
        Unit::MachInit(mi) => format!("{} (machine)", ctx.machines[*mi as usize].name),
        Unit::Event(mi, e) => format!("{}.{e} (event)", ctx.machines[*mi as usize].name),
        Unit::Inv(si) => format!("{} invariant", ctx.structs[*si as usize].name),
        Unit::Always => "always monitors".into(),
    }
}

/// Emits a module for every eligible top-level function in `prog` (host, no runtime HAL).
pub fn emit_module(prog: &[Stmt], module: &str) -> Report {
    emit_program(prog, module, &Options::default())
}

/// Syntactic type of a field default / initializer, when it is obvious without compiling.
fn syn_ty(ctx_structs: &[SInfo], machines: &[MInfo], e: &Expr) -> Option<Ty> {
    if let Some((_, t)) = literal(e) {
        return Some(t);
    }
    match e {
        Expr::Str(_) => Some(Ty::Str),
        Expr::Call(f, args, _) => match &**f {
            Expr::Path(m, n, _) if n == "new" => Dev::from_name(m).map(Ty::Dev),
            Expr::Ident(n, _) => ctx_structs
                .iter()
                .position(|s| &s.name == n)
                .map(|i| Ty::Obj(i as u16))
                .or_else(|| {
                    machines
                        .iter()
                        .position(|m| &m.name == n)
                        .map(|i| Ty::Mach(i as u16))
                })
                .or_else(|| {
                    let _ = args;
                    None
                }),
            _ => None,
        },
        Expr::List(items)
            if items
                .iter()
                .all(|x| matches!(literal(x), Some((_, Ty::I64)))) =>
        {
            Some(Ty::Arr(items.len() as u16))
        }
        Expr::Repeat(_, n) => match &**n {
            Expr::Int(k) if *k >= 0 && *k < 65536 => Some(Ty::Arr(*k as u16)),
            _ => None,
        },
        _ => None,
    }
}

/// Top-level statements that do something when the program runs (as opposed to definitions).
fn executable(s: &Stmt) -> bool {
    match s {
        Stmt::Fn(_)
        | Stmt::Struct(_)
        | Stmt::Interface(_)
        | Stmt::Machine(_)
        | Stmt::Import(..)
        | Stmt::MacroDef(..)
        | Stmt::Use(_) => false,
        Stmt::Let(names, e, true, _) => !(names.len() == 1 && literal(e).is_some()),
        _ => true,
    }
}

/// Emits a module for a whole program: top-level functions, structs and their methods,
/// machines, `always` monitors and (when there are top-level statements) a program entry
/// `void tulpa_entry(void)`. Whatever cannot be compiled is reported in `skipped`.
pub fn emit_program(prog: &[Stmt], module: &str, opts: &Options) -> Report {
    let mut consts: HashMap<String, (String, Ty)> = HashMap::new();
    consts.insert("PI".into(), (flit(std::f64::consts::PI), Ty::F64));
    consts.insert("TAU".into(), (flit(std::f64::consts::TAU), Ty::F64));
    consts.insert("E".into(), (flit(std::f64::consts::E), Ty::F64));
    consts.insert("HIGH".into(), ("1".into(), Ty::I64));
    consts.insert("LOW".into(), ("0".into(), Ty::I64));
    for s in prog {
        if let Stmt::Let(names, e, true, _) = s {
            if names.len() == 1 {
                if let Some(c) = literal(e) {
                    consts.insert(names[0].clone(), c);
                }
            }
        }
    }
    // ------------------------------------------------------------ module context
    let mut structs = Vec::new();
    let mut machines = Vec::new();
    let mut interfaces = HashSet::new();
    let mut iface_bases = HashMap::new();
    let mut monitors = Vec::new();
    for s in prog {
        match s {
            Stmt::Struct(d) => structs.push(SInfo {
                name: d.name.clone(),
                def: d.clone(),
                fields: d
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), None, None))
                    .collect(),
            }),
            Stmt::Machine(d) => machines.push(MInfo {
                name: d.name.clone(),
                def: d.clone(),
                params: d
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), None, None))
                    .collect(),
            }),
            Stmt::Interface(d) => {
                interfaces.insert(d.name.clone());
                iface_bases.insert(d.name.clone(), d.extends.clone());
            }
            Stmt::Always(c, h, _) => monitors.push((c.clone(), h.clone())),
            _ => {}
        }
    }
    let mut ctx = Ctx {
        structs,
        machines,
        interfaces,
        iface_bases,
        field_tys: RefCell::new(HashMap::new()),
        mparam_tys: RefCell::new(HashMap::new()),
        event_tys: RefCell::new(HashMap::new()),
        param_tys: RefCell::new(HashMap::new()),
        fn_names: prog
            .iter()
            .filter_map(|s| {
                if let Stmt::Fn(d) = s {
                    Some(d.name.clone())
                } else {
                    None
                }
            })
            .collect(),
        globals: RefCell::new(HashMap::new()),
        learned: RefCell::new(false),
        monitors,
        opts: opts.clone(),
        dropped: RefCell::new(HashMap::new()),
    };
    // declared field / parameter types (annotations, then obvious defaults)
    for si in 0..ctx.structs.len() {
        let def = ctx.structs[si].def.clone();
        for (k, f) in def.fields.iter().enumerate() {
            let t = match ctx.ann_ty(&f.ty) {
                Ok(Some((t, u))) => Some((t, u)),
                Ok(None) => f
                    .default
                    .as_ref()
                    .and_then(|d| syn_ty(&ctx.structs, &ctx.machines, d))
                    .map(|t| (t, None)),
                Err(_) => None,
            };
            if let Some((t, u)) = t {
                ctx.structs[si].fields[k].1 = Some(t);
                ctx.structs[si].fields[k].2 = u;
            }
        }
    }
    for mi in 0..ctx.machines.len() {
        let def = ctx.machines[mi].def.clone();
        for (k, p) in def.params.iter().enumerate() {
            let t = match ctx.ann_ty(&p.ty) {
                Ok(Some((t, u))) => Some((t, u)),
                Ok(None) => p
                    .default
                    .as_ref()
                    .and_then(|d| syn_ty(&ctx.structs, &ctx.machines, d))
                    .map(|t| (t, None)),
                Err(_) => None,
            };
            if let Some((t, u)) = t {
                ctx.machines[mi].params[k].1 = Some(t);
                ctx.machines[mi].params[k].2 = u;
            }
        }
    }
    let ctx = ctx;

    // ------------------------------------------------------------ units and signatures
    let mut units: Vec<Unit> = Vec::new();
    let top: Vec<Stmt> = prog.iter().filter(|s| executable(s)).cloned().collect();
    if !top.is_empty() {
        units.push(Unit::Entry(Rc::new(prog.to_vec())));
    }
    for s in prog {
        if let Stmt::Fn(d) = s {
            units.push(Unit::Fn(d.clone()));
        }
    }
    for (si, st) in ctx.structs.iter().enumerate() {
        for m in &st.def.methods {
            units.push(Unit::Method(si as u16, m.clone()));
        }
        if !st.def.invariants.is_empty() {
            units.push(Unit::Inv(si as u16));
        }
    }
    for (mi, m) in ctx.machines.iter().enumerate() {
        units.push(Unit::MachInit(mi as u16));
        let mut evs: Vec<String> = Vec::new();
        for t in m
            .def
            .states
            .iter()
            .flat_map(|s| s.transitions.iter())
            .chain(m.def.global.iter())
        {
            if !evs.contains(&t.event) {
                evs.push(t.event.clone());
            }
        }
        for e in evs {
            units.push(Unit::Event(mi as u16, e));
        }
    }
    if !ctx.monitors.is_empty() {
        units.push(Unit::Always);
    }

    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut sigs: HashMap<String, Sig> = HashMap::new();
    let mk = |params: Vec<(String, Ty)>, units_: Vec<UnitInfo>, ret: Ty, void: bool| Sig {
        params,
        ret,
        ret_annotated: true,
        units: units_,
        ret_unit: None,
        void,
        cparams: vec![],
        cret: String::new(),
        inferred: vec![],
        defaults: vec![],
    };
    for u in &units {
        let sym = unit_sym(&ctx, u);
        let sig = match u {
            Unit::Fn(f) | Unit::Method(_, f) => {
                let mut params = Vec::new();
                let mut punits = Vec::new();
                let mut inferred = Vec::new();
                let mut defaults = Vec::new();
                let mut bad = None;
                for (i, p) in f.params.iter().enumerate() {
                    if let (Unit::Method(si, _), 0) = (u, i) {
                        if p.name != "self" {
                            bad = Some("a method's first parameter must be self".to_string());
                            break;
                        }
                        params.push(("self".to_string(), Ty::Obj(*si)));
                        punits.push(None);
                        inferred.push(false);
                        defaults.push(None);
                        continue;
                    }
                    defaults.push(p.default.clone());
                    match ctx.ann_ty(&p.ty) {
                        Ok(t) => {
                            inferred.push(t.is_none());
                            let (ty, un) = t.unwrap_or((Ty::F64, None));
                            params.push((p.name.clone(), ty));
                            punits.push(un);
                        }
                        Err(e) => {
                            bad = Some(format!("parameter '{}': {e}", p.name));
                            break;
                        }
                    }
                }
                let ret = match ctx.ann_ty(&f.ret) {
                    Ok(t) => t,
                    Err(e) => {
                        bad = bad.or(Some(format!("return type: {e}")));
                        None
                    }
                };
                match bad {
                    Some(b) => {
                        skipped.push((unit_label(&ctx, u), b));
                        continue;
                    }
                    None => Sig {
                        params,
                        ret: ret.as_ref().map(|r| r.0).unwrap_or(Ty::F64),
                        ret_annotated: ret.is_some(),
                        units: punits,
                        ret_unit: ret.and_then(|r| r.1),
                        void: false,
                        cparams: vec![],
                        cret: String::new(),
                        inferred,
                        defaults,
                    },
                }
            }
            Unit::Entry(_) | Unit::Always => mk(vec![], vec![], Ty::I64, true),
            Unit::MachInit(mi) => mk(
                vec![("self".into(), Ty::Mach(*mi))],
                vec![None],
                Ty::I64,
                true,
            ),
            Unit::Inv(si) => mk(
                vec![("self".into(), Ty::Obj(*si))],
                vec![None],
                Ty::I64,
                true,
            ),
            Unit::Event(mi, _) => mk(
                vec![("self".into(), Ty::Mach(*mi))],
                vec![None],
                Ty::Bool,
                false,
            ),
        };
        sigs.insert(sym, sig);
    }

    // ------------------------------------------------------------ fixpoint
    // Compile everything; drop what fails (and, transitively, its callers); refine inferred
    // return types; learn struct field / global / event types from use sites. Repeat until stable.
    let mut bodies: Vec<(String, String)>;
    let mut strings: Vec<String>;
    let mut decls: HashSet<&'static str>;
    let mut alive: Vec<Unit> = units
        .iter()
        .filter(|u| sigs.contains_key(&unit_sym(&ctx, u)))
        .cloned()
        .collect();
    let entry_dead = RefCell::new(false);
    let mut uses_rt;
    let mut rounds = 0;
    loop {
        rounds += 1;
        *ctx.learned.borrow_mut() = false;
        // event signatures follow what the call sites pass
        for u in &alive {
            if let Unit::Event(mi, e) = u {
                let m = &ctx.machines[*mi as usize];
                let n = m
                    .def
                    .states
                    .iter()
                    .flat_map(|s| s.transitions.iter())
                    .chain(m.def.global.iter())
                    .find(|t| &t.event == e)
                    .map(|t| t.params.len())
                    .unwrap_or(0);
                let tys = ctx
                    .event_tys
                    .borrow()
                    .get(&(*mi, e.clone()))
                    .cloned()
                    .unwrap_or_else(|| vec![Ty::F64; n]);
                let sym = unit_sym(&ctx, u);
                if let Some(s) = sigs.get_mut(&sym) {
                    s.params.truncate(1);
                    s.units.truncate(1);
                    for (i, t) in tys.iter().enumerate() {
                        s.params.push((format!("p{i}"), *t));
                        s.units.push(None);
                    }
                }
            }
        }
        for ((sym, k), t) in ctx.param_tys.borrow().iter() {
            if let Some(s) = sigs.get_mut(sym) {
                if s.inferred.get(*k).copied().unwrap_or(false) {
                    s.params[*k].1 = *t;
                }
            }
        }
        bodies = Vec::new();
        strings = Vec::new();
        decls = HashSet::new();
        uses_rt = false;
        let mut failures: Vec<(Unit, String)> = Vec::new();
        let mut observed: HashMap<String, Ty> = HashMap::new();
        for u in &alive {
            let sym = unit_sym(&ctx, u);
            if !sigs.contains_key(&sym) {
                continue;
            }
            let mut g = FnGen::new(&ctx, &sigs, &consts, &mut strings, &mut decls, &sym);
            g.entry_dead = *entry_dead.borrow();
            match g.compile(u) {
                Ok(()) => {
                    uses_rt |= g.uses_rt;
                    observed.insert(sym.clone(), g.observed_ret.unwrap_or(Ty::F64));
                    let mut text = g.out;
                    text.push_str(&g.module_extra);
                    bodies.push((sym, text));
                }
                Err(e) => failures.push((u.clone(), e)),
            }
        }
        let learned = *ctx.learned.borrow();
        if std::env::var_os("TULPA_DEBUG_NATIVE").is_some() {
            eprintln!(
                "round {rounds}: learned={learned} observed={:?} failures={:?}",
                observed,
                failures
                    .iter()
                    .map(|(u, e)| (unit_sym(&ctx, u), e.clone()))
                    .collect::<Vec<_>>()
            );
        }
        let mut changed = false;
        // refine inferred return types first: a caller that failed on a provisional
        // signature gets another pass before it is dropped
        let mut refined = false;
        for (n, t) in &observed {
            if let Some(s) = sigs.get_mut(n) {
                if !s.ret_annotated && !s.void && s.ret != *t {
                    s.ret = *t;
                    refined = true;
                }
            }
        }
        if !((learned || refined) && !failures.is_empty() && rounds < 40) {
            // callers of a unit that failed in this same round learn the reason next round
            let real = failures
                .iter()
                .any(|(_, e)| !e.contains("not native (yet)"));
            let failures: Vec<(Unit, String)> = if real {
                failures
                    .into_iter()
                    .filter(|(_, e)| !e.contains("not native (yet)"))
                    .collect()
            } else {
                failures
            };
            if real {
                changed = true;
            }
            for (u, e) in failures {
                let sym = unit_sym(&ctx, &u);
                sigs.remove(&sym);
                if matches!(u, Unit::Entry(_)) {
                    *entry_dead.borrow_mut() = true;
                }
                ctx.dropped.borrow_mut().insert(sym, e.clone());
                skipped.push((unit_label(&ctx, &u), e));
                changed = true;
            }
        }
        changed |= refined;
        alive.retain(|u| sigs.contains_key(&unit_sym(&ctx, u)));
        if (!changed && !learned) || rounds > 60 {
            break;
        }
    }

    // ------------------------------------------------------------ assemble the module
    let mut ir = String::new();
    let _ = writeln!(
        ir,
        "; ModuleID = '{module}'\n; generated by tulpa {}\nsource_filename = \"{module}\"\n",
        env!("CARGO_PKG_VERSION")
    );
    for (i, s) in strings.iter().enumerate() {
        let bytes = s.as_bytes();
        let mut esc = String::new();
        for b in bytes {
            if b.is_ascii_graphic() && *b != b'"' && *b != b'\\' || *b == b' ' {
                esc.push(*b as char);
            } else {
                let _ = write!(esc, "\\{b:02X}");
            }
        }
        let _ = writeln!(
            ir,
            "@.msg{i} = private unnamed_addr constant [{} x i8] c\"{esc}\\00\"",
            bytes.len() + 1
        );
    }
    // types: devices, then structs and machines whose layout is fully known
    let complete_struct =
        |si: usize| (0..ctx.structs[si].fields.len()).all(|k| ctx.field_ty(si as u16, k).is_some());
    let complete_mach = |mi: usize| {
        (0..ctx.machines[mi].params.len()).all(|k| ctx.mparam_ty(mi as u16, k).is_some())
    };
    if uses_rt || !ctx.structs.is_empty() || !ctx.machines.is_empty() {
        for d in Dev::ALL {
            let _ = writeln!(ir, "%tulpa.{} = type {}", d.name(), d.ll_def());
        }
    }
    for si in 0..ctx.structs.len() {
        if complete_struct(si) {
            let fs: Vec<String> = (0..ctx.structs[si].fields.len())
                .map(|k| ctx.store_ty(ctx.field_ty(si as u16, k).unwrap()))
                .collect();
            let body = if fs.is_empty() {
                "{ i8 }".to_string()
            } else {
                format!("{{ {} }}", fs.join(", "))
            };
            let _ = writeln!(ir, "%S.{} = type {body}", ctx.structs[si].name);
        }
    }
    for mi in 0..ctx.machines.len() {
        if complete_mach(mi) {
            let mut fs = vec!["i32".to_string()];
            fs.extend(
                (0..ctx.machines[mi].params.len())
                    .map(|k| ctx.store_ty(ctx.mparam_ty(mi as u16, k).unwrap())),
            );
            let _ = writeln!(
                ir,
                "%M.{} = type {{ {} }}",
                ctx.machines[mi].name,
                fs.join(", ")
            );
            // state names, for printing `m.state`
            let names: Vec<String> = ctx.machines[mi]
                .def
                .states
                .iter()
                .map(|st| format!("ptr @mn.{}.{}", ctx.machines[mi].name, st.name))
                .collect();
            for st in &ctx.machines[mi].def.states {
                let _ = writeln!(
                    ir,
                    "@mn.{}.{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"",
                    ctx.machines[mi].name,
                    st.name,
                    st.name.len() + 1,
                    st.name
                );
            }
            let _ = writeln!(
                ir,
                "@mnames.{} = private constant [{} x ptr] [{}]",
                ctx.machines[mi].name,
                names.len(),
                names.join(", ")
            );
        }
    }
    let entry_ok = sigs.contains_key("entry");
    if entry_ok {
        let g = ctx.globals.borrow();
        let mut names: Vec<&String> = g.keys().collect();
        names.sort();
        for n in names {
            let _ = writeln!(
                ir,
                "@g.{} = internal global {} zeroinitializer",
                mangle(n),
                g[n].ll()
            );
        }
    }
    if !ctx.monitors.is_empty() {
        for k in 0..ctx.monitors.len() {
            let _ = writeln!(ir, "@tulpa.armed.{k} = internal global i1 false");
        }
        let _ = writeln!(ir, "@tulpa.checking = internal global i1 false");
    }
    ir.push_str(if opts.embedded {
        RUNTIME_EMBEDDED
    } else {
        RUNTIME
    });
    let mut decl_list: Vec<&&str> = decls.iter().collect();
    decl_list.sort();
    for d in decl_list {
        let _ = writeln!(ir, "{d}");
    }
    ir.push('\n');
    for (_, b) in &bodies {
        ir.push_str(b);
        ir.push('\n');
    }

    // ------------------------------------------------------------ report
    let mut compiled: Vec<(String, Sig)> = Vec::new();
    for u in &units {
        let sym = unit_sym(&ctx, u);
        if matches!(u, Unit::Inv(_) | Unit::Always) {
            continue;
        }
        if let Some(s) = sigs.get(&sym) {
            let mut s = s.clone();
            s.cparams = s.params.iter().map(|(_, t)| ctx.c_ty(*t)).collect();
            s.cret = ctx.c_ty(s.ret);
            compiled.push((sym, s));
        }
    }
    // C typedefs in dependency order
    let mut c_types = String::new();
    let mut done: HashSet<String> = HashSet::new();
    let mut progress = true;
    while progress {
        progress = false;
        for si in 0..ctx.structs.len() {
            let name = format!("S{si}");
            if done.contains(&name) || !complete_struct(si) {
                continue;
            }
            let deps_ok =
                (0..ctx.structs[si].fields.len()).all(|k| match ctx.field_ty(si as u16, k) {
                    Some(Ty::Obj(j)) => done.contains(&format!("S{j}")),
                    Some(Ty::Mach(j)) => done.contains(&format!("M{j}")),
                    _ => true,
                });
            if !deps_ok {
                continue;
            }
            let st = &ctx.structs[si];
            let _ = writeln!(c_types, "typedef struct tulpa_{} {{", st.name);
            if st.fields.is_empty() {
                c_types.push_str("    char _empty;\n");
            }
            for (k, (fname, _, u)) in st.fields.iter().enumerate() {
                let t = ctx.field_ty(si as u16, k).unwrap();
                let decl = c_field(&ctx, t, fname);
                let note = match (u, t) {
                    (Some((_, un)), Ty::Q(_)) => format!(" /* SI; declared [{un}] */"),
                    (None, Ty::Q(d)) => format!(" /* [{}] */", units::dim_name(&d)),
                    _ => String::new(),
                };
                let _ = writeln!(c_types, "    {decl};{note}");
            }
            let _ = writeln!(c_types, "}} tulpa_{};\n", st.name);
            done.insert(name);
            progress = true;
        }
        for mi in 0..ctx.machines.len() {
            let name = format!("M{mi}");
            if done.contains(&name) || !complete_mach(mi) {
                continue;
            }
            let m = &ctx.machines[mi];
            let _ = writeln!(c_types, "typedef struct tulpa_{} {{", m.name);
            let states: Vec<String> = m
                .def
                .states
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{i} = {}", s.name))
                .collect();
            let _ = writeln!(c_types, "    int32_t state; /* {} */", states.join(", "));
            for (k, (pname, _, _)) in m.params.iter().enumerate() {
                let t = ctx.mparam_ty(mi as u16, k).unwrap();
                let _ = writeln!(c_types, "    {};", c_field(&ctx, t, pname));
            }
            let _ = writeln!(c_types, "}} tulpa_{};\n", m.name);
            done.insert(name);
            progress = true;
        }
    }
    skipped.sort();
    skipped.dedup_by(|a, b| a.0 == b.0);
    Report {
        ir,
        compiled,
        skipped,
        c_types,
        needs_runtime: uses_rt,
        has_entry: entry_ok,
    }
}

/// "a"/"an" + phrase, for messages.
fn a_an(s: &str) -> String {
    let vowel = s.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c));
    format!("{} {s}", if vowel { "an" } else { "a" })
}

fn c_field(ctx: &Ctx, t: Ty, name: &str) -> String {
    match t {
        Ty::Obj(i) => format!("tulpa_{} {name}", ctx.structs[i as usize].name),
        Ty::Mach(i) => format!("tulpa_{} {name}", ctx.machines[i as usize].name),
        Ty::Dev(d) => format!("tulpa_{}_t {name}", d.name()),
        Ty::Arr(n) => format!("int64_t {name}[{n}]"),
        Ty::Str => format!("const char *{name}"),
        other => format!("{} {name}", other.c()),
    }
}

/// Identifier-safe spelling of a Tulpa name for LLVM symbols.
fn mangle(n: &str) -> String {
    n.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

const RUNTIME: &str = r#"
declare i64 @strlen(ptr)
declare i64 @write(i32, ptr, i64)
declare void @abort() noreturn
declare double @llvm.pow.f64(double, double)
declare double @llvm.powi.f64.i32(double, i32)
declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)

; Contract/assertion/overflow failures land here. Override it (it is weak) to e.g. trigger
; an emergency stop on a microcontroller instead of aborting.
@.nl = private unnamed_addr constant [2 x i8] c"\0A\00"

define weak void @tulpa_violation(ptr %msg) noreturn {
  %n = call i64 @strlen(ptr %msg)
  %w = call i64 @write(i32 2, ptr %msg, i64 %n)
  %w2 = call i64 @write(i32 2, ptr @.nl, i64 1)
  call void @abort()
  unreachable
}
"#;

/// Freestanding targets have no `write`/`abort`: violations go to the board's panic handler
/// (tulpa_hal.h), which typically stops the actuators and halts or resets.
const RUNTIME_EMBEDDED: &str = r#"
declare double @llvm.pow.f64(double, double)
declare double @llvm.powi.f64.i32(double, i32)
declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)
declare void @tulpa_hal_panic(ptr) noreturn

define weak void @tulpa_violation(ptr %msg) noreturn {
  call void @tulpa_hal_panic(ptr %msg)
  unreachable
}
"#;

fn literal(e: &Expr) -> Option<(String, Ty)> {
    match e {
        Expr::Int(n) => Some((n.to_string(), Ty::I64)),
        Expr::Float(f) => Some((flit(*f), Ty::F64)),
        Expr::Bool(b) => Some(((*b as u8).to_string(), Ty::Bool)),
        Expr::Quantity(v, u) => Some((flit(v * u.scale), qty(u.dim))),
        Expr::Unary(op, x, _) if op == "-" => match &**x {
            Expr::Int(n) => Some(((-n).to_string(), Ty::I64)),
            Expr::Float(f) => Some((flit(-f), Ty::F64)),
            _ => None,
        },
        _ => None,
    }
}

type G<T> = Result<T, String>;

struct FnGen<'a> {
    ctx: &'a Ctx,
    sigs: &'a HashMap<String, Sig>,
    consts: &'a HashMap<String, (String, Ty)>,
    strings: &'a mut Vec<String>,
    decls: &'a mut HashSet<&'static str>,
    /// Symbol being compiled (without the `tulpa_` prefix).
    name: String,
    out: String,
    /// Module-level definitions this unit needs (object storage for the program entry).
    module_extra: String,
    tmp: usize,
    lbl: usize,
    scopes: Vec<HashMap<String, (String, Ty)>>,
    initial: HashMap<String, (String, Ty)>,
    loops: Vec<(String, String)>,
    terminated: bool,
    ret: Ty,
    void: bool,
    observed_ret: Option<Ty>,
    ensures: Vec<Contract>,
    entry_allocas: String,
    /// Compiling the program entry: objects get static storage, top-level `let`s are globals.
    in_entry: bool,
    /// Inside a machine's bodies: its parameters are reachable by name.
    implicit: Option<(String, u16)>,
    /// Compiling the always-monitor check itself (no recursive checks).
    in_always: bool,
    /// The program entry could not be compiled, so globals are never initialized.
    entry_dead: bool,
    uses_rt: bool,
    objn: usize,
    monitor_seen: usize,
    /// The current block follows a `return` (nothing branches to it).
    after_return: bool,
}

impl<'a> FnGen<'a> {
    fn new(
        ctx: &'a Ctx,
        sigs: &'a HashMap<String, Sig>,
        consts: &'a HashMap<String, (String, Ty)>,
        strings: &'a mut Vec<String>,
        decls: &'a mut HashSet<&'static str>,
        name: &str,
    ) -> Self {
        let sig = &sigs[name];
        FnGen {
            ctx,
            sigs,
            consts,
            strings,
            decls,
            name: name.into(),
            out: String::new(),
            module_extra: String::new(),
            tmp: 0,
            lbl: 0,
            scopes: vec![],
            initial: HashMap::new(),
            loops: vec![],
            terminated: false,
            ret: sig.ret,
            void: sig.void,
            observed_ret: None,
            ensures: vec![],
            entry_allocas: String::new(),
            in_entry: false,
            implicit: None,
            in_always: false,
            entry_dead: false,
            uses_rt: false,
            objn: 0,
            monitor_seen: 0,
            after_return: false,
        }
    }

    fn t(&mut self) -> String {
        self.tmp += 1;
        format!("%t{}", self.tmp)
    }
    fn label(&mut self, base: &str) -> String {
        self.lbl += 1;
        format!("{base}{}", self.lbl)
    }
    fn line(&mut self, s: String) {
        if !self.terminated {
            self.out.push_str("  ");
            self.out.push_str(&s);
            self.out.push('\n');
        }
    }
    fn term(&mut self, s: String) {
        self.line(s);
        self.terminated = true;
    }
    fn start(&mut self, l: &str) {
        self.out.push_str(&format!("{l}:\n"));
        self.terminated = false;
        self.after_return = false;
    }
    fn intern(&mut self, text: String) -> String {
        let i = match self.strings.iter().position(|s| *s == text) {
            Some(i) => i,
            None => {
                self.strings.push(text);
                self.strings.len() - 1
            }
        };
        format!("@.msg{i}")
    }
    fn msg(&mut self, text: String) -> String {
        self.intern(format!("tulpa: {text}"))
    }
    /// A string literal as a C string constant.
    fn cstr(&mut self, text: &str) -> String {
        self.intern(text.to_string())
    }
    /// Declares a runtime function (tulpa_rt.c / the board HAL).
    fn rt(&mut self, d: &'static str) {
        self.uses_rt = true;
        self.decls.insert(d);
    }
    /// Branches to a violation report unless `cond` (i1) holds.
    fn check(&mut self, cond: &str, text: String) {
        let ok = self.label("ok");
        let bad = self.label("fail");
        self.term(format!("br i1 {cond}, label %{ok}, label %{bad}"));
        self.start(&bad);
        let m = self.msg(text);
        self.line(format!("call void @tulpa_violation(ptr {m})"));
        self.term("unreachable".into());
        self.start(&ok);
    }
    fn var(&self, n: &str) -> Option<(String, Ty)> {
        self.scopes.iter().rev().find_map(|s| s.get(n).cloned())
    }
    fn declare(&mut self, n: &str, ty: Ty) -> String {
        self.lbl += 1;
        let ptr = format!("%v{}.{}", self.lbl, mangle(n));
        self.entry_allocas
            .push_str(&format!("  {ptr} = alloca {}\n", ty.ll()));
        self.scopes
            .last_mut()
            .unwrap()
            .insert(n.to_string(), (ptr.clone(), ty));
        ptr
    }
    fn tyn(&self, t: Ty) -> String {
        self.ctx.ty_name(t)
    }
    fn i32_of(&mut self, v: (String, Ty), what: &str) -> G<String> {
        let x = self
            .convert(v, Ty::I64)
            .map_err(|e| format!("{what}: {e}"))?;
        let t = self.t();
        self.line(format!("{t} = trunc i64 {x} to i32"));
        Ok(t)
    }
    fn f64_of(&mut self, v: (String, Ty), what: &str) -> G<String> {
        if let Ty::Q(d) = v.1 {
            return Err(format!(
                "{what} expects a plain number, got {}",
                units::describe(&d)
            ));
        }
        self.convert(v, Ty::F64).map_err(|e| format!("{what}: {e}"))
    }

    /// A global variable of the program (declared by a top-level `let`).
    fn global(&self, n: &str) -> G<Option<(String, Ty)>> {
        let g = self.ctx.globals.borrow();
        match g.get(n) {
            Some(t) if self.entry_dead => {
                let _ = t;
                Err(format!(
                    "uses global '{n}', which is set up by top-level code that is not native"
                ))
            }
            Some(t) => Ok(Some((format!("@g.{}", mangle(n)), *t))),
            None => Ok(None),
        }
    }

    fn convert(&mut self, v: (String, Ty), to: Ty) -> G<String> {
        Ok(match (v.1, to) {
            (a, b) if a == b => v.0,
            (Ty::State(_), Ty::I64) => {
                let t = self.t();
                self.line(format!("{t} = sext i32 {} to i64", v.0));
                t
            }
            (a, b)
                if a.is_ref()
                    || b.is_ref()
                    || matches!(a, Ty::Str | Ty::State(_))
                    || matches!(b, Ty::Str | Ty::State(_)) =>
            {
                return Err(format!(
                    "{} value is used where {} is expected",
                    a_an(&self.tyn(a)),
                    self.tyn(b)
                ))
            }
            (Ty::Q(a), Ty::Q(b)) => {
                return Err(format!(
                    "unit mismatch: {} where {} is expected",
                    units::describe(&a),
                    units::describe(&b)
                ))
            }
            (Ty::Q(_), Ty::Bool) => {
                let t = self.t();
                self.line(format!("{t} = fcmp une double {}, 0.0", v.0));
                t
            }
            (Ty::Q(d), _) => {
                return Err(format!(
                    "{} value is used where a plain number is expected",
                    a_an(&units::describe(&d))
                ))
            }
            (_, Ty::Q(_)) if is_zero(&v.0) => "0.0".to_string(),
            (_, Ty::Q(d)) => {
                return Err(format!(
                "a plain number is used where {} is expected (write it with a unit, e.g. 2[{}])",
                units::describe(&d),
                units::dim_name(&d)
            ))
            }
            (Ty::I64, Ty::F64) => {
                let t = self.t();
                self.line(format!("{t} = sitofp i64 {} to double", v.0));
                t
            }
            (Ty::Bool, Ty::I64) => {
                let t = self.t();
                self.line(format!("{t} = zext i1 {} to i64", v.0));
                t
            }
            (Ty::Bool, Ty::F64) => {
                let t = self.t();
                self.line(format!("{t} = uitofp i1 {} to double", v.0));
                t
            }
            (Ty::I64, Ty::Bool) => {
                let t = self.t();
                self.line(format!("{t} = icmp ne i64 {}, 0", v.0));
                t
            }
            (Ty::F64, Ty::Bool) => {
                let t = self.t();
                self.line(format!("{t} = fcmp une double {}, 0.0", v.0));
                t
            }
            (Ty::F64, Ty::I64) => {
                return Err("a float value would be stored where an int is expected".into())
            }
            (a, b) => {
                return Err(format!(
                    "{} value is used where {} is expected",
                    a_an(&self.tyn(a)),
                    self.tyn(b)
                ))
            }
        })
    }

    // ------------------------------------------------------------------ units

    fn compile(&mut self, u: &Unit) -> G<()> {
        match u {
            Unit::Fn(d) => self.function(d),
            Unit::Method(_, d) => self.function(d),
            Unit::Entry(prog) => self.entry(prog),
            Unit::MachInit(mi) => self.mach_init(*mi),
            Unit::Event(mi, e) => self.event(*mi, e),
            Unit::Inv(si) => self.invariant_fn(*si),
            Unit::Always => self.always_fn(),
        }
    }

    fn params_ll(&self) -> Vec<String> {
        self.sigs[&self.name]
            .params
            .iter()
            .map(|(n, t)| format!("{} %a.{}", t.ll(), mangle(n)))
            .collect()
    }

    /// Wraps the generated body into a definition (allocas go first in the entry block).
    fn finish(&mut self, body_prefix: String) {
        let mut body = body_prefix;
        body.push_str(&std::mem::take(&mut self.out));
        let body = body.replacen("entry:\n", &format!("entry:\n{}", self.entry_allocas), 1);
        let ret = if self.void { "void" } else { self.ret.ll() };
        self.out = format!(
            "define {ret} @tulpa_{}({}) {{\n{body}}}\n",
            self.name,
            self.params_ll().join(", ")
        );
    }

    fn function(&mut self, def: &FnDef) -> G<()> {
        let sig = self.sigs[&self.name].clone();
        self.scopes.push(HashMap::new());
        let prefix = std::mem::take(&mut self.out);
        self.start("entry");
        for ((n, t), u) in sig.params.iter().zip(sig.units.iter()) {
            let ptr = self.declare(n, *t);
            // values arrive in the declared unit; work internally in SI
            let v = match u {
                Some((scale, _)) if (*scale - 1.0).abs() > 0.0 => {
                    let si = self.t();
                    self.line(format!(
                        "{si} = fmul double %a.{}, {}",
                        mangle(n),
                        flit(*scale)
                    ));
                    si
                }
                _ => format!("%a.{}", mangle(n)),
            };
            self.line(format!("store {} {v}, ptr {ptr}", t.ll()));
            if !t.is_ref() {
                self.initial.insert(n.clone(), (v, *t));
            }
        }
        for r in &def.requires {
            let c = self.cond(&r.expr)?;
            self.check(
                &c,
                format!(
                    "precondition violated in {}(): require {}",
                    def.name, r.text
                ),
            );
        }
        self.ensures = def.ensures.clone();
        let v = self.block(&def.body, true)?;
        if !self.terminated {
            if self.after_return {
                self.term("unreachable".into());
            } else {
                let v = v.unwrap_or(("0".into(), Ty::I64));
                self.emit_return(v)?;
            }
        }
        self.finish(prefix);
        Ok(())
    }

    fn emit_return(&mut self, v: (String, Ty)) -> G<()> {
        if self.void {
            self.term("ret void".into());
            return Ok(());
        }
        if v.1.is_ref() && v.0.starts_with("%obj") {
            return Err(format!(
                "{}() returns a {} created inside it; natively, objects live where they are constructed, so create it at top level or pass it in",
                self.name,
                self.tyn(v.1)
            ));
        }
        let obs = match (self.observed_ret, v.1) {
            (None, t) => t,
            (Some(a), b) if a == b => a,
            (Some(a), b)
                if a.is_ref() || b.is_ref() || matches!(a, Ty::Str) || matches!(b, Ty::Str) =>
            {
                return Err(format!(
                    "{}() returns both {} and {}",
                    self.name,
                    self.tyn(a),
                    self.tyn(b)
                ))
            }
            _ => Ty::F64,
        };
        self.observed_ret = Some(obs);
        let rv = if v.1 == Ty::F64 && self.ret == Ty::I64 {
            // provisional signature during inference; refined on the next pass
            let t = self.t();
            self.line(format!("{t} = fptosi double {} to i64", v.0));
            t
        } else if !self.sigs[&self.name].ret_annotated
            && v.1 != self.ret
            && !matches!((v.1, self.ret), (Ty::I64 | Ty::Bool, Ty::F64))
        {
            // provisional signature; the next pass returns the observed type
            let zero = match self.ret.ll() {
                "ptr" => "null",
                "double" => "0.0",
                _ => "0",
            };
            self.term(format!("ret {} {zero}", self.ret.ll()));
            return Ok(());
        } else {
            self.convert(v, self.ret)?
        };
        if !self.ensures.is_empty() {
            self.scopes.push(HashMap::new());
            let rp = self.declare("result'", self.ret);
            self.line(format!("store {} {rv}, ptr {rp}", self.ret.ll()));
            // `x` = initial value, `x'` = final value
            let finals: Vec<(String, String, Ty)> = self
                .initial
                .iter()
                .map(|(n, (_, t))| (n.clone(), self.var(n).unwrap().0, *t))
                .collect();
            for (n, ptr, t) in finals {
                let fp = self.declare(&format!("{n}'"), t);
                let cur = self.t();
                self.line(format!("{cur} = load {}, ptr {ptr}", t.ll()));
                self.line(format!("store {} {cur}, ptr {fp}", t.ll()));
                let ip = self.declare(&n, t);
                let init = self.initial[&n].0.clone();
                self.line(format!("store {} {init}, ptr {ip}", t.ll()));
            }
            for en in self.ensures.clone() {
                let c = self.cond(&en.expr)?;
                self.check(
                    &c,
                    format!(
                        "postcondition violated in {}(): ensure {}",
                        self.name, en.text
                    ),
                );
            }
            self.scopes.pop();
        }
        let ty = self.ret.ll();
        let rv = match self.sigs[&self.name].ret_unit.clone() {
            Some((scale, _)) if (scale - 1.0).abs() > 0.0 => {
                let out = self.t();
                self.line(format!("{out} = fdiv double {rv}, {}", flit(scale)));
                out
            }
            _ => rv,
        };
        self.term(format!("ret {ty} {rv}"));
        Ok(())
    }

    /// `void tulpa_entry(void)`: the top-level statements, in order. Top-level `let`s become
    /// module globals and objects constructed here get static storage, so they outlive the
    /// entry (firmware usually loops forever in it anyway).
    fn entry(&mut self, prog: &[Stmt]) -> G<()> {
        self.in_entry = true;
        self.scopes.push(HashMap::new());
        self.start("entry");
        for s in prog {
            if !executable(s) {
                continue;
            }
            match s {
                Stmt::Let(names, e, _, _) => {
                    if names.len() != 1 {
                        return Err("destructuring is not supported natively".into());
                    }
                    let v = self.expr(e)?;
                    let n = &names[0];
                    self.ctx.learn_global(n, v.1)?;
                    let v = v.0;
                    let ty = self.ctx.globals.borrow()[n];
                    self.line(format!("store {} {v}, ptr @g.{}", ty.ll(), mangle(n)));
                    self.after_effect()?;
                }
                Stmt::Always(..) => {
                    let k = self.monitor_seen;
                    self.monitor_seen += 1;
                    self.line(format!("store i1 1, ptr @tulpa.armed.{k}"));
                    self.after_effect()?;
                }
                Stmt::When(..) | Stmt::Deadline(..) => {
                    return Err("`when` / `after` rules need the interpreter's reactive runtime; use every! loops and `always` monitors in native code".into())
                }
                _ => {
                    self.stmt(s, false)?;
                }
            }
        }
        if !self.terminated {
            self.term("ret void".into());
        }
        self.finish(String::new());
        Ok(())
    }

    /// Re-checks every armed `always` monitor; a violated one runs its handler or stops the
    /// program through tulpa_violation, like the interpreter's safety envelopes.
    fn always_fn(&mut self) -> G<()> {
        self.in_always = true;
        self.scopes.push(HashMap::new());
        self.start("entry");
        let busy = self.t();
        self.line(format!("{busy} = load i1, ptr @tulpa.checking"));
        let go = self.label("go");
        let done = self.label("done");
        self.term(format!("br i1 {busy}, label %{done}, label %{go}"));
        self.start(&go);
        self.line("store i1 1, ptr @tulpa.checking".into());
        for (k, (c, handler)) in self.ctx.monitors.iter().enumerate() {
            let armed = self.t();
            self.line(format!("{armed} = load i1, ptr @tulpa.armed.{k}"));
            let test = self.label("mon");
            let next = self.label("mnext");
            self.term(format!("br i1 {armed}, label %{test}, label %{next}"));
            self.start(&test);
            let cv = self.cond(&c.expr)?;
            match handler {
                None => self.check(&cv, format!("safety envelope violated: always {}", c.text)),
                Some(h) => {
                    let bad = self.label("mbad");
                    self.term(format!("br i1 {cv}, label %{next}, label %{bad}"));
                    self.start(&bad);
                    self.block(h, false)?;
                }
            }
            self.term(format!("br label %{next}"));
            self.start(&next);
        }
        self.line("store i1 0, ptr @tulpa.checking".into());
        self.term(format!("br label %{done}"));
        self.start(&done);
        self.term("ret void".into());
        self.finish(String::new());
        Ok(())
    }

    /// Checks `always` monitors after an effect (device call, global or field write, transition).
    fn after_effect(&mut self) -> G<()> {
        if self.ctx.monitors.is_empty() || self.in_always {
            return Ok(());
        }
        if !self.sigs.contains_key("always__check") {
            let why = self
                .ctx
                .dropped
                .borrow()
                .get("always__check")
                .cloned()
                .unwrap_or_default();
            return Err(format!(
                "the program's `always` monitors are not native: {why}"
            ));
        }
        self.line("call void @tulpa_always__check()".into());
        Ok(())
    }

    fn invariant_fn(&mut self, si: u16) -> G<()> {
        self.scopes.push(HashMap::new());
        self.start("entry");
        let p = self.declare("self", Ty::Obj(si));
        self.line(format!("store ptr %a.self, ptr {p}"));
        let st = &self.ctx.structs[si as usize];
        let (name, invs) = (st.name.clone(), st.def.invariants.clone());
        for inv in &invs {
            let c = self.cond(&inv.expr)?;
            self.check(
                &c,
                format!("struct invariant violated for {name}: {}", inv.text),
            );
        }
        self.term("ret void".into());
        self.finish(String::new());
        Ok(())
    }

    fn check_invariant(&mut self, si: u16, obj: &str) -> G<()> {
        if !self.ctx.invariants(si) {
            return Ok(());
        }
        let sym = format!("{}__invariant", self.ctx.structs[si as usize].name);
        if !self.sigs.contains_key(&sym) {
            let why = self
                .ctx
                .dropped
                .borrow()
                .get(&sym)
                .cloned()
                .unwrap_or_default();
            return Err(format!(
                "the invariant of {} is not native: {why}",
                self.ctx.structs[si as usize].name
            ));
        }
        if self.name != sym {
            self.line(format!("call void @tulpa_{sym}(ptr {obj})"));
        }
        Ok(())
    }

    // ------------------------------------------------------------------ machines

    fn mach_bind_self(&mut self, mi: u16) {
        let p = self.declare("self", Ty::Mach(mi));
        self.line(format!("store ptr %a.self, ptr {p}"));
        self.implicit = Some(("%a.self".into(), mi));
    }

    fn mach_init(&mut self, mi: u16) -> G<()> {
        self.scopes.push(HashMap::new());
        self.start("entry");
        self.mach_bind_self(mi);
        let st = self.ctx.store_ty(Ty::Mach(mi));
        let f = self.t();
        self.line(format!(
            "{f} = getelementptr {st}, ptr %a.self, i32 0, i32 0"
        ));
        self.line(format!("store i32 0, ptr {f}"));
        let def = self.ctx.machines[mi as usize].def.clone();
        if let Some(enter) = &def.states[0].enter {
            self.block(enter, false)?;
        }
        self.after_effect()?;
        if !self.terminated {
            self.term("ret void".into());
        }
        self.finish(String::new());
        Ok(())
    }

    /// `tulpa_<M>__on_<event>(M *self, args...)`: picks the first enabled transition for the
    /// current state (then the global ones), runs exit / action / enter, and returns whether
    /// the event was handled. Same order as the interpreter.
    fn event(&mut self, mi: u16, ev: &str) -> G<()> {
        self.scopes.push(HashMap::new());
        self.start("entry");
        self.mach_bind_self(mi);
        let def = self.ctx.machines[mi as usize].def.clone();
        let st_ty = self.ctx.store_ty(Ty::Mach(mi));
        let sp = self.t();
        self.line(format!(
            "{sp} = getelementptr {st_ty}, ptr %a.self, i32 0, i32 0"
        ));
        let cur = self.t();
        self.line(format!("{cur} = load i32, ptr {sp}"));
        let glob = self.label("global");
        let mut cases = Vec::new();
        let mut blocks = Vec::new();
        for (k, s) in def.states.iter().enumerate() {
            if s.transitions.iter().any(|t| t.event == ev) {
                let l = self.label("in_state");
                cases.push(format!("i32 {k}, label %{l}"));
                blocks.push((k, l));
            }
        }
        self.term(format!(
            "switch i32 {cur}, label %{glob} [ {} ]",
            cases.join(" ")
        ));
        for (k, l) in blocks {
            self.start(&l);
            for t in def.states[k].transitions.iter().filter(|t| t.event == ev) {
                self.transition(mi, &def, t, Some(k), &sp)?;
            }
            self.term(format!("br label %{glob}"));
        }
        self.start(&glob);
        for t in def.global.iter().filter(|t| t.event == ev) {
            self.transition(mi, &def, t, None, &sp)?;
        }
        self.term("ret i1 0".into());
        self.finish(String::new());
        Ok(())
    }

    fn transition(
        &mut self,
        mi: u16,
        def: &MachineDef,
        t: &Transition,
        from: Option<usize>,
        sp: &str,
    ) -> G<()> {
        let sig = self.sigs[&self.name].clone();
        self.scopes.push(HashMap::new());
        if t.params.len() + 1 != sig.params.len() {
            return Err(format!(
                "event '{}' is declared with different numbers of parameters",
                t.event
            ));
        }
        for (i, p) in t.params.iter().enumerate() {
            let ty = sig.params[i + 1].1;
            let ptr = self.declare(p, ty);
            self.line(format!("store {} %a.p{i}, ptr {ptr}", ty.ll()));
        }
        let take = self.label("take");
        let next = self.label("next");
        match &t.guard {
            Some(g) => {
                let c = self.cond(g)?;
                self.term(format!("br i1 {c}, label %{take}, label %{next}"));
            }
            None => self.term(format!("br label %{take}")),
        }
        self.start(&take);
        match &t.target {
            None => {
                if let Some(a) = &t.action {
                    self.block(a, false)?;
                }
                self.after_effect()?;
            }
            Some(target) => {
                let to = def
                    .states
                    .iter()
                    .position(|s| &s.name == target)
                    .ok_or_else(|| format!("unknown state '{target}'"))?;
                match from {
                    Some(k) => {
                        if let Some(x) = &def.states[k].exit {
                            self.block(x, false)?;
                        }
                    }
                    None => {
                        // global transition: run the exit block of whichever state is current
                        let with_exit: Vec<usize> = (0..def.states.len())
                            .filter(|k| def.states[*k].exit.is_some())
                            .collect();
                        if !with_exit.is_empty() {
                            let cur = self.t();
                            self.line(format!("{cur} = load i32, ptr {sp}"));
                            let after = self.label("exited");
                            let mut cases = Vec::new();
                            let mut bl = Vec::new();
                            for k in with_exit {
                                let l = self.label("exit");
                                cases.push(format!("i32 {k}, label %{l}"));
                                bl.push((k, l));
                            }
                            self.term(format!(
                                "switch i32 {cur}, label %{after} [ {} ]",
                                cases.join(" ")
                            ));
                            for (k, l) in bl {
                                self.start(&l);
                                self.block(def.states[k].exit.as_ref().unwrap(), false)?;
                                self.term(format!("br label %{after}"));
                            }
                            self.start(&after);
                        }
                    }
                }
                if let Some(a) = &t.action {
                    self.block(a, false)?;
                }
                self.line(format!("store i32 {to}, ptr {sp}"));
                if let Some(e) = &def.states[to].enter {
                    self.block(e, false)?;
                }
                self.after_effect()?;
            }
        }
        let _ = mi;
        self.term("ret i1 1".into());
        self.start(&next);
        self.scopes.pop();
        Ok(())
    }

    // ------------------------------------------------------------------ objects

    /// Storage for a new object: static in the program entry, a stack slot elsewhere.
    fn alloc_obj(&mut self, t: Ty) -> String {
        let st = self.ctx.store_ty(t);
        let p = if self.in_entry {
            self.objn += 1;
            let p = format!("@o.{}", self.objn);
            self.module_extra
                .push_str(&format!("{p} = internal global {st} zeroinitializer\n"));
            p
        } else {
            self.lbl += 1;
            let p = format!("%obj{}", self.lbl);
            self.entry_allocas
                .push_str(&format!("  {p} = alloca {st}\n"));
            p
        };
        self.line(format!("store {st} zeroinitializer, ptr {p}"));
        p
    }

    fn struct_complete(&self, si: u16) -> G<()> {
        let st = &self.ctx.structs[si as usize];
        let unknown: Vec<&str> = (0..st.fields.len())
            .filter(|k| self.ctx.field_ty(si, *k).is_none())
            .map(|k| st.fields[k].0.as_str())
            .collect();
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "struct {}: no native type known for field(s) {} (annotate them, e.g. `bus: I2C`, or construct the struct natively)",
                st.name,
                unknown.join(", ")
            ))
        }
    }

    fn mach_complete(&self, mi: u16) -> G<()> {
        let m = &self.ctx.machines[mi as usize];
        let unknown: Vec<&str> = (0..m.params.len())
            .filter(|k| self.ctx.mparam_ty(mi, *k).is_none())
            .map(|k| m.params[k].0.as_str())
            .collect();
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "machine {}: no native type known for parameter(s) {}",
                m.name,
                unknown.join(", ")
            ))
        }
    }

    /// Stores `v` into an object slot of type `t` at `ptr` (copying inline objects).
    fn store_slot(&mut self, ptr: &str, t: Ty, v: (String, Ty), what: &str) -> G<()> {
        if t.is_ref() {
            if v.1 != t {
                return Err(format!(
                    "{what}: {} value is used where {} is expected",
                    a_an(&self.tyn(v.1)),
                    self.tyn(t)
                ));
            }
            let st = self.ctx.store_ty(t);
            let tmp = self.t();
            self.line(format!("{tmp} = load {st}, ptr {}", v.0));
            self.line(format!("store {st} {tmp}, ptr {ptr}"));
        } else {
            let c = self.convert(v, t).map_err(|e| format!("{what}: {e}"))?;
            self.line(format!("store {} {c}, ptr {ptr}", t.ll()));
        }
        Ok(())
    }

    fn field_ptr(&mut self, obj: &str, owner: Ty, k: usize) -> String {
        let st = self.ctx.store_ty(owner);
        let p = self.t();
        self.line(format!(
            "{p} = getelementptr {st}, ptr {obj}, i32 0, i32 {k}"
        ));
        p
    }

    fn load_slot(&mut self, ptr: String, t: Ty) -> (String, Ty) {
        if t.is_ref() {
            (ptr, t)
        } else {
            let v = self.t();
            self.line(format!("{v} = load {}, ptr {ptr}", t.ll()));
            (v, t)
        }
    }

    /// Matches call arguments to named slots (positional, then `name: value`).
    fn bind_args<'e>(
        &self,
        names: &[String],
        args: &'e [Expr],
        what: &str,
    ) -> G<Vec<Option<&'e Expr>>> {
        let mut out: Vec<Option<&Expr>> = vec![None; names.len()];
        let mut pos = 0;
        for a in args {
            match a {
                Expr::Named(n, e) => {
                    let k = names
                        .iter()
                        .position(|x| x == n)
                        .ok_or_else(|| format!("{what} has no field '{n}'"))?;
                    out[k] = Some(e);
                }
                e => {
                    if pos >= names.len() {
                        return Err(format!("{what} takes at most {} values", names.len()));
                    }
                    out[pos] = Some(e);
                    pos += 1;
                }
            }
        }
        Ok(out)
    }

    fn construct_struct(&mut self, si: u16, args: &[Expr]) -> G<(String, Ty)> {
        let st_name = self.ctx.structs[si as usize].name.clone();
        let def = self.ctx.structs[si as usize].def.clone();
        let names: Vec<String> = def.fields.iter().map(|f| f.name.clone()).collect();
        let bound = self.bind_args(&names, args, &st_name)?;
        // learn field types from this construction site first
        let mut vals = Vec::new();
        for (k, f) in def.fields.iter().enumerate() {
            let e = match bound[k] {
                Some(e) => e.clone(),
                None => match &f.default {
                    Some(d) => d.clone(),
                    None => return Err(format!("{st_name}() missing field '{}'", f.name)),
                },
            };
            let v = self.expr(&e)?;
            if self.ctx.field_ty(si, k).is_none() {
                let t = if v.1 == Ty::Str { None } else { Some(v.1) };
                if let Some(t) = t {
                    self.ctx.learn_field(si, k, t);
                }
            }
            vals.push(v);
        }
        self.struct_complete(si)?;
        let obj = self.alloc_obj(Ty::Obj(si));
        for (k, v) in vals.into_iter().enumerate() {
            let t = self.ctx.field_ty(si, k).unwrap();
            let p = self.field_ptr(&obj, Ty::Obj(si), k);
            self.store_slot(&p, t, v, &format!("field '{}' of {st_name}", names[k]))?;
        }
        self.check_invariant(si, &obj)?;
        Ok((obj, Ty::Obj(si)))
    }

    fn construct_mach(&mut self, mi: u16, args: &[Expr]) -> G<(String, Ty)> {
        let m_name = self.ctx.machines[mi as usize].name.clone();
        let def = self.ctx.machines[mi as usize].def.clone();
        let names: Vec<String> = def.params.iter().map(|p| p.name.clone()).collect();
        let bound = self.bind_args(&names, args, &m_name)?;
        let mut vals = Vec::new();
        for (k, p) in def.params.iter().enumerate() {
            let e = match bound[k] {
                Some(e) => e.clone(),
                None => match &p.default {
                    Some(d) => d.clone(),
                    None => return Err(format!("{m_name}() missing argument '{}'", p.name)),
                },
            };
            let v = self.expr(&e)?;
            if self.ctx.mparam_ty(mi, k).is_none() && v.1 != Ty::Str {
                self.ctx.learn_mparam(mi, k, v.1);
            }
            vals.push(v);
        }
        self.mach_complete(mi)?;
        let sym = format!("{m_name}__init");
        if !self.sigs.contains_key(&sym) {
            let why = self
                .ctx
                .dropped
                .borrow()
                .get(&sym)
                .cloned()
                .unwrap_or_default();
            return Err(format!("machine {m_name} is not native: {why}"));
        }
        let obj = self.alloc_obj(Ty::Mach(mi));
        for (k, v) in vals.into_iter().enumerate() {
            let t = self.ctx.mparam_ty(mi, k).unwrap();
            let p = self.field_ptr(&obj, Ty::Mach(mi), k + 1);
            self.store_slot(&p, t, v, &format!("parameter '{}' of {m_name}", names[k]))?;
        }
        self.line(format!("call void @tulpa_{sym}(ptr {obj})"));
        Ok((obj, Ty::Mach(mi)))
    }

    fn array_elem_ptr(&mut self, arr: &str, n: u16, idx: (String, Ty)) -> G<String> {
        let i = self
            .convert(idx, Ty::I64)
            .map_err(|e| format!("list index: {e}"))?;
        // negative indices count from the end, like the interpreter
        let neg = self.t();
        self.line(format!("{neg} = icmp slt i64 {i}, 0"));
        let wrapped = self.t();
        self.line(format!("{wrapped} = add i64 {i}, {n}"));
        let ix = self.t();
        self.line(format!("{ix} = select i1 {neg}, i64 {wrapped}, i64 {i}"));
        let okl = self.t();
        self.line(format!("{okl} = icmp sge i64 {ix}, 0"));
        let okh = self.t();
        self.line(format!("{okh} = icmp slt i64 {ix}, {n}"));
        let ok = self.t();
        self.line(format!("{ok} = and i1 {okl}, {okh}"));
        let name = self.name.clone();
        self.check(
            &ok,
            format!("list index out of range (length {n}) in {name}()"),
        );
        let p = self.t();
        self.line(format!(
            "{p} = getelementptr [{n} x i64], ptr {arr}, i64 0, i64 {ix}"
        ));
        Ok(p)
    }

    fn new_array(&mut self, items: &[Expr]) -> G<(String, Ty)> {
        let n = items.len();
        if n > 65535 {
            return Err("list literal too long".into());
        }
        let mut vals = Vec::new();
        for it in items {
            let v = self.expr(it)?;
            if !matches!(v.1, Ty::I64 | Ty::Bool) {
                return Err(format!(
                    "native lists hold integers (bytes, counts); got {}",
                    self.tyn(v.1)
                ));
            }
            vals.push(v);
        }
        let arr = self.alloc_obj(Ty::Arr(n as u16));
        for (i, v) in vals.into_iter().enumerate() {
            let c = self.convert(v, Ty::I64)?;
            let p = self.t();
            self.line(format!(
                "{p} = getelementptr [{n} x i64], ptr {arr}, i64 0, i64 {i}"
            ));
            self.line(format!("store i64 {c}, ptr {p}"));
        }
        Ok((arr, Ty::Arr(n as u16)))
    }

    fn repeat_array(&mut self, v: &Expr, n: &Expr) -> G<(String, Ty)> {
        let Expr::Int(k) = n else {
            return Err("native `[value; count]` needs an integer literal count".into());
        };
        if *k < 0 || *k > 65535 {
            return Err("list length out of range".into());
        }
        let k = *k as u16;
        let x = self.expr(v)?;
        let x = self
            .convert(x, Ty::I64)
            .map_err(|e| format!("list element: {e}"))?;
        let arr = self.alloc_obj(Ty::Arr(k));
        if x != "0" {
            for i in 0..k {
                let p = self.t();
                self.line(format!(
                    "{p} = getelementptr [{k} x i64], ptr {arr}, i64 0, i64 {i}"
                ));
                self.line(format!("store i64 {x}, ptr {p}"));
            }
        }
        Ok((arr, Ty::Arr(k)))
    }

    // ------------------------------------------------------------------ statements

    fn block(&mut self, stmts: &[Stmt], want: bool) -> G<Option<(String, Ty)>> {
        self.scopes.push(HashMap::new());
        let mut last = None;
        for (i, s) in stmts.iter().enumerate() {
            last = self.stmt(s, want && i + 1 == stmts.len())?;
        }
        self.scopes.pop();
        Ok(last)
    }

    fn assign_name(&mut self, n: &str, op: &Option<String>, e: &Expr) -> G<()> {
        // local variable
        if let Some((ptr, ty)) = self.var(n) {
            let rhs = match op {
                Some(o) => {
                    let cur = self.t();
                    self.line(format!("{cur} = load {}, ptr {ptr}", ty.ll()));
                    let r = self.expr(e)?;
                    self.binop(o, (cur, ty), r)?
                }
                None => self.expr(e)?,
            };
            if ty.is_ref() {
                if rhs.1 != ty {
                    return Err(format!(
                        "'{n}' holds a {}, not a {}",
                        self.tyn(ty),
                        self.tyn(rhs.1)
                    ));
                }
                self.line(format!("store ptr {}, ptr {ptr}", rhs.0));
            } else {
                let v = self.convert(rhs, ty)?;
                self.line(format!("store {} {v}, ptr {ptr}", ty.ll()));
            }
            return Ok(());
        }
        // a machine parameter, inside the machine's bodies
        if let Some((sp, mi)) = self.implicit.clone() {
            if let Some(k) = self.ctx.machines[mi as usize]
                .params
                .iter()
                .position(|p| p.0 == n)
            {
                let t = self
                    .ctx
                    .mparam_ty(mi, k)
                    .ok_or("machine parameter type unknown")?;
                let p = self.field_ptr(&sp, Ty::Mach(mi), k + 1);
                let rhs = match op {
                    Some(o) => {
                        let cur = self.load_slot(p.clone(), t);
                        let r = self.expr(e)?;
                        self.binop(o, cur, r)?
                    }
                    None => self.expr(e)?,
                };
                self.store_slot(&p, t, rhs, n)?;
                return self.after_effect();
            }
        }
        // a module global
        if let Some((gp, ty)) = self.global(n)? {
            let rhs = match op {
                Some(o) => {
                    let cur = self.t();
                    self.line(format!("{cur} = load {}, ptr {gp}", ty.ll()));
                    let r = self.expr(e)?;
                    self.binop(o, (cur, ty), r)?
                }
                None => self.expr(e)?,
            };
            if ty.is_ref() {
                if rhs.1 != ty {
                    return Err(format!(
                        "global '{n}' holds a {}, not a {}",
                        self.tyn(ty),
                        self.tyn(rhs.1)
                    ));
                }
                if rhs.0.starts_with("%obj") {
                    return Err(format!(
                        "an object created inside {}() would outlive it in global '{n}'",
                        self.name
                    ));
                }
                self.line(format!("store ptr {}, ptr {gp}", rhs.0));
            } else {
                let v = self.convert(rhs, ty)?;
                self.line(format!("store {} {v}, ptr {gp}", ty.ll()));
            }
            return self.after_effect();
        }
        Err(format!("assignment to unknown variable '{n}'"))
    }

    fn assign_field(&mut self, o: &Expr, f: &str, op: &Option<String>, e: &Expr) -> G<()> {
        let ov = self.expr(o)?;
        let (k, t, owner) = match ov.1 {
            Ty::Obj(si) => {
                let st = &self.ctx.structs[si as usize];
                let k = st
                    .fields
                    .iter()
                    .position(|x| x.0 == f)
                    .ok_or_else(|| format!("{} has no field '{f}'", st.name))?;
                let t = self.ctx.field_ty(si, k).ok_or_else(|| {
                    format!("type of {}.{f} is unknown natively (annotate it)", st.name)
                })?;
                (k, t, ov.1)
            }
            Ty::Mach(mi) => {
                let m = &self.ctx.machines[mi as usize];
                let k = m.params.iter().position(|x| x.0 == f).ok_or_else(|| {
                    format!("machine {} has no parameter '{f}' to assign", m.name)
                })?;
                let t = self
                    .ctx
                    .mparam_ty(mi, k)
                    .ok_or("machine parameter type unknown")?;
                (k + 1, t, ov.1)
            }
            other => return Err(format!("cannot assign a field of {}", self.tyn(other))),
        };
        self.struct_or_mach_complete(owner)?;
        let p = self.field_ptr(&ov.0, owner, k);
        let rhs = match op {
            Some(opn) => {
                if t.is_ref() {
                    return Err("compound assignment needs a number".into());
                }
                let cur = self.load_slot(p.clone(), t);
                let r = self.expr(e)?;
                self.binop(opn, cur, r)?
            }
            None => self.expr(e)?,
        };
        self.store_slot(&p, t, rhs, &format!("field '{f}'"))?;
        if let Ty::Obj(si) = owner {
            self.check_invariant(si, &ov.0)?;
        }
        self.after_effect()
    }

    fn struct_or_mach_complete(&self, t: Ty) -> G<()> {
        match t {
            Ty::Obj(si) => self.struct_complete(si),
            Ty::Mach(mi) => self.mach_complete(mi),
            _ => Ok(()),
        }
    }

    fn stmt(&mut self, s: &Stmt, want: bool) -> G<Option<(String, Ty)>> {
        match s {
            Stmt::Expr(e, _) => {
                let v = self.expr(e)?;
                return Ok(if want { Some(v) } else { None });
            }
            Stmt::Let(names, e, _, _) => {
                if names.len() != 1 {
                    return Err("destructuring is not supported natively".into());
                }
                let v = self.expr(e)?;
                let ptr = self.declare(&names[0], v.1);
                self.line(format!("store {} {}, ptr {ptr}", v.1.ll(), v.0));
            }
            Stmt::Assign(Target::Name(n), op, e, _) => self.assign_name(n, op, e)?,
            Stmt::Assign(Target::Field(o, f), op, e, _) => self.assign_field(o, f, op, e)?,
            Stmt::Assign(Target::Index(a, i), op, e, _) => {
                let av = self.expr(a)?;
                let Ty::Arr(n) = av.1 else {
                    return Err(format!("cannot index a {} natively", self.tyn(av.1)));
                };
                let iv = self.expr(i)?;
                let p = self.array_elem_ptr(&av.0, n, iv)?;
                let rhs = match op {
                    Some(o) => {
                        let cur = self.t();
                        self.line(format!("{cur} = load i64, ptr {p}"));
                        let r = self.expr(e)?;
                        self.binop(o, (cur, Ty::I64), r)?
                    }
                    None => self.expr(e)?,
                };
                let v = self
                    .convert(rhs, Ty::I64)
                    .map_err(|e| format!("list element: {e}"))?;
                self.line(format!("store i64 {v}, ptr {p}"));
            }
            Stmt::Return(e, _) => {
                let v = match e {
                    Some(e) => self.expr(e)?,
                    None => ("0".into(), Ty::I64),
                };
                self.emit_return(v)?;
                let dead = self.label("dead");
                self.start(&dead);
                self.after_return = true;
            }
            Stmt::If(c, a, b) => return self.if_chain(c, a, b.as_ref(), want),
            Stmt::While(c, invs, body, _) => {
                let head = self.label("while");
                let bodyl = self.label("do");
                let end = self.label("wend");
                self.term(format!("br label %{head}"));
                self.start(&head);
                for inv in invs {
                    let ci = self.cond(&inv.expr)?;
                    self.check(&ci, format!("loop invariant violated: {}", inv.text));
                }
                let cv = self.cond(c)?;
                self.term(format!("br i1 {cv}, label %{bodyl}, label %{end}"));
                self.start(&bodyl);
                self.loops.push((head.clone(), end.clone()));
                self.block(body, false)?;
                self.loops.pop();
                self.term(format!("br label %{head}"));
                self.start(&end);
            }
            Stmt::Loop(body) => {
                let head = self.label("loop");
                let end = self.label("lend");
                self.term(format!("br label %{head}"));
                self.start(&head);
                self.loops.push((head.clone(), end.clone()));
                self.block(body, false)?;
                self.loops.pop();
                self.term(format!("br label %{head}"));
                self.start(&end);
            }
            Stmt::For(names, it, body, _) => {
                if names.len() != 1 {
                    return Err("destructuring is not supported natively".into());
                }
                let (start_v, end_v, incl, arr) = match it {
                    Expr::Range(a, b, incl) => {
                        let av = self.expr(a)?;
                        let bv = self.expr(b)?;
                        if av.1 != Ty::I64 || bv.1 != Ty::I64 {
                            return Err("range bounds must be integers".into());
                        }
                        (av.0, bv.0, *incl, None)
                    }
                    other => {
                        let v = self.expr(other)?;
                        let Ty::Arr(n) = v.1 else {
                            return Err(
                                "native `for` loops iterate over a..b ranges or lists".into()
                            );
                        };
                        ("0".to_string(), n.to_string(), false, Some((v.0, n)))
                    }
                };
                self.scopes.push(HashMap::new());
                let iv = self.declare(&names[0], Ty::I64);
                let ctr = self.declare("\0ctr", Ty::I64);
                self.line(format!("store i64 {start_v}, ptr {ctr}"));
                let head = self.label("for");
                let bodyl = self.label("fbody");
                let step = self.label("fstep");
                let end = self.label("fend");
                self.term(format!("br label %{head}"));
                self.start(&head);
                let cur = self.t();
                self.line(format!("{cur} = load i64, ptr {ctr}"));
                let c = self.t();
                self.line(format!(
                    "{c} = icmp {} i64 {cur}, {end_v}",
                    if incl { "sle" } else { "slt" }
                ));
                self.term(format!("br i1 {c}, label %{bodyl}, label %{end}"));
                self.start(&bodyl);
                match &arr {
                    None => self.line(format!("store i64 {cur}, ptr {iv}")),
                    Some((a, n)) => {
                        let p = self.t();
                        self.line(format!(
                            "{p} = getelementptr [{n} x i64], ptr {a}, i64 0, i64 {cur}"
                        ));
                        let x = self.t();
                        self.line(format!("{x} = load i64, ptr {p}"));
                        self.line(format!("store i64 {x}, ptr {iv}"));
                    }
                }
                self.loops.push((step.clone(), end.clone()));
                self.block(body, false)?;
                self.loops.pop();
                self.term(format!("br label %{step}"));
                self.start(&step);
                let c2 = self.t();
                self.line(format!("{c2} = load i64, ptr {ctr}"));
                let n = self.t();
                self.line(format!("{n} = add i64 {c2}, 1"));
                self.line(format!("store i64 {n}, ptr {ctr}"));
                self.term(format!("br label %{head}"));
                self.start(&end);
                self.scopes.pop();
            }
            Stmt::Break | Stmt::Continue => {
                let (cont, brk) = self
                    .loops
                    .last()
                    .cloned()
                    .ok_or("break/continue outside loop")?;
                let target = if matches!(s, Stmt::Break) { brk } else { cont };
                self.term(format!("br label %{target}"));
                let dead = self.label("dead");
                self.start(&dead);
            }
            Stmt::Assert(c, _, text, _) => {
                let cv = self.cond(c)?;
                self.check(&cv, format!("assertion failed: {text}"));
            }
            Stmt::Use(_) => {}
            other => {
                return Err(format!(
                    "statement not supported natively: {}",
                    stmt_kind(other)
                ))
            }
        }
        Ok(None)
    }

    fn if_chain(
        &mut self,
        c: &Expr,
        a: &[Stmt],
        b: Option<&Vec<Stmt>>,
        want: bool,
    ) -> G<Option<(String, Ty)>> {
        let cv = self.cond(c)?;
        let then = self.label("then");
        let els = self.label("else");
        let end = self.label("endif");
        self.term(format!("br i1 {cv}, label %{then}, label %{els}"));
        self.start(&then);
        let av = self.block(a, want)?;
        let a_end_open = !self.terminated;
        let mut a_val = None;
        if want && a_end_open {
            a_val = av;
        }
        // values are joined through stack slots so both arms can convert types
        let slots = if want {
            Some((
                self.declare("\0ifv", Ty::F64),
                self.declare("\0ifi", Ty::I64),
                self.declare("\0ifb", Ty::Bool),
                self.declare("\0ifp", Ty::Str),
            ))
        } else {
            None
        };
        let mut tys = Vec::new();
        if let (Some(v), Some(sl)) = (&a_val, &slots) {
            tys.push(v.1);
            self.store_join(v.clone(), sl)?;
        }
        self.term(format!("br label %{end}"));
        self.start(&els);
        let bv = match b {
            Some(b) => self.block(b, want)?,
            None => None,
        };
        if let (true, Some(sl)) = (want && !self.terminated, &slots) {
            let v = bv.unwrap_or(("0".into(), Ty::I64));
            tys.push(v.1);
            self.store_join(v, sl)?;
        }
        self.term(format!("br label %{end}"));
        self.start(&end);
        if !want || tys.is_empty() {
            return Ok(None);
        }
        let ty = if tys.iter().all(|t| *t == tys[0]) {
            tys[0]
        } else if tys
            .iter()
            .any(|t| t.ll() == "ptr" || matches!(t, Ty::State(_)))
        {
            // mixed object / number arms: only usable as a statement
            return Ok(None);
        } else {
            Ty::F64
        };
        let (sf, si, sb, sp) = slots.unwrap();
        let t = self.t();
        let ptr = match ty {
            Ty::F64 | Ty::Q(_) => sf,
            Ty::I64 => si,
            Ty::Bool => sb,
            Ty::State(_) => return Ok(None),
            _ => sp,
        };
        self.line(format!("{t} = load {}, ptr {ptr}", ty.ll()));
        Ok(Some((t, ty)))
    }

    fn store_join(&mut self, v: (String, Ty), sl: &(String, String, String, String)) -> G<()> {
        if v.1.ll() == "ptr" {
            self.line(format!("store ptr {}, ptr {}", v.0, sl.3));
            return Ok(());
        }
        if matches!(v.1, Ty::State(_)) {
            return Ok(());
        }
        if v.1 == Ty::Bool {
            self.line(format!("store i1 {}, ptr {}", v.0, sl.2));
        }
        if v.1 == Ty::I64 {
            self.line(format!("store i64 {}, ptr {}", v.0, sl.1));
        }
        let fv = if matches!(v.1, Ty::Q(_)) {
            v.0
        } else {
            self.convert(v, Ty::F64)?
        };
        self.line(format!("store double {fv}, ptr {}", sl.0));
        Ok(())
    }

    fn cond(&mut self, e: &Expr) -> G<String> {
        let v = self.expr(e)?;
        self.convert(v, Ty::Bool)
    }

    // ------------------------------------------------------------------ expressions

    fn expr(&mut self, e: &Expr) -> G<(String, Ty)> {
        match e {
            Expr::Int(n) => Ok((n.to_string(), Ty::I64)),
            Expr::Float(f) => Ok((flit(*f), Ty::F64)),
            Expr::Bool(b) => Ok(((*b as u8).to_string(), Ty::Bool)),
            Expr::Str(s) => Ok((self.cstr(s), Ty::Str)),
            Expr::Ident(n, _) => {
                if let Some((ptr, ty)) = self.var(n) {
                    let t = self.t();
                    self.line(format!("{t} = load {}, ptr {ptr}", ty.ll()));
                    return Ok((t, ty));
                }
                if let Some((sp, mi)) = self.implicit.clone() {
                    if let Some(k) = self.ctx.machines[mi as usize]
                        .params
                        .iter()
                        .position(|p| &p.0 == n)
                    {
                        let t = self
                            .ctx
                            .mparam_ty(mi, k)
                            .ok_or("machine parameter type unknown")?;
                        let p = self.field_ptr(&sp, Ty::Mach(mi), k + 1);
                        return Ok(self.load_slot(p, t));
                    }
                }
                if let Some((gp, ty)) = self.global(n)? {
                    let t = self.t();
                    self.line(format!("{t} = load {}, ptr {gp}", ty.ll()));
                    return Ok((t, ty));
                }
                self.consts
                    .get(n)
                    .cloned()
                    .ok_or_else(|| format!("'{n}' is not a local, a global or a numeric constant"))
            }
            Expr::Path(m, n, _) if m == "math" => match n.as_str() {
                "pi" => Ok((flit(std::f64::consts::PI), Ty::F64)),
                "tau" => Ok((flit(std::f64::consts::TAU), Ty::F64)),
                "e" => Ok((flit(std::f64::consts::E), Ty::F64)),
                _ => Err(format!("math::{n} is not a native constant")),
            },
            Expr::List(items) => self.new_array(items),
            Expr::Repeat(v, n) => self.repeat_array(v, n),
            Expr::Unary(op, x, _) => {
                let v = self.expr(x)?;
                let t = self.t();
                match (op.as_str(), v.1) {
                    ("-", Ty::I64) => {
                        let r = self.overflow_op("ssub", "0", &v.0, "negation")?;
                        return Ok((r, Ty::I64));
                    }
                    ("-", Ty::F64) | ("-", Ty::Q(_)) => {
                        self.line(format!("{t} = fneg double {}", v.0))
                    }
                    ("!", _) | ("not", _) => {
                        let b = self.convert(v, Ty::Bool)?;
                        self.line(format!("{t} = xor i1 {b}, 1"));
                        return Ok((t, Ty::Bool));
                    }
                    ("~", Ty::I64) => self.line(format!("{t} = xor i64 {}, -1", v.0)),
                    _ => return Err(format!("unary '{op}' not supported natively for this type")),
                }
                Ok((t, v.1))
            }
            Expr::Binary(l, op, r, _) => {
                // `machine.state == "Name"`
                if matches!(op.as_str(), "==" | "!=") {
                    let (other, lit) = match (&**l, &**r) {
                        (o, Expr::Str(s)) => (Some(o), Some(s)),
                        (Expr::Str(s), o) => (Some(o), Some(s)),
                        _ => (None, None),
                    };
                    if let (Some(o), Some(lit)) = (other, lit) {
                        let v = self.expr(o)?;
                        if let Ty::State(mi) = v.1 {
                            let m = &self.ctx.machines[mi as usize];
                            let k = m
                                .def
                                .states
                                .iter()
                                .position(|s| &s.name == lit)
                                .ok_or_else(|| {
                                    format!("machine {} has no state '{lit}'", m.name)
                                })?;
                            let t = self.t();
                            self.line(format!(
                                "{t} = icmp {} i32 {}, {k}",
                                if op == "==" { "eq" } else { "ne" },
                                v.0
                            ));
                            return Ok((t, Ty::Bool));
                        }
                        return Err(
                            "strings can only be compared with a machine's state natively".into(),
                        );
                    }
                }
                let a = self.expr(l)?;
                let b = self.expr(r)?;
                self.binop(op, a, b)
            }
            Expr::Logic(l, is_and, r) => {
                let a = self.cond(l)?;
                let slot = self.declare("\0logic", Ty::Bool);
                self.line(format!("store i1 {a}, ptr {slot}"));
                let rhs = self.label("rhs");
                let end = self.label("lend");
                if *is_and {
                    self.term(format!("br i1 {a}, label %{rhs}, label %{end}"));
                } else {
                    self.term(format!("br i1 {a}, label %{end}, label %{rhs}"));
                }
                self.start(&rhs);
                let b = self.cond(r)?;
                self.line(format!("store i1 {b}, ptr {slot}"));
                self.term(format!("br label %{end}"));
                self.start(&end);
                let t = self.t();
                self.line(format!("{t} = load i1, ptr {slot}"));
                Ok((t, Ty::Bool))
            }
            Expr::IfExpr(c, a, b) => self
                .if_chain(c, a, b.as_ref(), true)?
                .ok_or_else(|| "if-expression without a value".to_string()),
            Expr::Call(f, args, _) => self.call(f, args),
            Expr::Quantity(v, u) => Ok((flit(v * u.scale), qty(u.dim))),
            Expr::MethodCall(o, m, args, _)
                if matches!(m.as_str(), "in" | "as" | "to")
                    && matches!(args.as_slice(), [Expr::Str(_)]) =>
            {
                let [Expr::Str(unit)] = args.as_slice() else {
                    unreachable!()
                };
                let v = self.expr(o)?;
                let u = units::parse(unit)?;
                if dim_of(v.1) != u.dim {
                    return Err(format!(
                        "cannot convert {} to '{unit}' ({})",
                        units::describe(&dim_of(v.1)),
                        units::describe(&u.dim)
                    ));
                }
                if m == "to" {
                    return Ok(v); // same physical value; display units don't exist natively
                }
                let t = self.t();
                self.line(format!("{t} = fdiv double {}, {}", v.0, flit(u.scale)));
                Ok((t, Ty::F64))
            }
            Expr::MethodCall(o, m, args, _) => self.method_call(o, m, args),
            Expr::Field(o, f, _) if f == "si" => {
                let v = self.expr(o)?;
                match v.1 {
                    Ty::Q(_) => Ok((v.0, Ty::F64)),
                    _ => Err(".si is only defined on quantities".into()),
                }
            }
            Expr::Field(o, f, _) => self.field_get(o, f),
            Expr::Index(a, i, _) => {
                let av = self.expr(a)?;
                let Ty::Arr(n) = av.1 else {
                    return Err(format!("cannot index a {} natively", self.tyn(av.1)));
                };
                let iv = self.expr(i)?;
                let p = self.array_elem_ptr(&av.0, n, iv)?;
                let t = self.t();
                self.line(format!("{t} = load i64, ptr {p}"));
                Ok((t, Ty::I64))
            }
            Expr::MacroCall(name, args, _) => self.macro_call(name, args),
            other => Err(format!(
                "expression not supported natively: {}",
                expr_kind(other)
            )),
        }
    }

    fn field_get(&mut self, o: &Expr, f: &str) -> G<(String, Ty)> {
        if self.is_module_alias(o) {
            return self.expr(&Expr::Ident(f.to_string(), 0));
        }
        let ov = self.expr(o)?;
        match ov.1 {
            Ty::Obj(si) => {
                let st = &self.ctx.structs[si as usize];
                let Some(k) = st.fields.iter().position(|x| x.0 == f) else {
                    return Err(format!("{} has no field '{f}'", st.name));
                };
                let t = self.ctx.field_ty(si, k).ok_or_else(|| {
                    format!("type of {}.{f} is unknown natively (annotate it)", st.name)
                })?;
                self.struct_complete(si)?;
                let p = self.field_ptr(&ov.0, ov.1, k);
                Ok(self.load_slot(p, t))
            }
            Ty::Mach(mi) => {
                self.mach_complete(mi)?;
                if f == "state" {
                    let p = self.field_ptr(&ov.0, ov.1, 0);
                    let v = self.t();
                    self.line(format!("{v} = load i32, ptr {p}"));
                    return Ok((v, Ty::State(mi)));
                }
                let m = &self.ctx.machines[mi as usize];
                let Some(k) = m.params.iter().position(|x| x.0 == f) else {
                    return Err(format!("machine {} has no field '{f}' natively", m.name));
                };
                let t = self
                    .ctx
                    .mparam_ty(mi, k)
                    .ok_or("machine parameter type unknown")?;
                let p = self.field_ptr(&ov.0, ov.1, k + 1);
                Ok(self.load_slot(p, t))
            }
            other => Err(format!("{} has no field '{f}' natively", self.tyn(other))),
        }
    }

    /// Calls a compiled unit with the arguments converted to its signature.
    fn call_unit(
        &mut self,
        sym: &str,
        label: &str,
        recv: Option<(String, Ty)>,
        args: &[Expr],
    ) -> G<(String, Ty)> {
        let Some(sig) = self.sigs.get(sym).cloned() else {
            let why = self.ctx.dropped.borrow().get(sym).cloned();
            return Err(match why {
                Some(w) => format!("calls {label}(), which is not native: {w}"),
                None => format!("call to '{label}' is not supported natively"),
            });
        };
        let mut vals = Vec::new();
        let skip = usize::from(recv.is_some());
        if let Some(r) = recv {
            vals.push(r);
        }
        // positional, then `name: value`, then defaults
        let n = sig.params.len();
        let mut slots: Vec<Option<&Expr>> = vec![None; n.saturating_sub(skip)];
        let mut pos = 0;
        for a in args {
            match a {
                Expr::Named(name, e) => {
                    let k = sig.params[skip..]
                        .iter()
                        .position(|(pn, _)| pn == name)
                        .ok_or_else(|| format!("{label}() has no parameter '{name}'"))?;
                    slots[k] = Some(e);
                }
                e => {
                    if pos >= slots.len() {
                        return Err(format!(
                            "{label}() takes at most {} argument(s)",
                            slots.len()
                        ));
                    }
                    slots[pos] = Some(e);
                    pos += 1;
                }
            }
        }
        for (k, slot) in slots.iter().enumerate() {
            let e = match slot {
                Some(e) => (*e).clone(),
                None => match sig.defaults.get(k + skip).cloned().flatten() {
                    Some(d) => d,
                    None => {
                        return Err(format!(
                            "{label}() missing argument '{}'",
                            sig.params[k + skip].0
                        ))
                    }
                },
            };
            vals.push(self.expr(&e)?);
        }
        if sig.params.len() == vals.len() {
            for (k, v) in vals.iter().enumerate() {
                if sig.inferred.get(k).copied().unwrap_or(false) && v.1 != sig.params[k].1 {
                    let key = (sym.to_string(), k);
                    let mut m = self.ctx.param_tys.borrow_mut();
                    if !m.contains_key(&key) && !(v.1 == Ty::I64 && sig.params[k].1 == Ty::F64) {
                        m.insert(key, v.1);
                        *self.ctx.learned.borrow_mut() = true;
                    }
                }
            }
        }
        if sig.params.len() != vals.len() {
            return Err(format!(
                "{label}() called with {} args, expects {}",
                vals.len() - usize::from(sig.params.first().is_some_and(|p| p.0 == "self")),
                sig.params.len() - usize::from(sig.params.first().is_some_and(|p| p.0 == "self"))
            ));
        }
        let mut argl = Vec::new();
        for ((v, (pn, t)), u) in vals
            .into_iter()
            .zip(sig.params.iter())
            .zip(sig.units.iter())
        {
            let mut c = if t.is_ref() {
                if v.1 != *t {
                    return Err(format!(
                        "argument '{pn}' of {label}(): {} value is used where {} is expected",
                        a_an(&self.tyn(v.1)),
                        self.tyn(*t)
                    ));
                }
                v.0
            } else {
                self.convert(v, *t)
                    .map_err(|e| format!("argument '{pn}' of {label}(): {e}"))?
            };
            if let Some((scale, _)) = u {
                if (*scale - 1.0).abs() > 0.0 {
                    let d = self.t();
                    self.line(format!("{d} = fdiv double {c}, {}", flit(*scale)));
                    c = d;
                }
            }
            argl.push(format!("{} {c}", t.ll()));
        }
        if sig.void {
            self.line(format!("call void @tulpa_{sym}({})", argl.join(", ")));
            return Ok(("0".into(), Ty::I64));
        }
        let mut t = self.t();
        self.line(format!(
            "{t} = call {} @tulpa_{sym}({})",
            sig.ret.ll(),
            argl.join(", ")
        ));
        if let Some((scale, _)) = &sig.ret_unit {
            if (*scale - 1.0).abs() > 0.0 {
                let si = self.t();
                self.line(format!("{si} = fmul double {t}, {}", flit(*scale)));
                t = si;
            }
        }
        Ok((t, sig.ret))
    }

    fn call(&mut self, f: &Expr, args: &[Expr]) -> G<(String, Ty)> {
        match f {
            Expr::Path(m, n, _) => return self.path_call(m, n, args),
            Expr::Ident(_, _) => {}
            _ => return Err("only direct calls are supported natively".into()),
        }
        let Expr::Ident(name, _) = f else {
            unreachable!()
        };
        if self.var(name).is_none() {
            if let Some(si) = self.ctx.sidx(name) {
                return self.construct_struct(si, args);
            }
            if let Some(mi) = self.ctx.midx(name) {
                return self.construct_mach(mi, args);
            }
            if self.sigs.contains_key(name.as_str()) || self.ctx.fn_names.contains(name.as_str()) {
                if !self.sigs.contains_key(name.as_str())
                    && !self.ctx.dropped.borrow().contains_key(name.as_str())
                {
                    return Err(format!("calls {name}(), which is not native (yet)"));
                }
                return self.call_unit(name, name, None, args);
            }
        }
        match name.as_str() {
            "print" | "println" => return self.print_call(args),
            "sleep" | "millis" | "micros" => return self.path_call("time", name, args),
            "len" => {
                let [a] = args else {
                    return Err("len() takes 1 argument".into());
                };
                if let Expr::Str(s) = a {
                    return Ok((s.chars().count().to_string(), Ty::I64));
                }
                let v = self.expr(a)?;
                return match v.1 {
                    Ty::Arr(n) => Ok((n.to_string(), Ty::I64)),
                    other => Err(format!("len() of {} is not native", self.tyn(other))),
                };
            }
            "format" => return Err("format() produces a string; natively it is only available inside print() and f-strings".into()),
            "implements" => {
                // decided at compile time: native values have static types
                let [x, Expr::Str(want)] = args else {
                    return Err("natively implements() needs an interface name literal".into());
                };
                if !self.ctx.interfaces.contains(want.as_str()) {
                    return Err(format!("implements(): unknown interface '{want}'"));
                }
                let v = self.expr(x)?;
                let yes = match v.1 {
                    Ty::Obj(si) => {
                        let mut todo = self.ctx.structs[si as usize].def.implements.clone();
                        let mut seen = HashSet::new();
                        let mut hit = false;
                        while let Some(n) = todo.pop() {
                            if &n == want {
                                hit = true;
                                break;
                            }
                            if seen.insert(n.clone()) {
                                todo.extend(self.ctx.iface_bases.get(&n).cloned().unwrap_or_default());
                            }
                        }
                        hit
                    }
                    _ => false,
                };
                return Ok(((yes as u8).to_string(), Ty::Bool));
            }
            _ => {}
        }
        let mut vals = Vec::new();
        for a in args {
            if matches!(a, Expr::Named(..)) {
                return Err("keyword arguments are not supported natively".into());
            }
            vals.push(self.expr(a)?);
        }
        if vals
            .iter()
            .any(|v| v.1.ll() == "ptr" || matches!(v.1, Ty::State(_)))
        {
            return Err(format!("call to '{name}' is not supported natively"));
        }
        self.builtin(name, vals)
    }

    fn arg(&mut self, args: &[Expr], i: usize) -> G<Option<(String, Ty)>> {
        match args.get(i) {
            Some(Expr::Named(..)) => Err("keyword arguments are not supported natively".into()),
            Some(e) => Ok(Some(self.expr(e)?)),
            None => Ok(None),
        }
    }

    /// An integer literal argument (a length known at compile time).
    fn lit_count(&self, args: &[Expr], i: usize, what: &str) -> G<Option<u16>> {
        match args.get(i) {
            None => Ok(None),
            Some(Expr::Int(n)) if (0..=4096).contains(n) => Ok(Some(*n as u16)),
            Some(_) => Err(format!(
                "{what}: natively the count must be an integer literal (it sizes the result list)"
            )),
        }
    }

    fn seconds(&mut self, v: (String, Ty), plain_scale: f64, what: &str) -> G<String> {
        match v.1 {
            Ty::Q(d) if d == DIM_S => Ok(v.0),
            Ty::Q(d) => Err(format!(
                "{what} expects a time, got {}",
                units::describe(&d)
            )),
            _ => {
                let x = self.convert(v, Ty::F64)?;
                if plain_scale == 1.0 {
                    return Ok(x);
                }
                let t = self.t();
                self.line(format!("{t} = fmul double {x}, {}", flit(plain_scale)));
                Ok(t)
            }
        }
    }

    fn path_call(&mut self, m: &str, n: &str, args: &[Expr]) -> G<(String, Ty)> {
        if n == "new" {
            if let Some(d) = Dev::from_name(m) {
                return self.construct_dev(d, args);
            }
        }
        match (m, n) {
            ("time", "sleep") | ("time", "delay") => {
                let v = self.arg(args, 0)?.ok_or("sleep() needs a duration")?;
                let s = self.seconds(v, 1.0, "sleep()")?;
                self.rt("declare void @tulpa_sleep(double)");
                self.line(format!("call void @tulpa_sleep(double {s})"));
                Ok(("0".into(), Ty::I64))
            }
            ("time", "millis") | ("time", "micros") => {
                let d = if n == "millis" {
                    "declare i64 @tulpa_millis()"
                } else {
                    "declare i64 @tulpa_micros()"
                };
                self.rt(d);
                let t = self.t();
                self.line(format!("{t} = call i64 @tulpa_{n}()"));
                Ok((t, Ty::I64))
            }
            ("tlm", "enabled") => Ok(("0".into(), Ty::Bool)),
            ("tlm", _) => {
                // telemetry is a HermiSim/interpreter feature; on hardware the calls are no-ops
                for i in 0..args.len() {
                    if !matches!(args[i], Expr::Str(_)) {
                        self.arg(args, i)?;
                    }
                }
                Ok(("0".into(), Ty::I64))
            }
            ("sim", _) => self.sim_call(n, args),
            ("math", f) => {
                let mut vals = Vec::new();
                for i in 0..args.len() {
                    vals.push(self.arg(args, i)?.unwrap());
                }
                self.builtin(f, vals)
            }
            _ => Err(format!("{m}::{n} is not supported natively")),
        }
    }

    fn sim_call(&mut self, n: &str, args: &[Expr]) -> G<(String, Ty)> {
        if !self.ctx.opts.sim {
            return Err(format!(
                "sim::{n} needs the simulation HAL (build with --target sim)"
            ));
        }
        let bus_or = |g: &mut Self, i: usize, d: &str| -> G<String> {
            match g.arg(args, i)? {
                Some(v) => g.convert(v, Ty::I64),
                None => Ok(d.to_string()),
            }
        };
        match n {
            "i2c_set" => {
                let addr = self
                    .arg(args, 0)?
                    .ok_or("sim::i2c_set(addr, reg, bytes[, bus])")?;
                let addr = self.convert(addr, Ty::I64)?;
                let reg = self
                    .arg(args, 1)?
                    .ok_or("sim::i2c_set(addr, reg, bytes[, bus])")?;
                let reg = self.convert(reg, Ty::I64)?;
                let data = self
                    .arg(args, 2)?
                    .ok_or("sim::i2c_set(addr, reg, bytes[, bus])")?;
                let (dp, dn) = self.bytes_arg(data)?;
                let bus = bus_or(self, 3, "1")?;
                self.rt("declare void @tulpa_sim_i2c_set(i64, i64, i64, ptr, i64)");
                self.line(format!("call void @tulpa_sim_i2c_set(i64 {bus}, i64 {addr}, i64 {reg}, ptr {dp}, i64 {dn})"));
                Ok(("0".into(), Ty::I64))
            }
            "i2c_get" => {
                let addr = self
                    .arg(args, 0)?
                    .ok_or("sim::i2c_get(addr, reg[, n[, bus]])")?;
                let addr = self.convert(addr, Ty::I64)?;
                let reg = self
                    .arg(args, 1)?
                    .ok_or("sim::i2c_get(addr, reg[, n[, bus]])")?;
                let reg = self.convert(reg, Ty::I64)?;
                let n = self.lit_count(args, 2, "sim::i2c_get")?.unwrap_or(1);
                let bus = bus_or(self, 3, "1")?;
                let out = self.alloc_obj(Ty::Arr(n));
                self.rt("declare void @tulpa_sim_i2c_get(i64, i64, i64, ptr, i64)");
                self.line(format!("call void @tulpa_sim_i2c_get(i64 {bus}, i64 {addr}, i64 {reg}, ptr {out}, i64 {n})"));
                Ok((out, Ty::Arr(n)))
            }
            "gpio_input" => {
                let pin = self.arg(args, 0)?.ok_or("sim::gpio_input(pin, level)")?;
                let pin = self.convert(pin, Ty::I64)?;
                let lv = self.arg(args, 1)?.ok_or("sim::gpio_input(pin, level)")?;
                let lv = self.convert(lv, Ty::Bool)?;
                let l = self.t();
                self.line(format!("{l} = zext i1 {lv} to i64"));
                self.rt("declare void @tulpa_sim_gpio_input(i64, i64)");
                self.line(format!(
                    "call void @tulpa_sim_gpio_input(i64 {pin}, i64 {l})"
                ));
                self.after_effect()?;
                Ok(("0".into(), Ty::I64))
            }
            "gpio_level" => {
                let pin = self.arg(args, 0)?.ok_or("sim::gpio_level(pin)")?;
                let pin = self.convert(pin, Ty::I64)?;
                self.rt("declare i64 @tulpa_sim_gpio_level(i64)");
                let t = self.t();
                self.line(format!("{t} = call i64 @tulpa_sim_gpio_level(i64 {pin})"));
                Ok((t, Ty::I64))
            }
            "spi_respond" => {
                let bus = self
                    .arg(args, 0)?
                    .ok_or("sim::spi_respond(bus, cs, bytes)")?;
                let bus = self.convert(bus, Ty::I64)?;
                let cs = self
                    .arg(args, 1)?
                    .ok_or("sim::spi_respond(bus, cs, bytes)")?;
                let cs = self.convert(cs, Ty::I64)?;
                let data = self
                    .arg(args, 2)?
                    .ok_or("sim::spi_respond(bus, cs, bytes)")?;
                let (dp, dn) = self.bytes_arg(data)?;
                self.rt("declare void @tulpa_sim_spi_respond(i64, i64, ptr, i64)");
                self.line(format!(
                    "call void @tulpa_sim_spi_respond(i64 {bus}, i64 {cs}, ptr {dp}, i64 {dn})"
                ));
                Ok(("0".into(), Ty::I64))
            }
            // the simulation HAL always runs on a virtual clock
            "virtual_time" => {
                for i in 0..args.len() {
                    self.arg(args, i)?;
                }
                Ok(("0".into(), Ty::I64))
            }
            "backend" => Ok((self.cstr("sim"), Ty::Str)),
            _ => Err(format!("sim::{n} is not supported natively")),
        }
    }

    /// A byte-list argument: an array, or a single integer byte.
    fn bytes_arg(&mut self, v: (String, Ty)) -> G<(String, u16)> {
        match v.1 {
            Ty::Arr(n) => Ok((v.0, n)),
            Ty::I64 => {
                let arr = self.alloc_obj(Ty::Arr(1));
                self.line(format!("store i64 {}, ptr {arr}", v.0));
                Ok((arr, 1))
            }
            other => Err(format!(
                "expected bytes (a list of ints), got {}",
                self.tyn(other)
            )),
        }
    }

    fn construct_dev(&mut self, d: Dev, args: &[Expr]) -> G<(String, Ty)> {
        let int_or = |g: &mut Self, i: usize, def: i64, what: &str| -> G<String> {
            match g.arg(args, i)? {
                Some(v) => g.i32_of(v, what),
                None => Ok(def.to_string()),
            }
        };
        let dirarg = |g: &Self, i: usize| -> G<Option<i32>> {
            match args.get(i) {
                None => Ok(None),
                Some(Expr::Str(s)) => match s.as_str() {
                    "out" | "output" => Ok(Some(1)),
                    "in" | "input" => Ok(Some(0)),
                    other => Err(format!(
                        "unknown GPIO direction '{other}' (use \"in\" or \"out\")"
                    )),
                },
                Some(_) => {
                    let _ = g;
                    Err("natively the GPIO direction must be the literal \"in\" or \"out\"".into())
                }
            }
        };
        let obj = self.alloc_obj(Ty::Dev(d));
        match d {
            Dev::Gpio => {
                let pin = int_or(self, 0, -1, "GPIO pin")?;
                let out = dirarg(self, 1)?.unwrap_or(1);
                self.rt("declare void @tulpa_gpio_open(ptr, i32, i32)");
                self.line(format!(
                    "call void @tulpa_gpio_open(ptr {obj}, i32 {pin}, i32 {out})"
                ));
            }
            Dev::I2c => {
                let (bus, addr) = if args.len() == 1 {
                    ("1".to_string(), int_or(self, 0, 0, "I2C address")?)
                } else {
                    (
                        int_or(self, 0, 1, "I2C bus")?,
                        int_or(self, 1, 0, "I2C address")?,
                    )
                };
                self.rt("declare void @tulpa_i2c_open(ptr, i32, i32)");
                self.line(format!(
                    "call void @tulpa_i2c_open(ptr {obj}, i32 {bus}, i32 {addr})"
                ));
            }
            Dev::Spi => {
                let bus = int_or(self, 0, 0, "SPI bus")?;
                let cs = int_or(self, 1, 0, "SPI chip select")?;
                self.rt("declare void @tulpa_spi_open(ptr, i32, i32)");
                self.line(format!(
                    "call void @tulpa_spi_open(ptr {obj}, i32 {bus}, i32 {cs})"
                ));
            }
            Dev::Uart => {
                let port = match args.first() {
                    Some(Expr::Str(s)) => self.cstr(s),
                    Some(Expr::Int(n)) => self.cstr(&n.to_string()),
                    _ => return Err("natively UART::new takes a port name literal, e.g. UART::new(\"uart0\", 115200)".into()),
                };
                let baud = int_or(self, 1, 9600, "UART baud")?;
                self.rt("declare void @tulpa_uart_open(ptr, ptr, i32)");
                self.line(format!(
                    "call void @tulpa_uart_open(ptr {obj}, ptr {port}, i32 {baud})"
                ));
            }
            Dev::Pwm => {
                let chip = int_or(self, 0, 0, "PWM chip")?;
                let ch = int_or(self, 1, 0, "PWM channel")?;
                self.rt("declare void @tulpa_pwm_open(ptr, i32, i32)");
                self.line(format!(
                    "call void @tulpa_pwm_open(ptr {obj}, i32 {chip}, i32 {ch})"
                ));
            }
            Dev::Servo => {
                let chip = int_or(self, 0, 0, "Servo chip")?;
                let ch = int_or(self, 1, 0, "Servo channel")?;
                let mut fl = Vec::new();
                for (i, def) in [(2usize, 500.0f64), (3, 2500.0), (4, 180.0)] {
                    let v = match self.arg(args, i)? {
                        Some(v) => self.f64_of(v, "Servo::new")?,
                        None => flit(def),
                    };
                    fl.push(v);
                }
                self.rt("declare void @tulpa_servo_open(ptr, i32, i32, double, double, double)");
                self.line(format!(
                    "call void @tulpa_servo_open(ptr {obj}, i32 {chip}, i32 {ch}, double {}, double {}, double {})",
                    fl[0], fl[1], fl[2]
                ));
            }
        }
        self.after_effect()?;
        Ok((obj, Ty::Dev(d)))
    }

    fn is_module_alias(&self, o: &Expr) -> bool {
        match o {
            Expr::Ident(a, _) => {
                self.ctx.opts.module_aliases.iter().any(|x| x == a)
                    && self.var(a).is_none()
                    && !self.ctx.globals.borrow().contains_key(a)
            }
            _ => false,
        }
    }

    fn method_call(&mut self, o: &Expr, m: &str, args: &[Expr]) -> G<(String, Ty)> {
        if self.is_module_alias(o) {
            return self.call(&Expr::Ident(m.to_string(), 0), args);
        }
        let ov = self.expr(o)?;
        match ov.1 {
            Ty::Obj(si) => {
                let st = &self.ctx.structs[si as usize];
                let sname = st.name.clone();
                if !st.def.methods.iter().any(|x| x.name == m) {
                    return Err(format!("{sname} has no method '{m}'"));
                }
                self.call_unit(
                    &format!("{sname}_{m}"),
                    &format!("{sname}.{m}"),
                    Some(ov),
                    args,
                )
            }
            Ty::Mach(mi) => self.mach_call(mi, ov.0, m, args),
            Ty::Dev(d) => {
                let r = self.dev_call(d, &ov.0, m, args)?;
                Ok(r)
            }
            other => Err(format!(
                "method '{m}' on {} is not supported natively",
                self.tyn(other)
            )),
        }
    }

    fn mach_call(&mut self, mi: u16, obj: String, m: &str, args: &[Expr]) -> G<(String, Ty)> {
        self.mach_complete(mi)?;
        let mname = self.ctx.machines[mi as usize].name.clone();
        let def = self.ctx.machines[mi as usize].def.clone();
        let is_event = |e: &str| {
            def.states
                .iter()
                .flat_map(|s| s.transitions.iter())
                .chain(def.global.iter())
                .any(|t| t.event == e)
        };
        let state_index = |name: &str| def.states.iter().position(|s| s.name == name);
        let (ev, rest): (String, &[Expr]) =
            if is_event(m) && !matches!(m, "send" | "fire" | "can" | "is" | "in_state") {
                (m.to_string(), args)
            } else {
                match m {
                    "send" | "fire" => match args.first() {
                        Some(Expr::Str(e)) => (e.clone(), &args[1..]),
                        _ => {
                            return Err(
                                "natively send() needs the event name as a string literal".into()
                            )
                        }
                    },
                    "is" | "in_state" => {
                        let Some(Expr::Str(sn)) = args.first() else {
                            return Err(format!("natively {m}() needs a state name literal"));
                        };
                        let k = state_index(sn)
                            .ok_or_else(|| format!("machine {mname} has no state '{sn}'"))?;
                        let p = self.field_ptr(&obj, Ty::Mach(mi), 0);
                        let cur = self.t();
                        self.line(format!("{cur} = load i32, ptr {p}"));
                        let t = self.t();
                        self.line(format!("{t} = icmp eq i32 {cur}, {k}"));
                        return Ok((t, Ty::Bool));
                    }
                    "reset" | "restart" => {
                        self.line(format!("call void @tulpa_{mname}__init(ptr {obj})"));
                        return Ok(("0".into(), Ty::I64));
                    }
                    other => {
                        return Err(format!(
                            "machine {mname} has no event or native method '{other}'"
                        ))
                    }
                }
            };
        if !is_event(&ev) {
            return Err(format!("machine {mname} has no event '{ev}'"));
        }
        // learn the event's parameter types from this call site
        let mut vals = Vec::new();
        for i in 0..rest.len() {
            vals.push(self.arg(rest, i)?.unwrap());
        }
        {
            let key = (mi, ev.clone());
            let known = self.ctx.event_tys.borrow().get(&key).cloned();
            if known.is_none() {
                self.ctx
                    .event_tys
                    .borrow_mut()
                    .insert(key, vals.iter().map(|v| v.1).collect());
                *self.ctx.learned.borrow_mut() = true;
            }
        }
        let sym = format!("{mname}__on_{ev}");
        let Some(sig) = self.sigs.get(&sym).cloned() else {
            let why = self
                .ctx
                .dropped
                .borrow()
                .get(&sym)
                .cloned()
                .unwrap_or_default();
            return Err(format!("event {mname}.{ev} is not native: {why}"));
        };
        if sig.params.len() != vals.len() + 1 {
            return Err(format!(
                "event '{ev}' expects {} argument(s), got {}",
                sig.params.len() - 1,
                vals.len()
            ));
        }
        let mut argl = vec![format!("ptr {obj}")];
        for (v, (_, t)) in vals.into_iter().zip(sig.params.iter().skip(1)) {
            let c = if t.is_ref() {
                v.0
            } else {
                self.convert(v, *t)?
            };
            argl.push(format!("{} {c}", t.ll()));
        }
        let t = self.t();
        self.line(format!("{t} = call i1 @tulpa_{sym}({})", argl.join(", ")));
        Ok((t, Ty::Bool))
    }

    /// Device methods → the runtime ABI in tulpa_rt.c. Monitors are re-checked afterwards.
    fn dev_call(&mut self, d: Dev, obj: &str, m: &str, args: &[Expr]) -> G<(String, Ty)> {
        let unit = || ("0".to_string(), Ty::I64);
        let need = |g: &mut Self, i: usize, what: &str| -> G<(String, Ty)> {
            g.arg(args, i)?
                .ok_or_else(|| format!("{}.{m}() needs {what}", d.display()))
        };
        let r: (String, Ty) = match (d, m) {
            (Dev::Gpio, "set_high" | "on") | (Dev::Gpio, "set_low" | "off") => {
                let lv = if matches!(m, "set_high" | "on") { 1 } else { 0 };
                self.rt("declare void @tulpa_gpio_write(ptr, i32)");
                self.line(format!("call void @tulpa_gpio_write(ptr {obj}, i32 {lv})"));
                unit()
            }
            (Dev::Gpio, "write" | "set") => {
                let v = need(self, 0, "a level")?;
                let b = self.convert(v, Ty::Bool)?;
                let l = self.t();
                self.line(format!("{l} = zext i1 {b} to i32"));
                self.rt("declare void @tulpa_gpio_write(ptr, i32)");
                self.line(format!("call void @tulpa_gpio_write(ptr {obj}, i32 {l})"));
                unit()
            }
            (Dev::Gpio, "read" | "value" | "is_high" | "is_low") => {
                self.rt("declare i64 @tulpa_gpio_read(ptr)");
                let t = self.t();
                self.line(format!("{t} = call i64 @tulpa_gpio_read(ptr {obj})"));
                match m {
                    "is_high" | "is_low" => {
                        let b = self.t();
                        self.line(format!(
                            "{b} = icmp {} i64 {t}, 0",
                            if m == "is_high" { "ne" } else { "eq" }
                        ));
                        (b, Ty::Bool)
                    }
                    _ => (t, Ty::I64),
                }
            }
            (Dev::Gpio, "toggle") => {
                self.rt("declare void @tulpa_gpio_toggle(ptr)");
                self.line(format!("call void @tulpa_gpio_toggle(ptr {obj})"));
                unit()
            }
            (Dev::Gpio, "set_direction") => {
                let out = match args.first() {
                    Some(Expr::Str(s)) if matches!(s.as_str(), "out" | "output") => 1,
                    Some(Expr::Str(s)) if matches!(s.as_str(), "in" | "input") => 0,
                    _ => return Err("natively set_direction() takes \"in\" or \"out\"".into()),
                };
                self.rt("declare void @tulpa_gpio_set_direction(ptr, i32)");
                self.line(format!(
                    "call void @tulpa_gpio_set_direction(ptr {obj}, i32 {out})"
                ));
                unit()
            }
            (Dev::Gpio, "pin") | (Dev::I2c, "address" | "bus") => {
                let k = match m {
                    "address" => 1,
                    _ => 0,
                };
                let p = self.t();
                self.line(format!(
                    "{p} = getelementptr %tulpa.{}, ptr {obj}, i32 0, i32 {k}",
                    d.name()
                ));
                let v = self.t();
                self.line(format!("{v} = load i32, ptr {p}"));
                let w = self.t();
                self.line(format!("{w} = sext i32 {v} to i64"));
                return Ok((w, Ty::I64));
            }
            (Dev::I2c, "write_reg") => {
                let r = need(self, 0, "a register")?;
                let r = self.convert(r, Ty::I64)?;
                let v = need(self, 1, "a value")?;
                match v.1 {
                    Ty::Arr(n) => {
                        self.rt("declare void @tulpa_i2c_write_reg_n(ptr, i64, ptr, i64)");
                        self.line(format!(
                            "call void @tulpa_i2c_write_reg_n(ptr {obj}, i64 {r}, ptr {}, i64 {n})",
                            v.0
                        ));
                    }
                    _ => {
                        let v = self.convert(v, Ty::I64)?;
                        self.rt("declare void @tulpa_i2c_write_reg(ptr, i64, i64)");
                        self.line(format!(
                            "call void @tulpa_i2c_write_reg(ptr {obj}, i64 {r}, i64 {v})"
                        ));
                    }
                }
                unit()
            }
            (Dev::I2c, "read_reg") => {
                let r = need(self, 0, "a register")?;
                let r = self.convert(r, Ty::I64)?;
                self.rt("declare i64 @tulpa_i2c_read_reg(ptr, i64)");
                let t = self.t();
                self.line(format!(
                    "{t} = call i64 @tulpa_i2c_read_reg(ptr {obj}, i64 {r})"
                ));
                (t, Ty::I64)
            }
            (Dev::I2c, "read_regs") => {
                let r = need(self, 0, "a register")?;
                let r = self.convert(r, Ty::I64)?;
                let n = self
                    .lit_count(args, 1, "I2C.read_regs")?
                    .ok_or("I2C.read_regs(reg, count)")?;
                let out = self.alloc_obj(Ty::Arr(n));
                self.rt("declare void @tulpa_i2c_read_regs(ptr, i64, ptr, i64)");
                self.line(format!(
                    "call void @tulpa_i2c_read_regs(ptr {obj}, i64 {r}, ptr {out}, i64 {n})"
                ));
                (out, Ty::Arr(n))
            }
            (Dev::I2c, "read_u16" | "read_i16") => {
                let r = need(self, 0, "a register")?;
                let r = self.convert(r, Ty::I64)?;
                let little = matches!(args.get(1), Some(Expr::Str(s)) if s == "little") as i32;
                let signed = (m == "read_i16") as i32;
                self.rt("declare i64 @tulpa_i2c_read_u16(ptr, i64, i32, i32)");
                let t = self.t();
                self.line(format!("{t} = call i64 @tulpa_i2c_read_u16(ptr {obj}, i64 {r}, i32 {little}, i32 {signed})"));
                (t, Ty::I64)
            }
            (Dev::I2c, "update_reg") => {
                let mut v = Vec::new();
                for i in 0..3 {
                    let x = need(self, i, "(reg, mask, value)")?;
                    v.push(self.convert(x, Ty::I64)?);
                }
                self.rt("declare void @tulpa_i2c_update_reg(ptr, i64, i64, i64)");
                self.line(format!(
                    "call void @tulpa_i2c_update_reg(ptr {obj}, i64 {}, i64 {}, i64 {})",
                    v[0], v[1], v[2]
                ));
                unit()
            }
            (Dev::I2c, "write") | (Dev::Spi, "write") => {
                let data = need(self, 0, "bytes")?;
                let (p, n) = self.bytes_arg(data)?;
                let (f, decl) = if d == Dev::I2c {
                    (
                        "tulpa_i2c_write",
                        "declare void @tulpa_i2c_write(ptr, ptr, i64)",
                    )
                } else {
                    (
                        "tulpa_spi_write",
                        "declare void @tulpa_spi_write(ptr, ptr, i64)",
                    )
                };
                self.rt(decl);
                self.line(format!("call void @{f}(ptr {obj}, ptr {p}, i64 {n})"));
                unit()
            }
            (Dev::I2c, "read") | (Dev::Spi, "read") | (Dev::Uart, "read" | "receive") => {
                let n = self
                    .lit_count(args, 0, &format!("{}.read", d.display()))?
                    .ok_or("read(count)")?;
                let out = self.alloc_obj(Ty::Arr(n));
                let (f, decl) = match d {
                    Dev::I2c => (
                        "tulpa_i2c_read",
                        "declare void @tulpa_i2c_read(ptr, ptr, i64)",
                    ),
                    Dev::Spi => (
                        "tulpa_spi_read",
                        "declare void @tulpa_spi_read(ptr, ptr, i64)",
                    ),
                    _ => (
                        "tulpa_uart_read",
                        "declare void @tulpa_uart_read(ptr, ptr, i64)",
                    ),
                };
                self.rt(decl);
                self.line(format!("call void @{f}(ptr {obj}, ptr {out}, i64 {n})"));
                (out, Ty::Arr(n))
            }
            (Dev::I2c, "write_read") => {
                let data = need(self, 0, "bytes")?;
                let (p, n) = self.bytes_arg(data)?;
                let k = self
                    .lit_count(args, 1, "I2C.write_read")?
                    .ok_or("I2C.write_read(bytes, count)")?;
                let out = self.alloc_obj(Ty::Arr(k));
                self.rt("declare void @tulpa_i2c_write_read(ptr, ptr, i64, ptr, i64)");
                self.line(format!("call void @tulpa_i2c_write_read(ptr {obj}, ptr {p}, i64 {n}, ptr {out}, i64 {k})"));
                (out, Ty::Arr(k))
            }
            (Dev::Spi, "transfer" | "xfer") => {
                let data = need(self, 0, "bytes")?;
                let (p, n) = self.bytes_arg(data)?;
                let out = self.alloc_obj(Ty::Arr(n));
                self.rt("declare void @tulpa_spi_transfer(ptr, ptr, ptr, i64)");
                self.line(format!(
                    "call void @tulpa_spi_transfer(ptr {obj}, ptr {p}, ptr {out}, i64 {n})"
                ));
                (out, Ty::Arr(n))
            }
            (Dev::Spi, "set_speed" | "set_bus_speed") => {
                let v = need(self, 0, "a frequency")?;
                let hz = match v.1 {
                    Ty::Q(dm) if dm == DIM_HZ => {
                        let t = self.t();
                        self.line(format!("{t} = fptosi double {} to i64", v.0));
                        t
                    }
                    Ty::F64 => {
                        let t = self.t();
                        self.line(format!("{t} = fptosi double {} to i64", v.0));
                        t
                    }
                    _ => self.convert(v, Ty::I64)?,
                };
                self.rt("declare void @tulpa_spi_set_speed(ptr, i64)");
                self.line(format!(
                    "call void @tulpa_spi_set_speed(ptr {obj}, i64 {hz})"
                ));
                unit()
            }
            (Dev::Spi, "set_mode") => {
                let v = need(self, 0, "a mode")?;
                let v = self.convert(v, Ty::I64)?;
                self.rt("declare void @tulpa_spi_set_mode(ptr, i64)");
                self.line(format!("call void @tulpa_spi_set_mode(ptr {obj}, i64 {v})"));
                unit()
            }
            (Dev::Uart, "send" | "write" | "send_line" | "println") => {
                let nl = matches!(m, "send_line" | "println") as i32;
                match args.first() {
                    Some(e @ (Expr::Str(_) | Expr::FStr(_))) => {
                        self.rt("declare void @tulpa_out_uart(ptr)");
                        self.rt("declare void @tulpa_out_console()");
                        self.line(format!("call void @tulpa_out_uart(ptr {obj})"));
                        let e = e.clone();
                        self.print_item(&e)?;
                        if nl == 1 {
                            self.rt("declare void @tulpa_print_nl()");
                            self.line("call void @tulpa_print_nl()".into());
                        }
                        self.line("call void @tulpa_out_console()".into());
                    }
                    Some(_) => {
                        let v = need(self, 0, "data")?;
                        let (p, n) = self.bytes_arg(v)?;
                        self.rt("declare void @tulpa_uart_write(ptr, ptr, i64)");
                        self.line(format!(
                            "call void @tulpa_uart_write(ptr {obj}, ptr {p}, i64 {n})"
                        ));
                    }
                    None => return Err("UART.send() needs data".into()),
                }
                unit()
            }
            (Dev::Uart, "available") => {
                self.rt("declare i64 @tulpa_uart_available(ptr)");
                let t = self.t();
                self.line(format!("{t} = call i64 @tulpa_uart_available(ptr {obj})"));
                (t, Ty::I64)
            }
            (Dev::Pwm, "frequency" | "set_frequency") => {
                let v = need(self, 0, "a frequency")?;
                let hz = match v.1 {
                    Ty::Q(dm) if dm == DIM_HZ => v.0,
                    Ty::Q(dm) => {
                        return Err(format!(
                            "frequency() expects Hz, got {}",
                            units::describe(&dm)
                        ))
                    }
                    _ => self.convert(v, Ty::F64)?,
                };
                self.rt("declare void @tulpa_pwm_frequency(ptr, double)");
                self.line(format!(
                    "call void @tulpa_pwm_frequency(ptr {obj}, double {hz})"
                ));
                unit()
            }
            (Dev::Pwm, "period") => {
                let v = need(self, 0, "a period")?;
                let s = self.seconds(v, 1e-9, "period()")?;
                self.rt("declare void @tulpa_pwm_period(ptr, double)");
                self.line(format!(
                    "call void @tulpa_pwm_period(ptr {obj}, double {s})"
                ));
                unit()
            }
            (Dev::Pwm, "duty" | "set_duty") => {
                let v = need(self, 0, "a duty fraction")?;
                let f = self.f64_of(v, "duty()")?;
                self.rt("declare void @tulpa_pwm_duty(ptr, double)");
                self.line(format!("call void @tulpa_pwm_duty(ptr {obj}, double {f})"));
                unit()
            }
            (Dev::Pwm, "pulse" | "pulse_width") => {
                let v = need(self, 0, "a pulse width")?;
                let s = self.seconds(v, 1e-6, "pulse()")?;
                self.rt("declare void @tulpa_pwm_pulse(ptr, double)");
                self.line(format!("call void @tulpa_pwm_pulse(ptr {obj}, double {s})"));
                unit()
            }
            (Dev::Pwm, "enable" | "on" | "disable" | "off") => {
                let on = matches!(m, "enable" | "on") as i32;
                self.rt("declare void @tulpa_pwm_enable(ptr, i32)");
                self.line(format!("call void @tulpa_pwm_enable(ptr {obj}, i32 {on})"));
                unit()
            }
            (Dev::Servo, "angle" | "write" | "move_to") if args.is_empty() => {
                self.rt("declare double @tulpa_servo_get_angle(ptr)");
                let t = self.t();
                self.line(format!(
                    "{t} = call double @tulpa_servo_get_angle(ptr {obj})"
                ));
                return Ok((t, Ty::Q(DIM_RAD)));
            }
            (Dev::Servo, "angle" | "write" | "move_to") => {
                let v = need(self, 0, "an angle")?;
                let rad = match v.1 {
                    Ty::Q(dm) if dm == DIM_RAD => v.0,
                    Ty::Q(dm) => {
                        return Err(format!(
                            "Servo.angle expects an angle, got {}",
                            units::describe(&dm)
                        ))
                    }
                    _ => {
                        // plain numbers are degrees, as in the interpreter
                        let x = self.convert(v, Ty::F64)?;
                        let t = self.t();
                        self.line(format!(
                            "{t} = fmul double {x}, {}",
                            flit(std::f64::consts::PI / 180.0)
                        ));
                        t
                    }
                };
                self.rt("declare void @tulpa_servo_angle(ptr, double)");
                self.line(format!(
                    "call void @tulpa_servo_angle(ptr {obj}, double {rad})"
                ));
                unit()
            }
            (Dev::Servo, "center" | "detach") => {
                let (f, decl) = if m == "center" {
                    (
                        "tulpa_servo_center",
                        "declare void @tulpa_servo_center(ptr)",
                    )
                } else {
                    (
                        "tulpa_servo_detach",
                        "declare void @tulpa_servo_detach(ptr)",
                    )
                };
                self.rt(decl);
                self.line(format!("call void @{f}(ptr {obj})"));
                unit()
            }
            (Dev::Servo, "pulse_us") => {
                self.rt("declare double @tulpa_servo_pulse_us(ptr)");
                let t = self.t();
                self.line(format!(
                    "{t} = call double @tulpa_servo_pulse_us(ptr {obj})"
                ));
                return Ok((t, Ty::F64));
            }
            _ => return Err(format!("{}.{m}() is not supported natively", d.display())),
        };
        self.after_effect()?;
        Ok(r)
    }

    // ------------------------------------------------------------------ macros

    fn macro_call(&mut self, name: &str, args: &[MacroArg]) -> G<(String, Ty)> {
        let expr_arg = |a: &MacroArg| -> G<Expr> {
            match a {
                MacroArg::Expr(e) => Ok(e.clone()),
                MacroArg::Block(_) => Err("expected an expression".into()),
            }
        };
        let body_of = |a: &MacroArg| -> Vec<Stmt> {
            match a {
                MacroArg::Block(b) => b.clone(),
                MacroArg::Expr(Expr::Block(b)) => b.clone(),
                MacroArg::Expr(e) => vec![Stmt::Expr(e.clone(), 0)],
            }
        };
        match name {
            "every" => {
                if args.len() != 2 && args.len() != 3 {
                    return Err("every! expects (period, body) or (period, count, body)".into());
                }
                let pv = self.expr(&expr_arg(&args[0])?)?;
                // plain numbers are milliseconds, as in the interpreter
                let period_s = self.seconds(pv, 1e-3, "every! period")?;
                let us = self.t();
                self.line(format!("{us} = fmul double {period_s}, 1.0e6"));
                let usi = self.t();
                self.line(format!("{usi} = fptosi double {us} to i64"));
                let count = if args.len() == 3 {
                    let c = self.expr(&expr_arg(&args[1])?)?;
                    Some(
                        self.convert(c, Ty::I64)
                            .map_err(|e| format!("every! count: {e}"))?,
                    )
                } else {
                    None
                };
                let body = body_of(args.last().unwrap());
                self.rt("declare void @tulpa_every_start(ptr)");
                self.rt("declare void @tulpa_every_wait(ptr, i64)");
                self.scopes.push(HashMap::new());
                let next = self.declare("\0next", Ty::I64);
                let ctr = self.declare("\0every", Ty::I64);
                self.line(format!("store i64 0, ptr {ctr}"));
                self.line(format!("call void @tulpa_every_start(ptr {next})"));
                let head = self.label("every");
                let bodyl = self.label("ebody");
                let step = self.label("estep");
                let end = self.label("eend");
                self.term(format!("br label %{head}"));
                self.start(&head);
                match &count {
                    Some(c) => {
                        let i = self.t();
                        self.line(format!("{i} = load i64, ptr {ctr}"));
                        let go = self.t();
                        self.line(format!("{go} = icmp slt i64 {i}, {c}"));
                        self.term(format!("br i1 {go}, label %{bodyl}, label %{end}"));
                    }
                    None => self.term(format!("br label %{bodyl}")),
                }
                self.start(&bodyl);
                self.loops.push((step.clone(), end.clone()));
                self.block(&body, false)?;
                self.loops.pop();
                self.term(format!("br label %{step}"));
                self.start(&step);
                let i = self.t();
                self.line(format!("{i} = load i64, ptr {ctr}"));
                let i2 = self.t();
                self.line(format!("{i2} = add i64 {i}, 1"));
                self.line(format!("store i64 {i2}, ptr {ctr}"));
                self.after_effect()?;
                self.line(format!(
                    "call void @tulpa_every_wait(ptr {next}, i64 {usi})"
                ));
                self.term(format!("br label %{head}"));
                self.start(&end);
                let n = self.t();
                self.line(format!("{n} = load i64, ptr {ctr}"));
                self.scopes.pop();
                Ok((n, Ty::I64))
            }
            "repeat" | "simple_loop" => {
                if args.len() != 2 {
                    return Err(format!("{name}! expects (count, body)"));
                }
                let c = self.expr(&expr_arg(&args[0])?)?;
                let c = self.convert(c, Ty::I64)?;
                let body = body_of(&args[1]);
                self.scopes.push(HashMap::new());
                let ctr = self.declare("\0rep", Ty::I64);
                self.line(format!("store i64 0, ptr {ctr}"));
                let head = self.label("rep");
                let bodyl = self.label("rbody");
                let step = self.label("rstep");
                let end = self.label("rend");
                self.term(format!("br label %{head}"));
                self.start(&head);
                let i = self.t();
                self.line(format!("{i} = load i64, ptr {ctr}"));
                let go = self.t();
                self.line(format!("{go} = icmp slt i64 {i}, {c}"));
                self.term(format!("br i1 {go}, label %{bodyl}, label %{end}"));
                self.start(&bodyl);
                self.loops.push((step.clone(), end.clone()));
                self.block(&body, false)?;
                self.loops.pop();
                self.term(format!("br label %{step}"));
                self.start(&step);
                let j = self.t();
                self.line(format!("{j} = load i64, ptr {ctr}"));
                let j2 = self.t();
                self.line(format!("{j2} = add i64 {j}, 1"));
                self.line(format!("store i64 {j2}, ptr {ctr}"));
                self.term(format!("br label %{head}"));
                self.start(&end);
                self.scopes.pop();
                Ok(("0".into(), Ty::I64))
            }
            _ => Err(format!("macro {name}! is not supported natively")),
        }
    }

    // ------------------------------------------------------------------ printing

    fn print_call(&mut self, args: &[Expr]) -> G<(String, Ty)> {
        self.rt("declare void @tulpa_print_str(ptr)");
        self.rt("declare void @tulpa_print_nl()");
        for (i, a) in args.iter().enumerate() {
            if matches!(a, Expr::Named(..)) {
                return Err("print() keyword arguments are not supported natively".into());
            }
            if i > 0 {
                let sp = self.cstr(" ");
                self.line(format!("call void @tulpa_print_str(ptr {sp})"));
            }
            self.print_item(a)?;
        }
        self.line("call void @tulpa_print_nl()".into());
        Ok(("0".into(), Ty::I64))
    }

    fn print_item(&mut self, e: &Expr) -> G<()> {
        self.rt("declare void @tulpa_print_str(ptr)");
        match e {
            Expr::Str(s) => {
                let p = self.cstr(s);
                self.line(format!("call void @tulpa_print_str(ptr {p})"));
            }
            Expr::FStr(parts) => {
                for p in parts {
                    match p {
                        FPart::Lit(s) => {
                            if !s.is_empty() {
                                let c = self.cstr(s);
                                self.line(format!("call void @tulpa_print_str(ptr {c})"));
                            }
                        }
                        FPart::Expr(x) => self.print_item(x)?,
                    }
                }
            }
            Expr::Call(f, a, _) if matches!(&**f, Expr::Ident(n, _) if n == "format") => match a
                .as_slice()
            {
                [x, Expr::Str(spec)] => {
                    if let Some((v, name)) = self.display_unit(x)? {
                        self.print_fmt_unit((v, Ty::F64), spec, Some(name))?;
                    } else {
                        let v = self.expr(x)?;
                        self.print_fmt(v, spec)?;
                    }
                }
                _ => {
                    return Err("natively format() needs a literal spec: format(x, \".2f\")".into())
                }
            },
            other => {
                if let Some((v, name)) = self.display_unit(other)? {
                    self.rt("declare void @tulpa_print_q(double, ptr)");
                    let u = self.cstr(&name);
                    self.line(format!("call void @tulpa_print_q(double {v}, ptr {u})"));
                } else {
                    let v = self.expr(other)?;
                    self.print_value(v)?;
                }
            }
        }
        Ok(())
    }

    /// `q.to("unit")` being printed: the value scaled into that unit plus the unit's name,
    /// so native output shows the same display unit as the interpreter.
    fn display_unit(&mut self, e: &Expr) -> G<Option<(String, String)>> {
        let Expr::MethodCall(o, m, args, _) = e else {
            return Ok(None);
        };
        let [Expr::Str(unit)] = args.as_slice() else {
            return Ok(None);
        };
        if m != "to" {
            return Ok(None);
        }
        let v = self.expr(o)?;
        let u = units::parse(unit)?;
        if dim_of(v.1) != u.dim {
            return Err(format!(
                "cannot convert {} to '{unit}' ({})",
                units::describe(&dim_of(v.1)),
                units::describe(&u.dim)
            ));
        }
        let t = self.t();
        self.line(format!("{t} = fdiv double {}, {}", v.0, flit(u.scale)));
        Ok(Some((t, u.name.clone())))
    }

    fn print_fmt(&mut self, v: (String, Ty), spec: &str) -> G<()> {
        self.print_fmt_unit(v, spec, None)
    }

    fn print_value(&mut self, v: (String, Ty)) -> G<()> {
        match v.1 {
            Ty::I64 => {
                self.rt("declare void @tulpa_print_i64(i64)");
                self.line(format!("call void @tulpa_print_i64(i64 {})", v.0));
            }
            Ty::F64 => {
                self.rt("declare void @tulpa_print_f64(double)");
                self.line(format!("call void @tulpa_print_f64(double {})", v.0));
            }
            Ty::Bool => {
                self.rt("declare void @tulpa_print_bool(i32)");
                let z = self.t();
                self.line(format!("{z} = zext i1 {} to i32", v.0));
                self.line(format!("call void @tulpa_print_bool(i32 {z})"));
            }
            Ty::Q(d) => {
                self.rt("declare void @tulpa_print_q(double, ptr)");
                let u = self.cstr(&units::dim_name(&d));
                self.line(format!("call void @tulpa_print_q(double {}, ptr {u})", v.0));
            }
            Ty::Str => {
                self.line(format!("call void @tulpa_print_str(ptr {})", v.0));
            }
            Ty::Arr(n) => {
                self.rt("declare void @tulpa_print_arr(ptr, i64)");
                self.line(format!("call void @tulpa_print_arr(ptr {}, i64 {n})", v.0));
            }
            Ty::State(mi) => {
                let m = &self.ctx.machines[mi as usize];
                let (name, n) = (m.name.clone(), m.def.states.len());
                let p = self.t();
                self.line(format!(
                    "{p} = getelementptr [{n} x ptr], ptr @mnames.{name}, i32 0, i32 {}",
                    v.0
                ));
                let s = self.t();
                self.line(format!("{s} = load ptr, ptr {p}"));
                self.line(format!("call void @tulpa_print_str(ptr {s})"));
            }
            other => {
                return Err(format!(
                    "printing a {} is not supported natively",
                    self.tyn(other)
                ))
            }
        }
        Ok(())
    }

    /// `format(x, spec)` inside print / f-strings: `[+][0][width][.prec](f|e|d|x|X|o|b|)`.
    fn print_fmt_unit(&mut self, v: (String, Ty), spec: &str, shown: Option<String>) -> G<()> {
        let bad = || format!("format spec '{spec}' is not supported natively");
        let mut chars = spec.chars().peekable();
        let mut flags = String::new();
        while let Some(&c) = chars.peek() {
            if c == '+' || c == '0' || c == '#' || c == ' ' {
                flags.push(c);
                chars.next();
            } else {
                break;
            }
        }
        let mut width = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                width.push(c);
                chars.next();
            } else {
                break;
            }
        }
        let mut prec = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            let mut p = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    p.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            prec = Some(p);
        }
        let kind = chars.next();
        if chars.next().is_some() {
            return Err(bad());
        }
        let unit = match (shown, v.1) {
            (Some(n), _) => Some(n),
            (None, Ty::Q(d)) => Some(units::dim_name(&d)),
            _ => None,
        };
        let num = if let Ty::Q(_) = v.1 {
            (v.0.clone(), Ty::F64)
        } else {
            v.clone()
        };
        match (kind, num.1) {
            (Some('b'), Ty::I64) => {
                self.rt("declare void @tulpa_print_bin(i64, i32, i32)");
                let w: i32 = width.parse().unwrap_or(0);
                let z = flags.contains('0') as i32;
                if flags.contains('#') {
                    let p = self.cstr("0b");
                    self.line(format!("call void @tulpa_print_str(ptr {p})"));
                }
                self.line(format!(
                    "call void @tulpa_print_bin(i64 {}, i32 {w}, i32 {z})",
                    num.0
                ));
            }
            (Some(k @ ('d' | 'x' | 'X' | 'o')), Ty::I64 | Ty::Bool)
            | (Some(k @ ('x' | 'X' | 'o')), _)
                if num.1 != Ty::F64 =>
            {
                let x = self.convert(num, Ty::I64)?;
                let f = format!("%{flags}{width}ll{k}");
                let fp = self.cstr(&f);
                self.rt("declare void @tulpa_print_fmt_i64(i64, ptr)");
                self.line(format!("call void @tulpa_print_fmt_i64(i64 {x}, ptr {fp})"));
            }
            (None, Ty::I64) if prec.is_none() => {
                let f = format!("%{flags}{width}lld");
                let fp = self.cstr(&f);
                self.rt("declare void @tulpa_print_fmt_i64(i64, ptr)");
                self.line(format!(
                    "call void @tulpa_print_fmt_i64(i64 {}, ptr {fp})",
                    num.0
                ));
            }
            (k @ (None | Some('f') | Some('e') | Some('E') | Some('g')), Ty::I64 | Ty::F64) => {
                let x = self.convert(num, Ty::F64)?;
                let kch = k.unwrap_or('f');
                let p = prec.map(|p| format!(".{p}")).unwrap_or_default();
                if k.is_none() && p.is_empty() {
                    self.print_value((x, Ty::F64))?;
                } else {
                    let f = format!("%{flags}{width}{p}{kch}");
                    let fp = self.cstr(&f);
                    self.rt("declare void @tulpa_print_fmt_f64(double, ptr)");
                    self.line(format!(
                        "call void @tulpa_print_fmt_f64(double {x}, ptr {fp})"
                    ));
                }
            }
            _ => return Err(bad()),
        }
        if let Some(u) = unit {
            let s = self.cstr(&format!(" {u}"));
            self.line(format!("call void @tulpa_print_str(ptr {s})"));
        }
        Ok(())
    }

    fn binop(&mut self, op: &str, a: (String, Ty), b: (String, Ty)) -> G<(String, Ty)> {
        for x in [a.1, b.1] {
            if x.ll() == "ptr" || matches!(x, Ty::State(_)) {
                return Err(format!(
                    "operator '{op}' is not defined natively for {}",
                    self.tyn(x)
                ));
            }
        }
        if matches!(a.1, Ty::Q(_)) || matches!(b.1, Ty::Q(_)) {
            return self.qbinop(op, a, b);
        }
        let both_int = a.1 == Ty::I64 && b.1 == Ty::I64;
        let both_bool = a.1 == Ty::Bool && b.1 == Ty::Bool;
        let t = self.t();
        match op {
            "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                if both_bool && matches!(op, "==" | "!=") {
                    self.line(format!(
                        "{t} = icmp {} i1 {}, {}",
                        if op == "==" { "eq" } else { "ne" },
                        a.0,
                        b.0
                    ));
                } else if both_int {
                    let cc = match op {
                        "==" => "eq",
                        "!=" => "ne",
                        "<" => "slt",
                        "<=" => "sle",
                        ">" => "sgt",
                        _ => "sge",
                    };
                    self.line(format!("{t} = icmp {cc} i64 {}, {}", a.0, b.0));
                } else {
                    let x = self.convert(a, Ty::F64)?;
                    let y = self.convert(b, Ty::F64)?;
                    let cc = match op {
                        "==" => "oeq",
                        "!=" => "une",
                        "<" => "olt",
                        "<=" => "ole",
                        ">" => "ogt",
                        _ => "oge",
                    };
                    self.line(format!("{t} = fcmp {cc} double {x}, {y}"));
                }
                Ok((t, Ty::Bool))
            }
            "+" | "-" | "*" if both_int => {
                let which = match op {
                    "+" => "sadd",
                    "-" => "ssub",
                    _ => "smul",
                };
                let r = self.overflow_op(which, &a.0, &b.0, op)?;
                Ok((r, Ty::I64))
            }
            "/" | "%" if both_int => {
                let nz = self.t();
                self.line(format!("{nz} = icmp ne i64 {}, 0", b.0));
                let name = self.name.clone();
                self.check(&nz, format!("division by zero in {name}()"));
                let q = self.t();
                self.line(format!("{q} = sdiv i64 {}, {}", a.0, b.0));
                let r = self.t();
                self.line(format!("{r} = srem i64 {}, {}", a.0, b.0));
                if op == "/" {
                    // Euclidean division (matches the interpreter's div_euclid):
                    // r < 0 ? (b > 0 ? q - 1 : q + 1) : q
                    let rn = self.t();
                    self.line(format!("{rn} = icmp slt i64 {r}, 0"));
                    let bp = self.t();
                    self.line(format!("{bp} = icmp sgt i64 {}, 0", b.0));
                    let qm = self.t();
                    self.line(format!("{qm} = sub i64 {q}, 1"));
                    let qp = self.t();
                    self.line(format!("{qp} = add i64 {q}, 1"));
                    let adj = self.t();
                    self.line(format!("{adj} = select i1 {bp}, i64 {qm}, i64 {qp}"));
                    let out = self.t();
                    self.line(format!("{out} = select i1 {rn}, i64 {adj}, i64 {q}"));
                    Ok((out, Ty::I64))
                } else {
                    // Euclidean remainder: r < 0 ? r + |b| : r
                    let rn = self.t();
                    self.line(format!("{rn} = icmp slt i64 {r}, 0"));
                    let bneg = self.t();
                    self.line(format!("{bneg} = sub i64 0, {}", b.0));
                    let bn = self.t();
                    self.line(format!("{bn} = icmp slt i64 {}, 0", b.0));
                    let babs = self.t();
                    self.line(format!("{babs} = select i1 {bn}, i64 {bneg}, i64 {}", b.0));
                    let fixed = self.t();
                    self.line(format!("{fixed} = add i64 {r}, {babs}"));
                    let out = self.t();
                    self.line(format!("{out} = select i1 {rn}, i64 {fixed}, i64 {r}"));
                    Ok((out, Ty::I64))
                }
            }
            "&" | "|" | "^" | "<<" | ">>" if both_int => {
                let inst = match op {
                    "&" => "and",
                    "|" => "or",
                    "^" => "xor",
                    "<<" => "shl",
                    _ => "ashr",
                };
                self.line(format!("{t} = {inst} i64 {}, {}", a.0, b.0));
                Ok((t, Ty::I64))
            }
            "**" => {
                if both_int {
                    let x = self.convert(a, Ty::F64)?;
                    let e32 = self.t();
                    self.line(format!("{e32} = trunc i64 {} to i32", b.0));
                    let p = self.t();
                    self.line(format!(
                        "{p} = call double @llvm.powi.f64.i32(double {x}, i32 {e32})"
                    ));
                    self.line(format!("{t} = fptosi double {p} to i64"));
                    return Ok((t, Ty::I64));
                }
                let x = self.convert(a, Ty::F64)?;
                let y = self.convert(b, Ty::F64)?;
                self.line(format!(
                    "{t} = call double @llvm.pow.f64(double {x}, double {y})"
                ));
                Ok((t, Ty::F64))
            }
            "+" | "-" | "*" | "/" | "%" => {
                let x = self.convert(a, Ty::F64)?;
                let y = self.convert(b, Ty::F64)?;
                let inst = match op {
                    "+" => "fadd",
                    "-" => "fsub",
                    "*" => "fmul",
                    "/" => "fdiv",
                    _ => "frem",
                };
                if op == "/" {
                    let nz = self.t();
                    self.line(format!("{nz} = fcmp une double {y}, 0.0"));
                    let name = self.name.clone();
                    self.check(&nz, format!("division by zero in {name}()"));
                }
                self.line(format!("{t} = {inst} double {x}, {y}"));
                if op == "%" {
                    // Euclidean: r < 0 ? r + |y| : r
                    let neg = self.t();
                    self.line(format!("{neg} = fcmp olt double {t}, 0.0"));
                    let ay = self.t();
                    self.decls.insert("declare double @llvm.fabs.f64(double)");
                    self.line(format!("{ay} = call double @llvm.fabs.f64(double {y})"));
                    let fixed = self.t();
                    self.line(format!("{fixed} = fadd double {t}, {ay}"));
                    let out = self.t();
                    self.line(format!(
                        "{out} = select i1 {neg}, double {fixed}, double {t}"
                    ));
                    return Ok((out, Ty::F64));
                }
                Ok((t, Ty::F64))
            }
            _ => Err(format!(
                "operator '{op}' not supported natively for these types"
            )),
        }
    }

    fn overflow_op(&mut self, which: &str, a: &str, b: &str, what: &str) -> G<String> {
        let pair = self.t();
        self.line(format!(
            "{pair} = call {{ i64, i1 }} @llvm.{which}.with.overflow.i64(i64 {a}, i64 {b})"
        ));
        let v = self.t();
        self.line(format!("{v} = extractvalue {{ i64, i1 }} {pair}, 0"));
        let o = self.t();
        self.line(format!("{o} = extractvalue {{ i64, i1 }} {pair}, 1"));
        let ok = self.t();
        self.line(format!("{ok} = xor i1 {o}, 1"));
        let name = self.name.clone();
        self.check(&ok, format!("integer overflow in {what} in {name}()"));
        Ok(v)
    }

    /// Arithmetic on quantities: dimensions are checked here, at compile time; the generated
    /// code is plain double arithmetic on SI values.
    fn qbinop(&mut self, op: &str, a: (String, Ty), b: (String, Ty)) -> G<(String, Ty)> {
        let (da, db) = (dim_of(a.1), dim_of(b.1));
        let zero = is_zero(&a.0) || is_zero(&b.0);
        let raw = |g: &mut Self, v: (String, Ty)| -> G<(String, Ty)> {
            match v.1 {
                Ty::Q(_) => Ok((v.0, Ty::F64)),
                Ty::I64 => Ok((g.convert(v, Ty::F64)?, Ty::F64)),
                Ty::F64 => Ok(v),
                Ty::Bool => Err("booleans cannot be combined with quantities".into()),
                _ => Err("this value cannot be combined with quantities".into()),
            }
        };
        let mismatch = || {
            format!(
                "unit mismatch: cannot apply '{op}' to {} and {}",
                units::describe(&da),
                units::describe(&db)
            )
        };
        if op == "**" {
            let n: i32 = if b.1 == Ty::I64 {
                b.0.parse()
                    .map_err(|_| "a quantity's exponent must be an integer literal".to_string())?
            } else {
                return Err("a quantity's exponent must be an integer literal".into());
            };
            let x = raw(self, a)?;
            let r = self.binop("**", x, (n.to_string(), Ty::I64))?;
            return Ok((r.0, qty(units::pow_dim(da, n))));
        }
        let x = raw(self, a)?;
        let y = raw(self, b)?;
        match op {
            "+" | "-" | "%" => {
                if da != db && !zero {
                    return Err(mismatch());
                }
                let r = self.binop(op, x, y)?;
                Ok((r.0, qty(if da == units::DIMLESS { db } else { da })))
            }
            "*" => Ok((self.binop(op, x, y)?.0, qty(units::mul_dim(da, db)))),
            "/" => Ok((self.binop(op, x, y)?.0, qty(units::div_dim(da, db)))),
            "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                if da != db && !zero {
                    return Err(mismatch());
                }
                self.binop(op, x, y)
            }
            _ => Err(format!("operator '{op}' is not defined for quantities")),
        }
    }

    fn builtin(&mut self, name: &str, args: Vec<(String, Ty)>) -> G<(String, Ty)> {
        if args.iter().any(|a| matches!(a.1, Ty::Q(_))) {
            return self.qbuiltin(name, args);
        }
        self.plain_builtin(name, args)
    }

    /// Math builtins on quantities, with dimensions checked at compile time.
    fn qbuiltin(&mut self, name: &str, args: Vec<(String, Ty)>) -> G<(String, Ty)> {
        let d0 = dim_of(args[0].1);
        let strip = |v: &(String, Ty)| {
            (
                v.0.clone(),
                if matches!(v.1, Ty::Q(_)) {
                    Ty::F64
                } else {
                    v.1
                },
            )
        };
        match (name, args.len()) {
            ("sqrt", 1) => {
                if d0.iter().any(|e| e % 2 != 0) {
                    return Err(format!("sqrt() of {} has no unit", units::describe(&d0)));
                }
                let mut half = d0;
                for e in half.iter_mut() {
                    *e /= 2;
                }
                let r = self.plain_builtin("sqrt", vec![strip(&args[0])])?;
                Ok((r.0, qty(half)))
            }
            ("sin" | "cos" | "tan", 1) => {
                if d0 != [0, 0, 0, 0, 0, 0, 0, 1] {
                    return Err(format!(
                        "{name}() expects an angle, got {}",
                        units::describe(&d0)
                    ));
                }
                self.plain_builtin(name, vec![strip(&args[0])])
            }
            ("abs", 1) => {
                let r = self.plain_builtin("abs", vec![strip(&args[0])])?;
                Ok((r.0, args[0].1))
            }
            ("min" | "max" | "clamp", _) => {
                for a in &args {
                    if dim_of(a.1) != d0 && !is_zero(&a.0) {
                        return Err(format!("{name}() arguments have different units"));
                    }
                }
                let plain: Vec<(String, Ty)> = args
                    .iter()
                    .map(|a| {
                        (
                            if is_zero(&a.0) {
                                "0.0".to_string()
                            } else {
                                a.0.clone()
                            },
                            Ty::F64,
                        )
                    })
                    .collect();
                let r = self.plain_builtin(name, plain)?;
                Ok((r.0, qty(d0)))
            }
            _ => Err(format!("{name}() does not accept unit quantities natively")),
        }
    }

    fn plain_builtin(&mut self, name: &str, args: Vec<(String, Ty)>) -> G<(String, Ty)> {
        let unary_f = |n: &str| -> Option<(&'static str, &'static str)> {
            Some(match n {
                "sqrt" => ("llvm.sqrt.f64", "declare double @llvm.sqrt.f64(double)"),
                "sin" => ("llvm.sin.f64", "declare double @llvm.sin.f64(double)"),
                "cos" => ("llvm.cos.f64", "declare double @llvm.cos.f64(double)"),
                "exp" => ("llvm.exp.f64", "declare double @llvm.exp.f64(double)"),
                "log" => ("llvm.log.f64", "declare double @llvm.log.f64(double)"),
                "log10" => ("llvm.log10.f64", "declare double @llvm.log10.f64(double)"),
                "tan" => ("tan", "declare double @tan(double)"),
                "asin" => ("asin", "declare double @asin(double)"),
                "acos" => ("acos", "declare double @acos(double)"),
                "atan" => ("atan", "declare double @atan(double)"),
                _ => return None,
            })
        };
        let t = self.t();
        if let Some((f, d)) = unary_f(name) {
            if args.len() != 1 {
                return Err(format!("{name}() takes 1 argument"));
            }
            self.decls.insert(d);
            let x = self.convert(args[0].clone(), Ty::F64)?;
            self.line(format!("{t} = call double @{f}(double {x})"));
            return Ok((t, Ty::F64));
        }
        match (name, args.len()) {
            ("atan2" | "hypot", 2) => {
                let d = if name == "atan2" {
                    "declare double @atan2(double, double)"
                } else {
                    "declare double @hypot(double, double)"
                };
                self.decls.insert(d);
                let x = self.convert(args[0].clone(), Ty::F64)?;
                let y = self.convert(args[1].clone(), Ty::F64)?;
                self.line(format!("{t} = call double @{name}(double {x}, double {y})"));
                Ok((t, Ty::F64))
            }
            ("pow", 2) => self.binop("**", args[0].clone(), args[1].clone()),
            ("abs", 1) => {
                let v = args[0].clone();
                if v.1 == Ty::I64 {
                    let n = self.overflow_op("ssub", "0", &v.0, "abs")?;
                    let neg = self.t();
                    self.line(format!("{neg} = icmp slt i64 {}, 0", v.0));
                    self.line(format!("{t} = select i1 {neg}, i64 {n}, i64 {}", v.0));
                    return Ok((t, Ty::I64));
                }
                self.decls.insert("declare double @llvm.fabs.f64(double)");
                let x = self.convert(v, Ty::F64)?;
                self.line(format!("{t} = call double @llvm.fabs.f64(double {x})"));
                Ok((t, Ty::F64))
            }
            ("round", 2) => {
                // round(x, digits): x rounded to `digits` decimals, as a float
                self.decls.insert("declare double @llvm.round.f64(double)");
                let x = self.convert(args[0].clone(), Ty::F64)?;
                let d = self.convert(args[1].clone(), Ty::I64)?;
                let d32 = self.t();
                self.line(format!("{d32} = trunc i64 {d} to i32"));
                let p = self.t();
                self.line(format!(
                    "{p} = call double @llvm.powi.f64.i32(double 10.0, i32 {d32})"
                ));
                let m = self.t();
                self.line(format!("{m} = fmul double {x}, {p}"));
                let r = self.t();
                self.line(format!("{r} = call double @llvm.round.f64(double {m})"));
                self.line(format!("{t} = fdiv double {r}, {p}"));
                Ok((t, Ty::F64))
            }
            ("floor" | "ceil" | "round", 1) => {
                let (f, d) = match name {
                    "floor" => ("llvm.floor.f64", "declare double @llvm.floor.f64(double)"),
                    "ceil" => ("llvm.ceil.f64", "declare double @llvm.ceil.f64(double)"),
                    _ => ("llvm.round.f64", "declare double @llvm.round.f64(double)"),
                };
                self.decls.insert(d);
                let x = self.convert(args[0].clone(), Ty::F64)?;
                let r = self.t();
                self.line(format!("{r} = call double @{f}(double {x})"));
                self.line(format!("{t} = fptosi double {r} to i64"));
                Ok((t, Ty::I64))
            }
            ("min" | "max", 2) => {
                let both_int = args[0].1 == Ty::I64 && args[1].1 == Ty::I64;
                let ty = if both_int { Ty::I64 } else { Ty::F64 };
                let x = self.convert(args[0].clone(), ty)?;
                let y = self.convert(args[1].clone(), ty)?;
                let c = self.t();
                let pred = match (both_int, name) {
                    (true, "min") => "icmp slt i64",
                    (true, _) => "icmp sgt i64",
                    (false, "min") => "fcmp olt double",
                    (false, _) => "fcmp ogt double",
                };
                self.line(format!("{c} = {pred} {x}, {y}"));
                self.line(format!(
                    "{t} = select i1 {c}, {} {x}, {} {y}",
                    ty.ll(),
                    ty.ll()
                ));
                Ok((t, ty))
            }
            ("clamp", 3) => {
                let lo = self.builtin("max", vec![args[0].clone(), args[1].clone()])?;
                self.builtin("min", vec![lo, args[2].clone()])
            }
            ("float", 1) => Ok((self.convert(args[0].clone(), Ty::F64)?, Ty::F64)),
            ("int", 1) => {
                if args[0].1 == Ty::F64 {
                    self.line(format!("{t} = fptosi double {} to i64", args[0].0));
                    Ok((t, Ty::I64))
                } else {
                    Ok((self.convert(args[0].clone(), Ty::I64)?, Ty::I64))
                }
            }
            ("radians" | "degrees", 1) => {
                let x = self.convert(args[0].clone(), Ty::F64)?;
                let k = if name == "radians" {
                    std::f64::consts::PI / 180.0
                } else {
                    180.0 / std::f64::consts::PI
                };
                self.line(format!("{t} = fmul double {x}, {}", flit(k)));
                Ok((t, Ty::F64))
            }
            _ => Err(format!("call to '{name}' is not supported natively")),
        }
    }
}

fn stmt_kind(s: &Stmt) -> &'static str {
    match s {
        Stmt::Fn(_) => "nested fn",
        Stmt::Try(..) => "try/catch",
        Stmt::Struct(_) => "struct",
        Stmt::Machine(_) => "machine",
        Stmt::When(..) | Stmt::Always(..) | Stmt::Deadline(..) => "reactive rule",
        Stmt::Import(..) => "import",
        Stmt::MacroDef(..) => "macro definition",
        _ => "this statement",
    }
}

fn expr_kind(e: &Expr) -> &'static str {
    match e {
        Expr::Str(_) | Expr::FStr(_) => "strings",
        Expr::List(_) | Expr::Repeat(..) => "lists",
        Expr::Dict(_) => "dicts",
        Expr::Lambda(_) => "closures",
        Expr::Match(..) => "match",
        Expr::MethodCall(..) => "method calls",
        Expr::Field(..) | Expr::Index(..) => "field/index access",
        Expr::Spawn(..) | Expr::Await(..) => "tasks",
        Expr::Nil => "nil",
        _ => "this expression",
    }
}

/// Exact double literal in LLVM's hex notation.
fn flit(f: f64) -> String {
    format!("0x{:016X}", f.to_bits())
}

fn is_zero(v: &str) -> bool {
    v == "0" || v == "0.0" || v == flit(0.0)
}
