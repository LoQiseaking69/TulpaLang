//! Tulpa 2.0 extensions: units of measure, keyword arguments, runtime type annotations,
//! structs with invariants, state machines, reactive signals with `when` rules and `always`
//! safety monitors, modules, and contract-driven property checking.

use crate::ast::*;
use crate::interp::*;
use crate::units::{self, Quantity, DIMLESS};
use crate::value::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

// ============================================================================ units

fn as_q(v: &Value) -> Option<Quantity> {
    match v {
        Value::Quantity(q) => Some(q.clone()),
        Value::Int(_) | Value::Float(_) => Some(Quantity::si(v.as_f64().unwrap(), DIMLESS)),
        _ => None,
    }
}

/// Collapses dimensionless results back to plain floats.
fn qv(q: Quantity) -> Value {
    if q.dim == DIMLESS {
        Value::Float(q.si)
    } else {
        Value::Quantity(q)
    }
}

fn mismatch(op: &str, a: &Quantity, b: &Quantity) -> String {
    format!(
        "unit mismatch: cannot apply '{op}' to {} and {}",
        units::describe(&a.dim),
        units::describe(&b.dim)
    )
}

pub fn quantity_binary(a: &Value, op: &str, b: &Value) -> Result<Value, String> {
    let (Some(x), Some(y)) = (as_q(a), as_q(b)) else {
        return match op {
            "==" => Ok(Value::Bool(false)),
            "!=" => Ok(Value::Bool(true)),
            "+" if matches!(a, Value::Str(_)) || matches!(b, Value::Str(_)) => {
                Ok(Value::str(format!("{a}{b}")))
            }
            _ => Err(format!(
                "unsupported operand types for {op}: {} and {}",
                a.type_name(),
                b.type_name()
            )),
        };
    };
    // Display unit: prefer the left operand's, else the right's.
    let unit = x.unit.clone().or_else(|| y.unit.clone());
    Ok(match op {
        "+" | "-" => {
            if x.dim != y.dim {
                // allow adding plain 0 (e.g. sum() start value)
                if y.dim == DIMLESS && y.si == 0.0 {
                    return Ok(Value::Quantity(x));
                }
                if x.dim == DIMLESS && x.si == 0.0 && op == "+" {
                    return Ok(Value::Quantity(y));
                }
                return Err(mismatch(op, &x, &y));
            }
            let si = if op == "+" { x.si + y.si } else { x.si - y.si };
            let u = if x.dim == DIMLESS {
                y.unit.clone()
            } else {
                x.unit.clone().or(y.unit.clone())
            };
            qv(Quantity {
                si,
                dim: x.dim,
                unit: u,
            })
        }
        "*" => {
            let dim = units::mul_dim(x.dim, y.dim);
            let keep = if y.dim == DIMLESS {
                x.unit.clone()
            } else if x.dim == DIMLESS {
                y.unit.clone()
            } else {
                None
            };
            qv(Quantity {
                si: x.si * y.si,
                dim,
                unit: keep,
            })
        }
        "/" => {
            if y.si == 0.0 {
                return Err("division by zero".into());
            }
            let dim = units::div_dim(x.dim, y.dim);
            let keep = if y.dim == DIMLESS {
                x.unit.clone()
            } else {
                None
            };
            qv(Quantity {
                si: x.si / y.si,
                dim,
                unit: keep,
            })
        }
        "%" => {
            if x.dim != y.dim {
                return Err(mismatch(op, &x, &y));
            }
            qv(Quantity {
                si: x.si.rem_euclid(y.si),
                dim: x.dim,
                unit,
            })
        }
        "**" => {
            if y.dim != DIMLESS || y.si.fract() != 0.0 {
                return Err("a quantity can only be raised to an integer power".into());
            }
            let n = y.si as i32;
            qv(Quantity::si(x.si.powi(n), units::pow_dim(x.dim, n)))
        }
        "==" => Value::Bool(a.equals(b)),
        "!=" => Value::Bool(!a.equals(b)),
        "<" | "<=" | ">" | ">=" => {
            let o = quantity_compare(a, b)?;
            Value::Bool(match op {
                "<" => o.is_lt(),
                "<=" => o.is_le(),
                ">" => o.is_gt(),
                _ => o.is_ge(),
            })
        }
        _ => return Err(format!("operator '{op}' is not defined for quantities")),
    })
}

pub fn quantity_compare(a: &Value, b: &Value) -> Result<std::cmp::Ordering, String> {
    let (Some(x), Some(y)) = (as_q(a), as_q(b)) else {
        return Err(format!(
            "cannot compare {} with {}",
            a.type_name(),
            b.type_name()
        ));
    };
    // Comparing against a bare 0 is always meaningful (sign checks).
    let zero = |q: &Quantity| q.dim == DIMLESS && q.si == 0.0;
    if x.dim != y.dim && !zero(&x) && !zero(&y) {
        return Err(format!(
            "unit mismatch: cannot compare {} with {} (write the bound with a unit, e.g. 2[m/s])",
            units::describe(&x.dim),
            units::describe(&y.dim)
        ));
    }
    x.si.partial_cmp(&y.si)
        .ok_or_else(|| "cannot compare NaN".into())
}

/// Milliseconds from a plain number (taken as ms) or a time quantity.
pub fn millis_of(v: &Value) -> Option<f64> {
    match v {
        Value::Quantity(q) if q.dim == [0, 0, 1, 0, 0, 0, 0, 0] => Some(q.si * 1000.0),
        Value::Quantity(_) => None,
        other => other.as_f64(),
    }
}

/// Seconds from a plain number (taken as s) or a time quantity.
pub fn seconds_of(v: &Value) -> Option<f64> {
    match v {
        Value::Quantity(q) if q.dim == [0, 0, 1, 0, 0, 0, 0, 0] => Some(q.si),
        Value::Quantity(_) => None,
        other => other.as_f64(),
    }
}

/// Radians from a plain number (taken as rad) or an angle quantity.
pub fn radians_of(v: &Value) -> Option<f64> {
    match v {
        Value::Quantity(q) if q.dim == [0, 0, 0, 0, 0, 0, 0, 1] => Some(q.si),
        Value::Quantity(_) => None,
        other => other.as_f64(),
    }
}

// ============================================================================ reactive rules

pub struct Rule {
    cond: Expr,
    body: Vec<Stmt>,
    els: Option<Vec<Stmt>>,
    env: Env,
    last: bool,
    /// `when cond for <hold>`: the condition must stay true this long (seconds) before firing.
    hold: Option<f64>,
    since: Option<f64>,
}

/// Bounded-response monitor: after `trigger` rises, `response` must hold within `within` seconds.
pub struct Deadline {
    trigger: Expr,
    within: f64,
    response: Contract,
    trigger_text: String,
    handler: Option<Vec<Stmt>>,
    env: Env,
    last_trigger: bool,
    armed_at: Option<f64>,
}

pub struct Monitor {
    c: Contract,
    handler: Option<Vec<Stmt>>,
    env: Env,
    tripped: bool,
}

const MAX_SETTLE: usize = 100;

/// Keyword arguments in call order.
pub type KwArgs = Vec<(String, Value)>;

fn guard_name(params: &[Param], i: usize) -> &str {
    params.get(i).map(|p| p.name.as_str()).unwrap_or("?")
}

impl Interpreter {
    // ------------------------------------------------------------------ calls & kwargs

    pub(crate) fn eval_args(&mut self, args: &[Expr], env: &Env) -> R<(Vec<Value>, KwArgs)> {
        let mut pos = Vec::with_capacity(args.len());
        let mut kw = Vec::new();
        for a in args {
            match a {
                Expr::Named(n, e) => {
                    let v = self.eval(e, env)?;
                    if kw.iter().any(|(k, _): &(String, Value)| k == n) {
                        return rt(format!("keyword argument '{n}' given twice"));
                    }
                    kw.push((n.clone(), v));
                }
                e => {
                    if !kw.is_empty() {
                        return rt("positional argument after keyword argument");
                    }
                    pos.push(self.eval(e, env)?);
                }
            }
        }
        Ok((pos, kw))
    }

    pub(crate) fn merge_kwargs(
        &mut self,
        fname: &str,
        params: &[Param],
        mut args: Vec<Value>,
        kw: Vec<(String, Value)>,
    ) -> R<Vec<Value>> {
        let placeholder = || Value::Thunk(Rc::new((MacroArg::Expr(Expr::Nil), new_env(None))));
        while args.len() < params.len() {
            args.push(placeholder());
        }
        let n_pos = args
            .iter()
            .filter(|a| !matches!(a, Value::Thunk(_)))
            .count();
        for (k, v) in kw {
            let Some(i) = params.iter().position(|p| p.name == k) else {
                let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                return rt(format!(
                    "{fname}() has no parameter '{k}' (parameters: {})",
                    names.join(", ")
                ));
            };
            if i < n_pos {
                return rt(format!(
                    "{fname}() got multiple values for '{}'",
                    guard_name(params, i)
                ));
            }
            args[i] = v;
        }
        Ok(args)
    }

    pub(crate) fn call_kw(
        &mut self,
        f: &Value,
        pos: Vec<Value>,
        kw: Vec<(String, Value)>,
    ) -> R<Value> {
        match f {
            Value::Func(c) => self.call_closure_kw(c, pos, kw),
            Value::Builtin(name, func) => {
                if !kw.is_empty() {
                    return rt(format!("builtin {name}() does not take keyword arguments"));
                }
                func(self, pos).map_err(Ctrl::from)
            }
            Value::StructType(t) => self.construct(t, pos, kw),
            Value::MachineType(t) => self.instantiate_machine(t, pos, kw),
            other => rt(format!("{} is not callable", other.type_name())),
        }
    }

    // ------------------------------------------------------------------ types

    pub(crate) fn type_matches(&self, ty: &TypeAnn, v: &Value) -> bool {
        let (ok, optional) = match ty {
            TypeAnn::Unit(u, opt) => (matches!(v, Value::Quantity(q) if q.dim == u.dim), *opt),
            TypeAnn::Name(n, opt) => {
                let ok = match n.as_str() {
                    "any" => true,
                    "int" => matches!(v, Value::Int(_)),
                    "float" => matches!(v, Value::Float(_)),
                    "num" | "number" => matches!(v, Value::Int(_) | Value::Float(_)),
                    "str" => matches!(v, Value::Str(_)),
                    "bool" => matches!(v, Value::Bool(_)),
                    "list" => matches!(v, Value::List(_)),
                    "dict" => matches!(v, Value::Dict(_)),
                    "fn" => matches!(
                        v,
                        Value::Func(_) | Value::Builtin(..) | Value::StructType(_)
                    ),
                    "nil" => matches!(v, Value::Nil),
                    "task" => matches!(v, Value::Task(_)),
                    "range" => matches!(v, Value::Range(..)),
                    "quantity" => matches!(v, Value::Quantity(_)),
                    "signal" => matches!(v, Value::Signal(_)),
                    other => match v {
                        Value::Object(o) => o.borrow().ty.def.name == other,
                        Value::Machine(m) => m.borrow().ty.def.name == other,
                        Value::Native(n) => n.borrow().type_name() == other,
                        _ => false,
                    },
                };
                (ok, *opt)
            }
        };
        ok || (optional && matches!(v, Value::Nil))
    }

    pub(crate) fn check_type(&self, ty: &TypeAnn, v: &Value, what: &str) -> R<()> {
        if self.type_matches(ty, v) {
            return Ok(());
        }
        let want = match ty {
            TypeAnn::Name(n, o) => format!("{n}{}", if *o { "?" } else { "" }),
            TypeAnn::Unit(u, o) => {
                let d = units::describe(&u.dim);
                let kind = d.split(" [").next().unwrap_or("").to_string();
                format!("[{}] ({kind}){}", u.name, if *o { "?" } else { "" })
            }
        };
        let got = match v {
            Value::Quantity(q) => format!("{} = {}", units::describe(&q.dim), v),
            other => format!("{} {}", other.type_name(), other.repr()),
        };
        let hint = match (ty, v) {
            (TypeAnn::Unit(u, _), Value::Int(_) | Value::Float(_)) => {
                format!(" (did you mean {v}[{}]?)", u.name)
            }
            _ => String::new(),
        };
        rt(format!(
            "type error: {what} expects {want}, got {got}{hint}"
        ))
    }

    /// String conversion honouring a struct's `to_str(self)` method.
    pub fn stringify(&mut self, v: &Value) -> R<String> {
        if let Value::Object(o) = v {
            let ty = o.borrow().ty.clone();
            if let Some(m) = ty.def.methods.iter().find(|m| m.name == "to_str") {
                let c = Rc::new(Closure {
                    def: m.clone(),
                    env: ty.env.clone(),
                });
                return Ok(self.call_closure(&c, vec![v.clone()])?.to_string());
            }
        }
        Ok(v.to_string())
    }

    /// Binary operator with struct operator overloading (`op_add`, `op_mul`, `op_eq`, `op_cmp`, ...).
    pub(crate) fn binary_op(&mut self, a: &Value, op: &str, b: &Value) -> R<Value> {
        let ((Value::Object(_), _) | (_, Value::Object(_))) = (a, b) else {
            return binary(a, op, b).map_err(Ctrl::from);
        };
        let name = match op {
            "+" => "op_add",
            "-" => "op_sub",
            "*" => "op_mul",
            "/" => "op_div",
            "%" => "op_mod",
            "==" | "!=" => "op_eq",
            "<" | "<=" | ">" | ">=" => "op_cmp",
            _ => "",
        };
        let find = |v: &Value, n: &str| -> Option<Rc<Closure>> {
            if let Value::Object(o) = v {
                let ty = o.borrow().ty.clone();
                return ty.def.methods.iter().find(|m| m.name == n).map(|m| {
                    Rc::new(Closure {
                        def: m.clone(),
                        env: ty.env.clone(),
                    })
                });
            }
            None
        };
        if let Some(c) = find(a, name) {
            let r = self.call_closure(&c, vec![a.clone(), b.clone()])?;
            return Ok(match op {
                "!=" => Value::Bool(!r.truthy()),
                "<" | "<=" | ">" | ">=" => {
                    let n = r.as_f64().ok_or("op_cmp must return a number")?;
                    Value::Bool(match op {
                        "<" => n < 0.0,
                        "<=" => n <= 0.0,
                        ">" => n > 0.0,
                        _ => n >= 0.0,
                    })
                }
                _ => r,
            });
        }
        // Reflected forms for `2 * v` / `1 + v`.
        let rname = match op {
            "+" => "op_radd",
            "*" => "op_rmul",
            _ => "",
        };
        if let Some(c) = find(b, rname).or_else(|| {
            if matches!(op, "+" | "*") {
                find(b, name)
            } else {
                None
            }
        }) {
            return self.call_closure(&c, vec![b.clone(), a.clone()]);
        }
        binary(a, op, b).map_err(Ctrl::from)
    }

    // ------------------------------------------------------------------ fields

    pub(crate) fn get_field(&mut self, o: &Value, f: &str) -> R<Value> {
        match o {
            Value::Object(obj) => {
                if let Some(v) = obj.borrow().get(f) {
                    return Ok(v);
                }
                let name = obj.borrow().ty.def.name.clone();
                rt(format!("{name} has no field '{f}'"))
            }
            Value::Machine(m) => {
                let m = m.borrow();
                match f {
                    "state" => Ok(Value::str(m.ty.def.states[m.state].name.as_str())),
                    "history" => Ok(Value::list(
                        m.history.iter().map(|s| Value::str(s.as_str())).collect(),
                    )),
                    _ => lookup_own(&m.env, f).ok_or_else(|| {
                        Ctrl::from(format!("machine {} has no variable '{f}'", m.ty.def.name))
                    }),
                }
            }
            Value::Signal(s) => match f {
                "value" => self.signal_get(s),
                "prev" => Ok(s.borrow().prev.clone()),
                _ => rt(format!("signal has no field '{f}'")),
            },
            Value::Quantity(q) => match f {
                "value" => Ok(Value::Float(q.display_value())),
                "unit" => Ok(Value::str(q.unit_name())),
                "si" => Ok(Value::Float(q.si)),
                _ => rt(format!("quantity has no field '{f}'")),
            },
            other => field(other, f).map_err(Ctrl::from),
        }
    }

    pub(crate) fn set_field(&mut self, o: &Value, f: &str, v: Value) -> R<()> {
        match o {
            Value::Object(obj) => {
                let ty = obj.borrow().ty.clone();
                if let Some(p) = ty.def.fields.iter().find(|p| p.name == f) {
                    if let Some(t) = &p.ty {
                        self.check_type(t, &v, &format!("{}.{f}", ty.def.name))?;
                    }
                }
                let old = obj.borrow().get(f);
                if !obj.borrow_mut().set(f, v) {
                    return rt(format!("{} has no field '{f}'", ty.def.name));
                }
                if let Err(e) = self.check_struct_invariants(o) {
                    // roll back so the object never stays in a state that violates its invariant
                    obj.borrow_mut().set(f, old.unwrap_or(Value::Nil));
                    return Err(e);
                }
                Ok(())
            }
            Value::Machine(m) => {
                let env = m.borrow().env.clone();
                if lookup_own(&env, f).is_none() {
                    return rt(format!("machine has no variable '{f}'"));
                }
                define(&env, f, v);
                Ok(())
            }
            Value::Signal(s) => {
                if f != "value" {
                    return rt(format!("signal has no settable field '{f}'"));
                }
                self.signal_set(s, v)
            }
            other => set_index(other, &Value::str(f), v).map_err(Ctrl::from),
        }
    }

    // ------------------------------------------------------------------ methods

    pub(crate) fn call_method_kw(
        &mut self,
        o: &Value,
        name: &str,
        args: Vec<Value>,
        kw: Vec<(String, Value)>,
    ) -> R<Value> {
        match o {
            Value::Object(obj) => {
                let ty = obj.borrow().ty.clone();
                if let Some(m) = ty.def.methods.iter().find(|m| m.name == name).cloned() {
                    let c = Rc::new(Closure {
                        def: m.clone(),
                        env: ty.env.clone(),
                    });
                    let mut a = args;
                    if m.params.first().map(|p| p.name == "self").unwrap_or(false) {
                        a.insert(0, o.clone());
                    }
                    let r = self.call_closure_kw(&c, a, kw)?;
                    self.check_struct_invariants(o)
                        .map_err(|e| prefix(e, &format!("after {}.{name}(): ", ty.def.name)))?;
                    return Ok(r);
                }
                // a field holding a function
                if let Some(f @ (Value::Func(_) | Value::Builtin(..))) = obj.borrow().get(name) {
                    return self.call_kw(&f, args, kw);
                }
                rt(format!("{} has no method '{name}'", ty.def.name))
            }
            Value::StructType(t) => {
                if let Some(m) = t.def.methods.iter().find(|m| m.name == name) {
                    let c = Rc::new(Closure {
                        def: m.clone(),
                        env: t.env.clone(),
                    });
                    return self.call_closure_kw(&c, args, kw);
                }
                match name {
                    "fields" => Ok(Value::list(
                        t.def
                            .fields
                            .iter()
                            .map(|p| Value::str(p.name.as_str()))
                            .collect(),
                    )),
                    "name" => Ok(Value::str(t.def.name.as_str())),
                    _ => rt(format!(
                        "struct {} has no static method '{name}'",
                        t.def.name
                    )),
                }
            }
            Value::Machine(m) => self.machine_method(m, name, args),
            Value::MachineType(t) => match name {
                "mermaid" | "diagram" => Ok(Value::str(mermaid(&t.def))),
                "states" => Ok(Value::list(
                    t.def
                        .states
                        .iter()
                        .map(|s| Value::str(s.name.as_str()))
                        .collect(),
                )),
                _ => rt(format!("machine {} has no method '{name}'", t.def.name)),
            },
            Value::Signal(s) => self.signal_method(s, name, args),
            Value::Quantity(q) => quantity_method(q, name, &args).map_err(Ctrl::from),
            other => {
                if !kw.is_empty() {
                    return rt(format!(
                        "{}.{name}() does not take keyword arguments",
                        other.type_name()
                    ));
                }
                self.call_method(other, name, args)
            }
        }
    }

    // ------------------------------------------------------------------ structs

    fn construct(
        &mut self,
        t: &Rc<StructType>,
        pos: Vec<Value>,
        kw: Vec<(String, Value)>,
    ) -> R<Value> {
        let def = &t.def;
        if pos.len() > def.fields.len() {
            return rt(format!(
                "{}() takes at most {} field values, got {}",
                def.name,
                def.fields.len(),
                pos.len()
            ));
        }
        for (k, _) in &kw {
            if !def.fields.iter().any(|p| &p.name == k) {
                let names: Vec<&str> = def.fields.iter().map(|p| p.name.as_str()).collect();
                return rt(format!(
                    "{} has no field '{k}' (fields: {})",
                    def.name,
                    names.join(", ")
                ));
            }
        }
        let scope = new_env(Some(t.env.clone()));
        let mut fields = Vec::new();
        let mut pos = pos.into_iter();
        for p in &def.fields {
            let positional = pos.next();
            let named = kw
                .iter()
                .find(|(k, _)| k == &p.name)
                .map(|(_, v)| v.clone());
            if positional.is_some() && named.is_some() {
                return rt(format!("{}(): field '{}' given twice", def.name, p.name));
            }
            let v = match positional.or(named) {
                Some(v) => v,
                None => match &p.default {
                    Some(d) => self.eval(d, &scope)?,
                    None => return rt(format!("{}(): missing field '{}'", def.name, p.name)),
                },
            };
            if let Some(ty) = &p.ty {
                self.check_type(ty, &v, &format!("{}.{}", def.name, p.name))?;
            }
            define(&scope, &p.name, v.clone());
            fields.push((p.name.clone(), v));
        }
        let obj = Value::Object(Rc::new(RefCell::new(Object {
            ty: t.clone(),
            fields,
        })));
        self.check_struct_invariants(&obj)
            .map_err(|e| prefix(e, &format!("constructing {}: ", def.name)))?;
        if let Some(init) = def.methods.iter().find(|m| m.name == "init") {
            let c = Rc::new(Closure {
                def: init.clone(),
                env: t.env.clone(),
            });
            self.call_closure(&c, vec![obj.clone()])?;
            self.check_struct_invariants(&obj)?;
        }
        Ok(obj)
    }

    fn check_struct_invariants(&mut self, o: &Value) -> R<()> {
        let Value::Object(obj) = o else { return Ok(()) };
        let ty = obj.borrow().ty.clone();
        if ty.def.invariants.is_empty() {
            return Ok(());
        }
        let scope = new_env(Some(ty.env.clone()));
        define(&scope, "self", o.clone());
        for inv in &ty.def.invariants {
            if !self.eval(&inv.expr, &scope)?.truthy() {
                return rt(format!(
                    "struct invariant violated for {}: {} (now {})",
                    ty.def.name, inv.text, o
                ));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ state machines

    fn instantiate_machine(
        &mut self,
        t: &Rc<MachineType>,
        pos: Vec<Value>,
        kw: Vec<(String, Value)>,
    ) -> R<Value> {
        let def = &t.def;
        let args = if kw.is_empty() {
            pos
        } else {
            self.merge_kwargs(&def.name, &def.params, pos, kw)?
        };
        if args.len() > def.params.len() {
            return rt(format!(
                "{}() takes {} argument(s), got {}",
                def.name,
                def.params.len(),
                args.len()
            ));
        }
        let env = new_env(Some(t.env.clone()));
        let mut it = args.into_iter();
        for p in &def.params {
            let v = match it.next() {
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
        let first = def.states[0].name.clone();
        let inst = Rc::new(RefCell::new(MachineInst {
            ty: t.clone(),
            env: env.clone(),
            state: 0,
            history: vec![first],
        }));
        define(&env, "self", Value::Machine(inst.clone()));
        if let Some(enter) = &def.states[0].enter {
            self.exec_block(enter, &env)?;
        }
        Ok(Value::Machine(inst))
    }

    /// Finds the first enabled transition for `event` (state transitions first, then global ones).
    fn pick_transition(
        &mut self,
        m: &Rc<RefCell<MachineInst>>,
        event: &str,
        args: &[Value],
    ) -> R<Option<(Transition, Env)>> {
        let (def, env, state) = {
            let b = m.borrow();
            (b.ty.def.clone(), b.env.clone(), b.state)
        };
        let candidates = def.states[state]
            .transitions
            .iter()
            .chain(def.global.iter());
        for t in candidates {
            if t.event != event {
                continue;
            }
            if t.params.len() != args.len() {
                return rt(format!(
                    "event '{event}' expects {} argument(s), got {}",
                    t.params.len(),
                    args.len()
                ));
            }
            let scope = new_env(Some(env.clone()));
            for (p, v) in t.params.iter().zip(args) {
                define(&scope, p, v.clone());
            }
            let ok = match &t.guard {
                Some(g) => self.eval(g, &scope)?.truthy(),
                None => true,
            };
            if ok {
                return Ok(Some((t.clone(), scope)));
            }
        }
        Ok(None)
    }

    fn machine_send(
        &mut self,
        m: &Rc<RefCell<MachineInst>>,
        event: &str,
        args: &[Value],
    ) -> R<bool> {
        let Some((t, scope)) = self.pick_transition(m, event, args)? else {
            return Ok(false);
        };
        let (def, env, from) = {
            let b = m.borrow();
            (b.ty.def.clone(), b.env.clone(), b.state)
        };
        match &t.target {
            None => {
                if let Some(a) = &t.action {
                    self.exec_block_in(a, &scope)?;
                }
            }
            Some(target) => {
                let to = def.states.iter().position(|s| &s.name == target).unwrap();
                if let Some(exit) = &def.states[from].exit {
                    self.exec_block(exit, &env)?;
                }
                if let Some(a) = &t.action {
                    self.exec_block_in(a, &scope)?;
                }
                {
                    let mut b = m.borrow_mut();
                    b.state = to;
                    b.history.push(target.clone());
                    if b.history.len() > 1000 {
                        b.history.remove(0);
                    }
                }
                if let Some(enter) = &def.states[to].enter {
                    self.exec_block(enter, &env)?;
                }
            }
        }
        if self.has_reactive() {
            self.react()?;
        }
        Ok(true)
    }

    fn machine_method(
        &mut self,
        m: &Rc<RefCell<MachineInst>>,
        name: &str,
        args: Vec<Value>,
    ) -> R<Value> {
        // A user-defined event always wins over a convenience method of the same name.
        let is_event = {
            let b = m.borrow();
            b.ty.def
                .states
                .iter()
                .flat_map(|s| s.transitions.iter())
                .chain(b.ty.def.global.iter())
                .any(|t| t.event == name)
        };
        if is_event && !matches!(name, "send" | "fire" | "can" | "is" | "in_state" | "events") {
            return Ok(Value::Bool(self.machine_send(m, name, &args)?));
        }
        match name {
            "send" | "fire" => {
                let Some(ev) = args.first() else {
                    return rt("send() needs an event name");
                };
                let ev = ev.to_string();
                Ok(Value::Bool(self.machine_send(m, &ev, &args[1..])?))
            }
            "can" => {
                let Some(ev) = args.first() else {
                    return rt("can() needs an event name");
                };
                let ev = ev.to_string();
                Ok(Value::Bool(
                    self.pick_transition(m, &ev, &args[1..])?.is_some(),
                ))
            }
            "is" | "in_state" => {
                let want = args.first().map(|v| v.to_string()).unwrap_or_default();
                let b = m.borrow();
                Ok(Value::Bool(b.ty.def.states[b.state].name == want))
            }
            "events" => {
                let b = m.borrow();
                let mut evs: Vec<String> = b.ty.def.states[b.state]
                    .transitions
                    .iter()
                    .chain(b.ty.def.global.iter())
                    .map(|t| t.event.clone())
                    .collect();
                evs.dedup();
                Ok(Value::list(evs.into_iter().map(Value::str).collect()))
            }
            "restart" | "reset" => {
                let (def, env) = {
                    let mut b = m.borrow_mut();
                    b.state = 0;
                    let first = b.ty.def.states[0].name.clone();
                    b.history = vec![first];
                    (b.ty.def.clone(), b.env.clone())
                };
                if let Some(enter) = &def.states[0].enter {
                    self.exec_block(enter, &env)?;
                }
                Ok(Value::Nil)
            }
            "mermaid" | "diagram" => Ok(Value::str(mermaid(&m.borrow().ty.def))),
            // `door.open()` is shorthand for `door.send("open")`
            ev => {
                let known = {
                    let b = m.borrow();
                    b.ty.def
                        .states
                        .iter()
                        .flat_map(|s| s.transitions.iter())
                        .chain(b.ty.def.global.iter())
                        .any(|t| t.event == ev)
                };
                if known {
                    Ok(Value::Bool(self.machine_send(m, ev, &args)?))
                } else {
                    rt(format!(
                        "machine {} has no event or method '{ev}'",
                        m.borrow().ty.def.name
                    ))
                }
            }
        }
    }

    // ------------------------------------------------------------------ signals

    pub(crate) fn signal_get(&mut self, s: &Rc<RefCell<Signal>>) -> R<Value> {
        let d = s.borrow().derive.clone();
        match d {
            Some(f) => self.call(&f, vec![]),
            None => Ok(s.borrow().value.clone()),
        }
    }

    pub(crate) fn signal_set(&mut self, s: &Rc<RefCell<Signal>>, v: Value) -> R<()> {
        if s.borrow().derive.is_some() {
            return rt("cannot set a derived signal");
        }
        let now = self.now();
        let (old, subs) = {
            let mut b = s.borrow_mut();
            let old = std::mem::replace(&mut b.value, v.clone());
            b.prev = old.clone();
            b.prev_at = b.changed_at;
            b.changed_at = now;
            b.history.push_back(v.clone());
            while b.history.len() > b.capacity {
                b.history.pop_front();
            }
            (old, b.subscribers.clone())
        };
        for f in subs {
            self.call(&f, vec![v.clone(), old.clone()])?;
        }
        self.react()
    }

    fn signal_method(&mut self, s: &Rc<RefCell<Signal>>, name: &str, args: Vec<Value>) -> R<Value> {
        let hist = || s.borrow().history.iter().cloned().collect::<Vec<_>>();
        match name {
            "get" => self.signal_get(s),
            "set" => {
                let v = args.into_iter().next().ok_or("set() needs a value")?;
                self.signal_set(s, v)?;
                Ok(Value::Nil)
            }
            "update" => {
                let f = args.into_iter().next().ok_or("update() needs a function")?;
                let cur = self.signal_get(s)?;
                let v = self.call(&f, vec![cur])?;
                self.signal_set(s, v.clone())?;
                Ok(v)
            }
            "prev" => Ok(s.borrow().prev.clone()),
            "history" => Ok(Value::list(hist())),
            "subscribe" | "on_change" => {
                let f = args
                    .into_iter()
                    .next()
                    .ok_or("subscribe() needs a function")?;
                s.borrow_mut().subscribers.push(f);
                Ok(Value::Nil)
            }
            "rate" => {
                // derivative per second between the last two samples
                let (v, p, t1, t0) = {
                    let b = s.borrow();
                    (b.value.clone(), b.prev.clone(), b.changed_at, b.prev_at)
                };
                let dt = t1 - t0;
                if matches!(p, Value::Nil) || dt <= 0.0 {
                    return Ok(Value::Float(0.0));
                }
                let dv = binary(&v, "-", &p)?;
                let secs = Value::Quantity(Quantity::si(dt, [0, 0, 1, 0, 0, 0, 0, 0]));
                Ok(binary(&dv, "/", &secs)?)
            }
            "avg" | "mean" => {
                let h = hist();
                let n = args
                    .first()
                    .and_then(|v| v.as_i64())
                    .map(|n| n.max(1) as usize)
                    .unwrap_or(h.len());
                let tail: Vec<Value> = h.iter().rev().take(n).cloned().collect();
                if tail.is_empty() {
                    return Ok(Value::Nil);
                }
                let mut acc = Value::Int(0);
                for v in &tail {
                    acc = binary(&acc, "+", v)?;
                }
                Ok(binary(&acc, "/", &Value::Float(tail.len() as f64))?)
            }
            "min" | "max" => {
                let h = hist();
                let mut best: Option<Value> = None;
                for v in h {
                    best = Some(match best {
                        None => v,
                        Some(b) => {
                            let o = compare(&v, &b)?;
                            if (name == "max" && o.is_gt()) || (name == "min" && o.is_lt()) {
                                v
                            } else {
                                b
                            }
                        }
                    });
                }
                Ok(best.unwrap_or(Value::Nil))
            }
            _ => rt(format!("signal has no method '{name}'")),
        }
    }

    // ------------------------------------------------------------------ when / always

    pub(crate) fn register_when(
        &mut self,
        cond: &Expr,
        hold: Option<&Expr>,
        body: &[Stmt],
        els: Option<&Vec<Stmt>>,
        env: &Env,
    ) -> R<Value> {
        let hold = match hold {
            Some(e) => {
                let v = self.eval(e, env)?;
                Some(seconds_of(&v).ok_or("`when ... for` needs a duration, e.g. 50[ms] or 0.05")?)
            }
            None => None,
        };
        self.rules.push(Rc::new(RefCell::new(Rule {
            cond: cond.clone(),
            body: body.to_vec(),
            els: els.cloned(),
            env: env.clone(),
            last: false,
            hold,
            since: None,
        })));
        self.react()?;
        Ok(Value::Nil)
    }

    pub(crate) fn register_always(
        &mut self,
        c: &Contract,
        handler: Option<&Vec<Stmt>>,
        env: &Env,
    ) -> R<Value> {
        self.monitors.push(Rc::new(RefCell::new(Monitor {
            c: c.clone(),
            handler: handler.cloned(),
            env: env.clone(),
            tripped: false,
        })));
        self.react()?;
        Ok(Value::Nil)
    }

    pub(crate) fn register_deadline(
        &mut self,
        trigger: &Expr,
        within: &Expr,
        response: &Contract,
        handler: Option<&Vec<Stmt>>,
        env: &Env,
    ) -> R<Value> {
        let v = self.eval(within, env)?;
        let secs = seconds_of(&v).ok_or("`within` needs a duration, e.g. 200[ms] or 0.2")?;
        self.deadlines.push(Rc::new(RefCell::new(Deadline {
            trigger: trigger.clone(),
            within: secs,
            response: response.clone(),
            trigger_text: format!("{v}"),
            handler: handler.cloned(),
            env: env.clone(),
            last_trigger: false,
            armed_at: None,
        })));
        self.react()?;
        Ok(Value::Nil)
    }

    /// True when any reactive rule or monitor is registered.
    pub fn has_reactive(&self) -> bool {
        !self.rules.is_empty() || !self.monitors.is_empty() || !self.deadlines.is_empty()
    }

    fn has_timed(&self) -> bool {
        !self.deadlines.is_empty() || self.rules.iter().any(|r| r.borrow().hold.is_some())
    }

    /// Current time in seconds since start (virtual when the simulated clock is on).
    pub fn now(&self) -> f64 {
        match self.virtual_time {
            Some(t) => t,
            None => self.start.elapsed().as_secs_f64(),
        }
    }

    /// Sleeps (or advances the virtual clock), re-checking time-based rules as time passes.
    pub fn sleep_secs(&mut self, secs: f64) -> R<()> {
        let secs = secs.max(0.0);
        let timed = self.has_timed();
        if let Some(t) = self.virtual_time {
            if !timed {
                self.virtual_time = Some(t + secs);
                return self.react();
            }
            // 1 ms steps computed from the start time, so no rounding error accumulates
            let end = t + secs;
            let steps = (secs * 1000.0 - 1e-9).ceil().max(0.0) as u64;
            for k in 1..=steps {
                self.virtual_time = Some((t + k as f64 / 1000.0).min(end));
                self.react()?;
            }
            self.virtual_time = Some(end);
            return Ok(());
        }
        if !timed {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            return Ok(());
        }
        let end = self.now() + secs;
        loop {
            let left = end - self.now();
            if left <= 0.0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs_f64(left.min(0.002)));
            self.react()?;
        }
        Ok(())
    }

    /// Re-evaluates `when` rules (edge-triggered) and `always` monitors until they settle.
    pub fn react(&mut self) -> R<()> {
        if self.reacting || !self.has_reactive() {
            return Ok(());
        }
        self.reacting = true;
        let r = self.react_inner();
        self.reacting = false;
        r
    }

    fn react_inner(&mut self) -> R<()> {
        for _ in 0..MAX_SETTLE {
            let mut fired = false;
            let rules = self.rules.clone();
            for rule in rules {
                let (cond, env, last) = {
                    let r = rule.borrow();
                    (r.cond.clone(), r.env.clone(), r.last)
                };
                let raw = self.eval(&cond, &env)?.truthy();
                let hold = rule.borrow().hold;
                let now = match (hold, raw) {
                    (Some(h), true) => {
                        let t = self.now();
                        let since = *rule.borrow_mut().since.get_or_insert(t);
                        t - since >= h - 1e-9
                    }
                    (Some(_), false) => {
                        rule.borrow_mut().since = None;
                        false
                    }
                    (None, v) => v,
                };
                if now != last {
                    rule.borrow_mut().last = now;
                    let block = {
                        let r = rule.borrow();
                        if now {
                            Some(r.body.clone())
                        } else {
                            r.els.clone()
                        }
                    };
                    if let Some(b) = block {
                        self.exec_block(&b, &env)?;
                        fired = true;
                    }
                }
            }
            let monitors = self.monitors.clone();
            for m in monitors {
                let (c, env, tripped) = {
                    let b = m.borrow();
                    (b.c.clone(), b.env.clone(), b.tripped)
                };
                let ok = self.eval(&c.expr, &env)?.truthy();
                if ok {
                    m.borrow_mut().tripped = false;
                    continue;
                }
                if tripped {
                    continue;
                }
                m.borrow_mut().tripped = true;
                let handler = m.borrow().handler.clone();
                match handler {
                    Some(h) => {
                        self.exec_block(&h, &env)?;
                        fired = true;
                    }
                    None => return rt(format!("safety envelope violated: always {}", c.text)),
                }
            }
            let deadlines = self.deadlines.clone();
            for d in deadlines {
                let (trig, env, last) = {
                    let b = d.borrow();
                    (b.trigger.clone(), b.env.clone(), b.last_trigger)
                };
                let t_now = self.eval(&trig, &env)?.truthy();
                let now = self.now();
                if t_now && !last {
                    d.borrow_mut().armed_at = Some(now);
                }
                d.borrow_mut().last_trigger = t_now;
                let armed = d.borrow().armed_at;
                let Some(armed) = armed else { continue };
                let resp = d.borrow().response.clone();
                if self.eval(&resp.expr, &env)?.truthy() {
                    d.borrow_mut().armed_at = None; // satisfied in time
                    continue;
                }
                let within = d.borrow().within;
                if now - armed > within + 1e-9 {
                    d.borrow_mut().armed_at = None;
                    let (handler, text) = {
                        let b = d.borrow();
                        (b.handler.clone(), b.trigger_text.clone())
                    };
                    match handler {
                        Some(h) => {
                            self.exec_block(&h, &env)?;
                            fired = true;
                        }
                        None => {
                            return rt(format!(
                                "deadline missed: response '{}' did not hold within {text} of the trigger",
                                resp.text
                            ))
                        }
                    }
                }
            }
            if !fired {
                return Ok(());
            }
        }
        rt(format!("reactive rules did not settle after {MAX_SETTLE} rounds (a when-rule keeps re-triggering itself)"))
    }

    // ------------------------------------------------------------------ modules

    pub(crate) fn import(
        &mut self,
        path: &str,
        alias: Option<&str>,
        names: &[String],
        env: &Env,
    ) -> R<Value> {
        let base = self.base_dirs.last().cloned().unwrap_or_else(|| ".".into());
        let mut full = base.join(path);
        if !full.exists() && full.extension().is_none() {
            full.set_extension("rav");
        }
        let canon = std::fs::canonicalize(&full)
            .map_err(|e| format!("import \"{path}\": {} ({e})", full.display()))?;
        let module = match self.modules.get(&canon) {
            Some(m) => m.clone(),
            None => {
                if self.loading.contains(&canon) {
                    return rt(format!("circular import of \"{path}\""));
                }
                let src = std::fs::read_to_string(&canon)
                    .map_err(|e| format!("import \"{path}\": {e}"))?;
                let prog = crate::parser::parse(&src).map_err(|e| {
                    format!(
                        "syntax error in {}:{}:{}: {}",
                        canon.display(),
                        e.line,
                        e.col,
                        e.msg
                    )
                })?;
                let menv = new_env(Some(self.builtins_env.clone()));
                self.loading.push(canon.clone());
                self.base_dirs
                    .push(canon.parent().map(|p| p.to_path_buf()).unwrap_or_default());
                self.hoist(&prog, &menv);
                let r = self.exec_block_in(&prog, &menv);
                self.base_dirs.pop();
                self.loading.pop();
                match r {
                    Ok(_) | Err(Ctrl::Return(_)) => {}
                    Err(Ctrl::Error(mut e)) => {
                        e.msg = format!("in module \"{path}\": {}", e.msg);
                        return Err(Ctrl::Error(e));
                    }
                    Err(_) => {
                        return rt(format!("break/continue at top level of module \"{path}\""))
                    }
                }
                let mut d = Dict::default();
                for (k, v) in env_vars(&menv) {
                    if !k.starts_with('_') {
                        d.set(Value::str(k.as_str()), v)?;
                    }
                }
                let m = Value::dict(d);
                self.modules.insert(canon.clone(), m.clone());
                m
            }
        };
        if names.is_empty() {
            let name = match alias {
                Some(a) => a.to_string(),
                None => canon
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "module".into()),
            };
            define(env, &name, module);
        } else {
            let Value::Dict(d) = &module else {
                unreachable!()
            };
            for n in names {
                let v = d
                    .borrow()
                    .get(&Value::str(n.as_str()))?
                    .ok_or_else(|| format!("module \"{path}\" has no export '{n}'"))?;
                define(env, n, v);
            }
        }
        Ok(Value::Nil)
    }

    // ------------------------------------------------------------------ contract fuzzing

    /// Property-checks a function against its own contract: generates arguments that satisfy
    /// every `require`, runs the function, and reports (and shrinks) any input that makes it fail
    /// an `ensure`, a type annotation, or crash.
    pub fn check_fn(&mut self, f: &Value, trials: usize) -> R<Value> {
        let Value::Func(c) = f else {
            return rt("check() needs a user-defined function");
        };
        let c = c.clone();
        let mut rng = Gen::new(0x5eed_7a1a ^ c.def.name.len() as u64);
        let (mut passed, mut discarded) = (0usize, 0usize);
        let mut failure: Option<(Vec<Value>, String)> = None;
        'trials: for _ in 0..trials {
            let mut args = None;
            for _ in 0..100 {
                let cand: Vec<Value> = c
                    .def
                    .params
                    .iter()
                    .map(|p| rng.value(p.ty.as_ref()))
                    .collect();
                if self.requires_hold(&c, &cand)? {
                    args = Some(cand);
                    break;
                }
            }
            let Some(args) = args else {
                discarded += 1;
                continue;
            };
            match self.run_quiet(&c, args.clone()) {
                Ok(()) => passed += 1,
                Err(msg) => {
                    failure = Some((args, msg));
                    break 'trials;
                }
            }
        }
        let mut d = Dict::default();
        d.set(Value::str("name"), Value::str(c.def.name.as_str()))?;
        d.set(Value::str("passed"), Value::Int(passed as i64))?;
        d.set(Value::str("discarded"), Value::Int(discarded as i64))?;
        match failure {
            None => {
                d.set(Value::str("ok"), Value::Bool(true))?;
                d.set(Value::str("counterexample"), Value::Nil)?;
                d.set(Value::str("error"), Value::Nil)?;
            }
            Some((args, msg)) => {
                let (args, msg) = self.shrink(&c, args, msg)?;
                d.set(Value::str("ok"), Value::Bool(false))?;
                d.set(Value::str("counterexample"), Value::list(args))?;
                d.set(Value::str("error"), Value::str(msg))?;
            }
        }
        Ok(Value::dict(d))
    }

    fn requires_hold(&mut self, c: &Rc<Closure>, args: &[Value]) -> R<bool> {
        let env = new_env(Some(c.env.clone()));
        for (p, v) in c.def.params.iter().zip(args) {
            if let Some(ty) = &p.ty {
                if !self.type_matches(ty, v) {
                    return Ok(false);
                }
            }
            define(&env, &p.name, v.clone());
        }
        for r in &c.def.requires {
            match self.eval(&r.expr, &env) {
                Ok(v) if v.truthy() => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Runs a call with output suppressed; returns the error message on failure.
    fn run_quiet(&mut self, c: &Rc<Closure>, args: Vec<Value>) -> Result<(), String> {
        let saved = std::mem::replace(
            &mut self.output,
            Output::Buffer(Rc::new(RefCell::new(String::new()))),
        );
        let r = self.call_closure(c, args);
        self.output = saved;
        match r {
            Ok(_) | Err(Ctrl::Return(_)) => Ok(()),
            Err(Ctrl::Error(e)) => Err(e.msg),
            Err(_) => Err("break/continue escaped the function".into()),
        }
    }

    fn shrink(
        &mut self,
        c: &Rc<Closure>,
        mut args: Vec<Value>,
        mut msg: String,
    ) -> R<(Vec<Value>, String)> {
        for _ in 0..200 {
            let mut improved = false;
            for i in 0..args.len() {
                for cand in shrink_candidates(&args[i]) {
                    let mut trial = args.clone();
                    trial[i] = cand;
                    if !self.requires_hold(c, &trial)? {
                        continue;
                    }
                    if let Err(m) = self.run_quiet(c, trial.clone()) {
                        args = trial;
                        msg = m;
                        improved = true;
                        break;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        Ok((args, msg))
    }
}

fn prefix(c: Ctrl, p: &str) -> Ctrl {
    match c {
        Ctrl::Error(mut e) => {
            e.msg = format!("{p}{}", e.msg);
            Ctrl::Error(e)
        }
        other => other,
    }
}

fn lookup_own(env: &Env, name: &str) -> Option<Value> {
    env_vars(env)
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
}

fn quantity_method(q: &Quantity, name: &str, a: &[Value]) -> Result<Value, String> {
    let unit_arg = || {
        a.first()
            .map(|v| v.to_string())
            .ok_or_else(|| format!("{name}() needs a unit string"))
    };
    Ok(match name {
        "to" => Value::Quantity(q.to(&unit_arg()?)?),
        "in" | "as" => Value::Float(q.to(&unit_arg()?)?.display_value()),
        "value" => Value::Float(q.display_value()),
        "unit" => Value::str(q.unit_name()),
        "si" => Value::Float(q.si),
        "dim" | "kind" => Value::str(units::describe(&q.dim)),
        "abs" => Value::Quantity(Quantity {
            si: q.si.abs(),
            ..q.clone()
        }),
        "round" => {
            let n = a.first().and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let p = 10f64.powi(n);
            let v = (q.display_value() * p).round() / p;
            let scale = q.unit.as_ref().map_or(1.0, |u| u.scale);
            Value::Quantity(Quantity {
                si: v * scale,
                ..q.clone()
            })
        }
        _ => return Err(format!("quantity has no method '{name}'")),
    })
}

/// Mermaid `stateDiagram-v2` source for a machine definition.
pub fn mermaid(def: &MachineDef) -> String {
    let mut out = String::from("stateDiagram-v2\n");
    out.push_str(&format!("    [*] --> {}\n", def.states[0].name));
    let label = |t: &Transition| {
        let mut l = t.event.clone();
        if !t.params.is_empty() {
            l.push_str(&format!("({})", t.params.join(", ")));
        }
        if t.guard.is_some() {
            l.push_str(" [guarded]");
        }
        l
    };
    for s in &def.states {
        for t in &s.transitions {
            let to = t.target.as_deref().unwrap_or(&s.name);
            out.push_str(&format!("    {} --> {} : {}\n", s.name, to, label(t)));
        }
    }
    for t in &def.global {
        if let Some(to) = &t.target {
            for s in &def.states {
                if &s.name != to {
                    out.push_str(&format!("    {} --> {} : {}\n", s.name, to, label(t)));
                }
            }
        }
    }
    out
}

// ============================================================================ generators

struct Gen(u64);

impl Gen {
    fn new(seed: u64) -> Self {
        Gen(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn unit_f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn int(&mut self) -> i64 {
        const EDGE: [i64; 12] = [0, 1, -1, 2, -2, 10, -10, 100, -100, 255, 256, 1000];
        match self.below(10) {
            0..=2 => self.below(21) as i64 - 10,
            3..=5 => self.below(2001) as i64 - 1000,
            6..=7 => EDGE[self.below(EDGE.len() as u64) as usize],
            _ => self.below(2_000_001) as i64 - 1_000_000,
        }
    }
    fn float(&mut self) -> f64 {
        const EDGE: [f64; 9] = [0.0, 1.0, -1.0, 0.5, -0.5, 1e-6, -1e-6, 100.0, -100.0];
        match self.below(10) {
            0..=3 => self.unit_f() * 20.0 - 10.0,
            4..=6 => self.unit_f() * 2000.0 - 1000.0,
            7..=8 => EDGE[self.below(EDGE.len() as u64) as usize],
            _ => self.unit_f() * 2e6 - 1e6,
        }
    }
    fn value(&mut self, ty: Option<&TypeAnn>) -> Value {
        match ty {
            Some(TypeAnn::Unit(u, _)) => Value::Quantity(Quantity::new(self.float(), u.clone())),
            Some(TypeAnn::Name(n, _)) => match n.as_str() {
                "int" => Value::Int(self.int()),
                "float" => Value::Float(self.float()),
                "bool" => Value::Bool(self.below(2) == 1),
                "str" => {
                    const S: [&str; 7] = ["", "a", "hello", "Tulpa", " ", "0", "αβγ"];
                    Value::str(S[self.below(S.len() as u64) as usize])
                }
                "list" => {
                    let n = self.below(6);
                    Value::list((0..n).map(|_| Value::Int(self.int())).collect())
                }
                _ => self.num(),
            },
            None => self.num(),
        }
    }
    fn num(&mut self) -> Value {
        if self.below(10) < 6 {
            Value::Int(self.int())
        } else {
            Value::Float(self.float())
        }
    }
}

fn shrink_candidates(v: &Value) -> Vec<Value> {
    match v {
        Value::Int(n) if *n != 0 => {
            let mut c = vec![Value::Int(0), Value::Int(n / 2), Value::Int(n - n.signum())];
            if *n < 0 {
                c.push(Value::Int(-n));
            }
            c
        }
        Value::Float(f) if *f != 0.0 => {
            let mut c = vec![
                Value::Float(0.0),
                Value::Float(f.trunc()),
                Value::Float(f / 2.0),
                Value::Float((f * 10.0).round() / 10.0),
            ];
            if *f < 0.0 {
                c.push(Value::Float(-f));
            }
            c.retain(|x| !x.equals(v));
            c
        }
        Value::Quantity(q) if q.si != 0.0 => {
            let scale = q.unit.as_ref().map_or(1.0, |u| u.scale);
            let dv = q.display_value();
            [0.0, dv.trunc(), dv / 2.0]
                .into_iter()
                .filter(|x| *x != dv)
                .map(|x| {
                    Value::Quantity(Quantity {
                        si: x * scale,
                        ..q.clone()
                    })
                })
                .collect()
        }
        Value::List(l) if !l.borrow().is_empty() => {
            let items = l.borrow().clone();
            vec![
                Value::list(vec![]),
                Value::list(items[..items.len() / 2].to_vec()),
                Value::list(items[1..].to_vec()),
            ]
        }
        Value::Str(s) if !s.is_empty() => vec![Value::str("")],
        Value::Bool(true) => vec![Value::Bool(false)],
        _ => vec![],
    }
}

/// Creates a fresh signal value.
pub fn new_signal(v: Value, capacity: usize, derive: Option<Value>) -> Value {
    let mut history = VecDeque::new();
    if derive.is_none() {
        history.push_back(v.clone());
    }
    Value::Signal(Rc::new(RefCell::new(Signal {
        value: v,
        prev: Value::Nil,
        changed_at: 0.0,
        prev_at: 0.0,
        history,
        capacity: capacity.max(1),
        derive,
        subscribers: Vec::new(),
    })))
}
