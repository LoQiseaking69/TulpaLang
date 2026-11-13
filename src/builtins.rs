//! Built-in functions, modules and methods.

use crate::devices;
use crate::interp::{compare, define, Ctrl, Env, Interpreter};
use crate::value::*;
use std::rc::Rc;

type Res = Result<Value, String>;
type I<'a> = &'a mut Interpreter;

// ---------------------------------------------------------------------------- arg helpers

fn arity(name: &str, args: &[Value], min: usize, max: usize) -> Result<(), String> {
    if args.len() < min || args.len() > max {
        let want = if min == max {
            min.to_string()
        } else {
            format!("{min}..{max}")
        };
        return Err(format!(
            "{name}() takes {want} argument(s), got {}",
            args.len()
        ));
    }
    Ok(())
}

pub fn num(name: &str, v: &Value) -> Result<f64, String> {
    v.as_f64()
        .ok_or_else(|| format!("{name}() expected a number, got {}", v.type_name()))
}

pub fn int(name: &str, v: &Value) -> Result<i64, String> {
    v.as_i64()
        .ok_or_else(|| format!("{name}() expected an integer, got {}", v.type_name()))
}

fn s_arg(name: &str, v: &Value) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        other => Err(format!(
            "{name}() expected a string, got {}",
            other.type_name()
        )),
    }
}

fn unwrap_ctrl(r: Result<Value, Ctrl>) -> Res {
    match r {
        Ok(v) | Err(Ctrl::Return(v)) => Ok(v),
        Err(Ctrl::Error(e)) => Err(e.to_string()),
        Err(_) => Err("'break' or 'continue' escaped a callback".into()),
    }
}

fn call(i: I, f: &Value, args: Vec<Value>) -> Res {
    unwrap_ctrl(i.call(f, args))
}

fn items(i: I, v: &Value) -> Result<Vec<Value>, String> {
    i.iterate(v, 1).map_err(|c| match c {
        Ctrl::Error(e) => e.msg,
        _ => "iteration error".into(),
    })
}

/// Turns a number back into an int when it has no fractional part and fits.
fn num_value(f: f64) -> Value {
    Value::Float(f)
}

// ---------------------------------------------------------------------------- registry

const GLOBALS: &[(&str, NativeFn)] = &[
    ("print", b_print),
    ("println", b_print),
    ("input", b_input),
    ("len", b_len),
    ("str", b_str),
    ("repr", b_repr),
    ("int", b_int),
    ("float", b_float),
    ("bool", |_, a| {
        arity("bool", &a, 1, 1)?;
        Ok(Value::Bool(a[0].truthy()))
    }),
    ("type", |_, a| {
        arity("type", &a, 1, 1)?;
        Ok(Value::str(a[0].type_name()))
    }),
    ("list", b_list),
    ("dict", b_dict),
    ("range", b_range),
    ("abs", b_abs),
    ("min", b_min),
    ("max", b_max),
    ("sum", b_sum),
    ("round", b_round),
    ("floor", |_, a| {
        m1("floor", a, f64::floor).map(to_int_if_whole)
    }),
    ("ceil", |_, a| m1("ceil", a, f64::ceil).map(to_int_if_whole)),
    ("sqrt", |_, a| m1("sqrt", a, f64::sqrt)),
    ("sin", |_, a| m1("sin", a, f64::sin)),
    ("cos", |_, a| m1("cos", a, f64::cos)),
    ("tan", |_, a| m1("tan", a, f64::tan)),
    ("asin", |_, a| m1("asin", a, f64::asin)),
    ("acos", |_, a| m1("acos", a, f64::acos)),
    ("atan", |_, a| m1("atan", a, f64::atan)),
    ("exp", |_, a| m1("exp", a, f64::exp)),
    ("log", b_log),
    ("log10", |_, a| m1("log10", a, f64::log10)),
    ("atan2", |_, a| {
        arity("atan2", &a, 2, 2)?;
        Ok(Value::Float(
            num("atan2", &a[0])?.atan2(num("atan2", &a[1])?),
        ))
    }),
    ("hypot", |_, a| {
        arity("hypot", &a, 2, 2)?;
        Ok(Value::Float(
            num("hypot", &a[0])?.hypot(num("hypot", &a[1])?),
        ))
    }),
    ("pow", |_, a| {
        arity("pow", &a, 2, 2)?;
        crate::interp::binary(&a[0], "**", &a[1])
    }),
    ("radians", |_, a| m1("radians", a, f64::to_radians)),
    ("degrees", |_, a| m1("degrees", a, f64::to_degrees)),
    ("clamp", b_clamp),
    ("lerp", |_, a| {
        arity("lerp", &a, 3, 3)?;
        let (x, y, t) = (
            num("lerp", &a[0])?,
            num("lerp", &a[1])?,
            num("lerp", &a[2])?,
        );
        Ok(Value::Float(x + (y - x) * t))
    }),
    ("map", b_map),
    ("filter", b_filter),
    ("reduce", b_reduce),
    ("sorted", b_sorted),
    ("reversed", |i, a| {
        arity("reversed", &a, 1, 1)?;
        let mut v = items(i, &a[0])?;
        v.reverse();
        Ok(Value::list(v))
    }),
    ("enumerate", b_enumerate),
    ("zip", b_zip),
    ("any", |i, a| {
        arity("any", &a, 1, 1)?;
        Ok(Value::Bool(items(i, &a[0])?.iter().any(|v| v.truthy())))
    }),
    ("all", |i, a| {
        arity("all", &a, 1, 1)?;
        Ok(Value::Bool(items(i, &a[0])?.iter().all(|v| v.truthy())))
    }),
    ("keys", |i, a| {
        method(i, &a.first().cloned().unwrap_or(Value::Nil), "keys", vec![])
    }),
    ("values", |i, a| {
        method(
            i,
            &a.first().cloned().unwrap_or(Value::Nil),
            "values",
            vec![],
        )
    }),
    ("items", |i, a| {
        method(
            i,
            &a.first().cloned().unwrap_or(Value::Nil),
            "items",
            vec![],
        )
    }),
    ("sleep", b_sleep),
    ("sleep_ms", |_, a| {
        arity("sleep_ms", &a, 1, 1)?;
        std::thread::sleep(std::time::Duration::from_secs_f64(
            num("sleep_ms", &a[0])?.max(0.0) / 1000.0,
        ));
        Ok(Value::Nil)
    }),
    ("millis", |i, _| {
        Ok(Value::Int(i.start.elapsed().as_millis() as i64))
    }),
    ("micros", |i, _| {
        Ok(Value::Int(i.start.elapsed().as_micros() as i64))
    }),
    ("now", |_, _| {
        Ok(Value::Float(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
        ))
    }),
    ("randint", b_randint),
    ("random", |_, _| Ok(Value::Float(rand_f64()))),
    ("choice", |i, a| {
        arity("choice", &a, 1, 1)?;
        let v = items(i, &a[0])?;
        if v.is_empty() {
            return Err("choice() from empty sequence".into());
        }
        Ok(v[(rand_u64() % v.len() as u64) as usize].clone())
    }),
    ("read_file", |_, a| {
        arity("read_file", &a, 1, 1)?;
        std::fs::read_to_string(s_arg("read_file", &a[0])?)
            .map(Value::str)
            .map_err(|e| e.to_string())
    }),
    ("write_file", |_, a| {
        arity("write_file", &a, 2, 2)?;
        std::fs::write(s_arg("write_file", &a[0])?, a[1].to_string())
            .map(|_| Value::Nil)
            .map_err(|e| e.to_string())
    }),
    ("append_file", b_append_file),
    ("format", b_format),
    ("hex", |_, a| {
        arity("hex", &a, 1, 1)?;
        let n = int("hex", &a[0])?;
        Ok(Value::str(if n < 0 {
            format!("-0x{:x}", -n)
        } else {
            format!("0x{n:x}")
        }))
    }),
    ("bin", |_, a| {
        arity("bin", &a, 1, 1)?;
        let n = int("bin", &a[0])?;
        Ok(Value::str(if n < 0 {
            format!("-0b{:b}", -n)
        } else {
            format!("0b{n:b}")
        }))
    }),
    ("ord", |_, a| {
        arity("ord", &a, 1, 1)?;
        let s = s_arg("ord", &a[0])?;
        s.chars()
            .next()
            .map(|c| Value::Int(c as i64))
            .ok_or_else(|| "ord() of empty string".into())
    }),
    ("chr", |_, a| {
        arity("chr", &a, 1, 1)?;
        char::from_u32(int("chr", &a[0])? as u32)
            .map(|c| Value::str(c.to_string()))
            .ok_or_else(|| "chr() arg out of range".into())
    }),
    ("bytes", |_, a| {
        arity("bytes", &a, 1, 1)?;
        Ok(Value::from_bytes(&a[0].to_bytes()?))
    }),
    ("decode", |_, a| {
        arity("decode", &a, 1, 1)?;
        Ok(Value::str(
            String::from_utf8_lossy(&a[0].to_bytes()?).to_string(),
        ))
    }),
    ("int_from_bytes", b_int_from_bytes),
    ("int_to_bytes", b_int_to_bytes),
    ("error", |_, a| {
        Err(a
            .first()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "error".into()))
    }),
    ("exit", |_, a| {
        std::process::exit(a.first().and_then(|v| v.as_i64()).unwrap_or(0) as i32)
    }),
    ("join_all", |i, _| {
        i.drain_tasks()
            .map(|_| Value::Nil)
            .map_err(|e| e.to_string())
    }),
    ("await_all", |i, a| {
        arity("await_all", &a, 1, 1)?;
        unwrap_ctrl(i.await_value(a[0].clone()))
    }),
];

pub fn install(env: &Env) {
    for (name, f) in GLOBALS {
        define(env, name, Value::Builtin(name, *f));
    }
    define(env, "PI", Value::Float(std::f64::consts::PI));
    define(env, "TAU", Value::Float(std::f64::consts::TAU));
    define(env, "E", Value::Float(std::f64::consts::E));
    define(env, "HIGH", Value::Int(1));
    define(env, "LOW", Value::Int(0));
}

const MODULES: &[&str] = &[
    "std",
    "math",
    "kin",
    "kinematics",
    "time",
    "io",
    "sim",
    "control",
    "gpio",
    "i2c",
    "spi",
    "uart",
    "GPIO",
    "I2C",
    "SPI",
    "UART",
    "PID",
];

pub fn module_exists(m: &str) -> bool {
    MODULES.contains(&m)
}

fn global(name: &'static str) -> Option<Value> {
    GLOBALS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, f)| Value::Builtin(n, *f))
}

/// Resolves `Module::name`.
pub fn path(m: &str, name: &str) -> Option<Value> {
    match (m, name) {
        ("math", "pi") => Some(Value::Float(std::f64::consts::PI)),
        ("math", "tau") => Some(Value::Float(std::f64::consts::TAU)),
        ("math", "e") => Some(Value::Float(std::f64::consts::E)),
        ("math", "inf") => Some(Value::Float(f64::INFINITY)),
        ("math", "nan") => Some(Value::Float(f64::NAN)),
        ("math", _) | ("std", _) | ("io", _) => GLOBALS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(n, f)| Value::Builtin(n, *f)),
        ("time", "millis") => global("millis"),
        ("time", "micros") => global("micros"),
        ("time", "now") => global("now"),
        ("time", "sleep") => global("sleep"),
        ("time", "sleep_ms") => global("sleep_ms"),
        ("kin" | "kinematics", _) => KIN
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(n, f)| Value::Builtin(n, *f)),
        _ => devices::path(m, name),
    }
}

// ---------------------------------------------------------------------------- kinematics

const G: f64 = 9.81;

const KIN: &[(&str, NativeFn)] = &[
    ("velocity", |_, a| {
        arity("kin::velocity", &a, 3, 3)?;
        Ok(num_value(
            num("velocity", &a[0])? + num("velocity", &a[1])? * num("velocity", &a[2])?,
        ))
    }),
    ("position", |_, a| {
        arity("kin::position", &a, 4, 4)?;
        let (x0, v0, acc, t) = (
            num("position", &a[0])?,
            num("position", &a[1])?,
            num("position", &a[2])?,
            num("position", &a[3])?,
        );
        Ok(num_value(x0 + v0 * t + 0.5 * acc * t * t))
    }),
    ("acceleration", |_, a| {
        arity("kin::acceleration", &a, 2, 2)?;
        let t = num("acceleration", &a[1])?;
        if t == 0.0 {
            return Err("kin::acceleration: time must be non-zero".into());
        }
        Ok(num_value(num("acceleration", &a[0])? / t))
    }),
    ("time_of_flight", |_, a| {
        arity("kin::time_of_flight", &a, 2, 2)?;
        Ok(Value::Float(
            2.0 * num("tof", &a[0])? * num("tof", &a[1])?.to_radians().sin() / G,
        ))
    }),
    ("range", |_, a| {
        arity("kin::range", &a, 2, 2)?;
        Ok(Value::Float(
            num("range", &a[0])?.powi(2) * (2.0 * num("range", &a[1])?).to_radians().sin() / G,
        ))
    }),
    ("max_height", |_, a| {
        arity("kin::max_height", &a, 2, 2)?;
        Ok(Value::Float(
            (num("max_height", &a[0])? * num("max_height", &a[1])?.to_radians().sin()).powi(2)
                / (2.0 * G),
        ))
    }),
    // Planar 2-link arm. Angles in degrees.
    ("fk2", |_, a| {
        arity("kin::fk2", &a, 4, 4)?;
        let (l1, l2) = (num("fk2", &a[0])?, num("fk2", &a[1])?);
        let (t1, t2) = (
            num("fk2", &a[2])?.to_radians(),
            num("fk2", &a[3])?.to_radians(),
        );
        let x = l1 * t1.cos() + l2 * (t1 + t2).cos();
        let y = l1 * t1.sin() + l2 * (t1 + t2).sin();
        Ok(Value::list(vec![Value::Float(x), Value::Float(y)]))
    }),
    ("ik2", |_, a| {
        arity("kin::ik2", &a, 4, 5)?;
        let (l1, l2, x, y) = (
            num("ik2", &a[0])?,
            num("ik2", &a[1])?,
            num("ik2", &a[2])?,
            num("ik2", &a[3])?,
        );
        let elbow_up = a.get(4).is_some_and(|v| v.truthy());
        let c2 = (x * x + y * y - l1 * l1 - l2 * l2) / (2.0 * l1 * l2);
        if !(-1.0..=1.0).contains(&c2) {
            return Err(format!("kin::ik2: target ({x}, {y}) is out of reach"));
        }
        let mut t2 = c2.acos();
        if elbow_up {
            t2 = -t2;
        }
        let t1 = y.atan2(x) - (l2 * t2.sin()).atan2(l1 + l2 * t2.cos());
        Ok(Value::list(vec![
            Value::Float(t1.to_degrees()),
            Value::Float(t2.to_degrees()),
        ]))
    }),
];

// ---------------------------------------------------------------------------- globals

fn b_print(i: I, a: Vec<Value>) -> Res {
    let s: Vec<String> = a.iter().map(|v| v.to_string()).collect();
    i.write_out(&format!("{}\n", s.join(" ")));
    Ok(Value::Nil)
}

fn b_input(i: I, a: Vec<Value>) -> Res {
    if let Some(p) = a.first() {
        i.write_out(&p.to_string());
    }
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(Value::str(line.trim_end_matches(['\n', '\r'])))
}

fn b_len(_: I, a: Vec<Value>) -> Res {
    arity("len", &a, 1, 1)?;
    Ok(Value::Int(match &a[0] {
        Value::Str(s) => s.chars().count() as i64,
        Value::List(l) => l.borrow().len() as i64,
        Value::Dict(d) => d.borrow().len() as i64,
        Value::Range(x, y, incl) => (if *incl { y - x + 1 } else { y - x }).max(0),
        other => return Err(format!("len() of {}", other.type_name())),
    }))
}

fn b_str(_: I, a: Vec<Value>) -> Res {
    arity("str", &a, 0, 1)?;
    Ok(Value::str(
        a.first().map(|v| v.to_string()).unwrap_or_default(),
    ))
}

fn b_repr(_: I, a: Vec<Value>) -> Res {
    arity("repr", &a, 1, 1)?;
    Ok(Value::str(a[0].repr()))
}

fn b_int(_: I, a: Vec<Value>) -> Res {
    arity("int", &a, 1, 2)?;
    match &a[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(f) => Ok(Value::Int(f.trunc() as i64)),
        Value::Bool(b) => Ok(Value::Int(*b as i64)),
        Value::Str(s) => {
            let radix = a.get(1).map(|r| int("int", r)).transpose()?.unwrap_or(10) as u32;
            let t = s.trim().replace('_', "");
            let (neg, t) = match t.strip_prefix('-') {
                Some(r) => (true, r.to_string()),
                None => (false, t),
            };
            let (radix, body) = if radix == 10 {
                if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    (16, h.to_string())
                } else if let Some(b) = t.strip_prefix("0b") {
                    (2, b.to_string())
                } else {
                    (10, t)
                }
            } else {
                (radix, t)
            };
            i64::from_str_radix(&body, radix)
                .map(|n| Value::Int(if neg { -n } else { n }))
                .map_err(|_| format!("int(): invalid literal {:?}", s.as_ref()))
        }
        other => Err(format!("int() of {}", other.type_name())),
    }
}

fn b_float(_: I, a: Vec<Value>) -> Res {
    arity("float", &a, 1, 1)?;
    match &a[0] {
        Value::Str(s) => s
            .trim()
            .parse()
            .map(Value::Float)
            .map_err(|_| format!("float(): invalid literal {:?}", s.as_ref())),
        v => num("float", v).map(Value::Float),
    }
}

fn b_list(i: I, a: Vec<Value>) -> Res {
    arity("list", &a, 0, 1)?;
    match a.first() {
        None => Ok(Value::list(vec![])),
        Some(v) => Ok(Value::list(items(i, v)?)),
    }
}

fn b_dict(i: I, a: Vec<Value>) -> Res {
    arity("dict", &a, 0, 1)?;
    let mut d = Dict::default();
    if let Some(src) = a.first() {
        if let Value::Dict(x) = src {
            return Ok(Value::dict(x.borrow().clone()));
        }
        for pair in items(i, src)? {
            match pair {
                Value::List(p) if p.borrow().len() == 2 => {
                    let p = p.borrow();
                    d.set(p[0].clone(), p[1].clone())?;
                }
                _ => return Err("dict() expects a list of [key, value] pairs".into()),
            }
        }
    }
    Ok(Value::dict(d))
}

fn b_range(_: I, a: Vec<Value>) -> Res {
    arity("range", &a, 1, 3)?;
    let ints: Vec<i64> = a
        .iter()
        .map(|v| int("range", v))
        .collect::<Result<_, _>>()?;
    match ints.as_slice() {
        [n] => Ok(Value::Range(0, *n, false)),
        [x, y] => Ok(Value::Range(*x, *y, false)),
        [x, y, step] => {
            if *step == 0 {
                return Err("range() step must not be zero".into());
            }
            let mut v = Vec::new();
            let mut k = *x;
            while (*step > 0 && k < *y) || (*step < 0 && k > *y) {
                v.push(Value::Int(k));
                k += step;
            }
            Ok(Value::list(v))
        }
        _ => unreachable!(),
    }
}

fn b_abs(_: I, a: Vec<Value>) -> Res {
    arity("abs", &a, 1, 1)?;
    match &a[0] {
        Value::Int(n) => Ok(Value::Int(n.abs())),
        v => Ok(Value::Float(num("abs", v)?.abs())),
    }
}

fn extreme(i: I, name: &str, a: Vec<Value>, want_max: bool) -> Res {
    let vals = if a.len() == 1 { items(i, &a[0])? } else { a };
    let mut best: Option<Value> = None;
    for v in vals {
        best = Some(match best {
            None => v,
            Some(b) => {
                let o = compare(&v, &b)?;
                if (want_max && o.is_gt()) || (!want_max && o.is_lt()) {
                    v
                } else {
                    b
                }
            }
        });
    }
    best.ok_or_else(|| format!("{name}() of empty sequence"))
}

fn b_min(i: I, a: Vec<Value>) -> Res {
    extreme(i, "min", a, false)
}
fn b_max(i: I, a: Vec<Value>) -> Res {
    extreme(i, "max", a, true)
}

fn b_sum(i: I, a: Vec<Value>) -> Res {
    arity("sum", &a, 1, 2)?;
    let mut acc = a.get(1).cloned().unwrap_or(Value::Int(0));
    for v in items(i, &a[0])? {
        acc = crate::interp::binary(&acc, "+", &v)?;
    }
    Ok(acc)
}

fn to_int_if_whole(v: Value) -> Value {
    match v {
        Value::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Value::Int(f as i64),
        other => other,
    }
}

fn m1(name: &str, a: Vec<Value>, f: fn(f64) -> f64) -> Res {
    arity(name, &a, 1, 1)?;
    Ok(Value::Float(f(num(name, &a[0])?)))
}

fn b_log(_: I, a: Vec<Value>) -> Res {
    arity("log", &a, 1, 2)?;
    let x = num("log", &a[0])?;
    Ok(Value::Float(match a.get(1) {
        Some(b) => x.log(num("log", b)?),
        None => x.ln(),
    }))
}

fn b_round(_: I, a: Vec<Value>) -> Res {
    arity("round", &a, 1, 2)?;
    let x = num("round", &a[0])?;
    match a.get(1) {
        None => Ok(Value::Int(x.round() as i64)),
        Some(d) => {
            let p = 10f64.powi(int("round", d)? as i32);
            Ok(Value::Float((x * p).round() / p))
        }
    }
}

fn b_clamp(_: I, a: Vec<Value>) -> Res {
    arity("clamp", &a, 3, 3)?;
    if compare(&a[0], &a[1])?.is_lt() {
        Ok(a[1].clone())
    } else if compare(&a[0], &a[2])?.is_gt() {
        Ok(a[2].clone())
    } else {
        Ok(a[0].clone())
    }
}

/// Accepts both `map(f, xs)` (Python) and `map(xs, f)` orders.
fn fn_and_seq(name: &str, a: &[Value]) -> Result<(Value, Value), String> {
    let is_fn = |v: &Value| matches!(v, Value::Func(_) | Value::Builtin(..));
    match (a.first(), a.get(1)) {
        (Some(f), Some(s)) if is_fn(f) => Ok((f.clone(), s.clone())),
        (Some(s), Some(f)) if is_fn(f) => Ok((f.clone(), s.clone())),
        _ => Err(format!("{name}() expects a function and a sequence")),
    }
}

fn b_map(i: I, a: Vec<Value>) -> Res {
    arity("map", &a, 2, 2)?;
    let (f, s) = fn_and_seq("map", &a)?;
    let mut out = Vec::new();
    for v in items(i, &s)? {
        out.push(call(i, &f, vec![v])?);
    }
    Ok(Value::list(out))
}

fn b_filter(i: I, a: Vec<Value>) -> Res {
    arity("filter", &a, 2, 2)?;
    let (f, s) = fn_and_seq("filter", &a)?;
    let mut out = Vec::new();
    for v in items(i, &s)? {
        if call(i, &f, vec![v.clone()])?.truthy() {
            out.push(v);
        }
    }
    Ok(Value::list(out))
}

fn b_reduce(i: I, a: Vec<Value>) -> Res {
    arity("reduce", &a, 2, 3)?;
    let (f, s) = fn_and_seq("reduce", &a[..2])?;
    let mut it = items(i, &s)?.into_iter();
    let mut acc = match a.get(2) {
        Some(init) => init.clone(),
        None => it
            .next()
            .ok_or("reduce() of empty sequence with no initial value")?,
    };
    for v in it {
        acc = call(i, &f, vec![acc, v])?;
    }
    Ok(acc)
}

fn sort_values(i: I, mut v: Vec<Value>, key: Option<&Value>) -> Result<Vec<Value>, String> {
    let keys: Vec<Value> = match key {
        Some(k) => v
            .iter()
            .map(|x| call(i, k, vec![x.clone()]))
            .collect::<Result<_, _>>()?,
        None => v.clone(),
    };
    let mut idx: Vec<usize> = (0..v.len()).collect();
    let mut err = None;
    idx.sort_by(|&x, &y| {
        compare(&keys[x], &keys[y]).unwrap_or_else(|e| {
            err.get_or_insert(e);
            std::cmp::Ordering::Equal
        })
    });
    if let Some(e) = err {
        return Err(e);
    }
    let src = std::mem::take(&mut v);
    Ok(idx.into_iter().map(|k| src[k].clone()).collect())
}

fn b_sorted(i: I, a: Vec<Value>) -> Res {
    arity("sorted", &a, 1, 2)?;
    let v = items(i, &a[0])?;
    Ok(Value::list(sort_values(i, v, a.get(1))?))
}

fn b_enumerate(i: I, a: Vec<Value>) -> Res {
    arity("enumerate", &a, 1, 2)?;
    let start = a
        .get(1)
        .map(|s| int("enumerate", s))
        .transpose()?
        .unwrap_or(0);
    Ok(Value::list(
        items(i, &a[0])?
            .into_iter()
            .enumerate()
            .map(|(k, v)| Value::list(vec![Value::Int(k as i64 + start), v]))
            .collect(),
    ))
}

fn b_zip(i: I, a: Vec<Value>) -> Res {
    let lists: Vec<Vec<Value>> = a.iter().map(|v| items(i, v)).collect::<Result<_, _>>()?;
    let n = lists.iter().map(|l| l.len()).min().unwrap_or(0);
    Ok(Value::list(
        (0..n)
            .map(|k| Value::list(lists.iter().map(|l| l[k].clone()).collect()))
            .collect(),
    ))
}

fn b_sleep(_: I, a: Vec<Value>) -> Res {
    arity("sleep", &a, 1, 1)?;
    let s = num("sleep", &a[0])?;
    std::thread::sleep(std::time::Duration::from_secs_f64(s.max(0.0)));
    Ok(Value::Nil)
}

fn b_append_file(_: I, a: Vec<Value>) -> Res {
    use std::io::Write;
    arity("append_file", &a, 2, 2)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(s_arg("append_file", &a[0])?)
        .map_err(|e| e.to_string())?;
    f.write_all(a[1].to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(Value::Nil)
}

fn b_format(_: I, a: Vec<Value>) -> Res {
    arity("format", &a, 1, 2)?;
    let spec = match a.get(1) {
        Some(s) => s_arg("format", s)?,
        None => String::new(),
    };
    Ok(Value::str(format_spec(&a[0], &spec)?))
}

fn b_int_from_bytes(_: I, a: Vec<Value>) -> Res {
    arity("int_from_bytes", &a, 1, 3)?;
    let b = a[0].to_bytes()?;
    if b.is_empty() || b.len() > 8 {
        return Err("int_from_bytes() needs 1..8 bytes".into());
    }
    let little = matches!(a.get(1), Some(Value::Str(s)) if s.as_ref() == "little");
    let signed = a.get(2).is_some_and(|v| v.truthy());
    let mut n: u64 = 0;
    let ordered: Vec<u8> = if little {
        b.iter().rev().copied().collect()
    } else {
        b.clone()
    };
    for x in &ordered {
        n = (n << 8) | *x as u64;
    }
    let bits = b.len() * 8;
    let v = if signed && bits < 64 && (n >> (bits - 1)) & 1 == 1 {
        n as i64 - (1i64 << bits)
    } else {
        n as i64
    };
    Ok(Value::Int(v))
}

fn b_int_to_bytes(_: I, a: Vec<Value>) -> Res {
    arity("int_to_bytes", &a, 2, 3)?;
    let n = int("int_to_bytes", &a[0])?;
    let len = int("int_to_bytes", &a[1])? as usize;
    if len == 0 || len > 8 {
        return Err("int_to_bytes() length must be 1..8".into());
    }
    let little = matches!(a.get(2), Some(Value::Str(s)) if s.as_ref() == "little");
    let mut out: Vec<u8> = (0..len)
        .rev()
        .map(|k| ((n >> (8 * k)) & 0xff) as u8)
        .collect();
    if little {
        out.reverse();
    }
    Ok(Value::from_bytes(&out))
}

// ---------------------------------------------------------------------------- randomness

thread_local! {
    static RNG: std::cell::Cell<u64> = std::cell::Cell::new({
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9e3779b97f4a7c15);
        t | 1
    });
}

fn rand_u64() -> u64 {
    RNG.with(|r| {
        // xorshift64*
        let mut x = r.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        r.set(x);
        x.wrapping_mul(0x2545F4914F6CDD1D)
    })
}

fn rand_f64() -> f64 {
    (rand_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn b_randint(_: I, a: Vec<Value>) -> Res {
    arity("randint", &a, 2, 2)?;
    let (lo, hi) = (int("randint", &a[0])?, int("randint", &a[1])?);
    if hi < lo {
        return Err("randint(): empty range".into());
    }
    let span = (hi - lo) as u64 + 1;
    Ok(Value::Int(lo + (rand_u64() % span) as i64))
}

// ---------------------------------------------------------------------------- formatting

pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let ms = secs % 1000;
    let s = (secs / 1000) % 86400;
    format!(
        "{:02}:{:02}:{:02}.{:03} UTC",
        s / 3600,
        (s / 60) % 60,
        s % 60,
        ms
    )
}

/// Formats a value with a Python/Rust style spec: `[[fill]align][#][0][width][.prec][type]`.
pub fn format_spec(v: &Value, spec: &str) -> Result<String, String> {
    if spec.is_empty() {
        return Ok(v.to_string());
    }
    if spec == "?" {
        return Ok(v.repr());
    }
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;
    let (mut fill, mut align) = (' ', None);
    if chars.len() >= 2 && matches!(chars[1], '<' | '>' | '^') {
        fill = chars[0];
        align = Some(chars[1]);
        i = 2;
    } else if matches!(chars.first(), Some('<' | '>' | '^')) {
        align = Some(chars[0]);
        i = 1;
    }
    let mut plus = false;
    if chars.get(i) == Some(&'+') {
        plus = true;
        i += 1;
    }
    let mut alt = false;
    if chars.get(i) == Some(&'#') {
        alt = true;
        i += 1;
    }
    let mut zero = false;
    if chars.get(i) == Some(&'0') {
        zero = true;
        i += 1;
    }
    let ws = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    let width: usize = chars[ws..i].iter().collect::<String>().parse().unwrap_or(0);
    let mut prec = None;
    if chars.get(i) == Some(&'.') {
        i += 1;
        let ps = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        prec = Some(
            chars[ps..i]
                .iter()
                .collect::<String>()
                .parse::<usize>()
                .map_err(|_| format!("bad precision in format spec '{spec}'"))?,
        );
    }
    let ty: String = chars[i..].iter().collect();
    let as_int = |v: &Value| {
        v.as_i64()
            .ok_or_else(|| format!("format type '{ty}' needs an integer, got {}", v.type_name()))
    };
    let mut body = match ty.as_str() {
        "x" => {
            let n = as_int(v)?;
            format!("{}{n:x}", if alt { "0x" } else { "" })
        }
        "X" => {
            let n = as_int(v)?;
            format!("{}{n:X}", if alt { "0x" } else { "" })
        }
        "b" => {
            let n = as_int(v)?;
            format!("{}{n:b}", if alt { "0b" } else { "" })
        }
        "o" => {
            let n = as_int(v)?;
            format!("{}{n:o}", if alt { "0o" } else { "" })
        }
        "d" => as_int(v)?.to_string(),
        "e" => format!("{:.*e}", prec.unwrap_or(6), num("format", v)?),
        "%" => format!("{:.*}%", prec.unwrap_or(6), num("format", v)? * 100.0),
        "f" => format!("{:.*}", prec.unwrap_or(6), num("format", v)?),
        "s" | "" => match (prec, v) {
            (Some(p), Value::Int(_) | Value::Float(_)) => format!("{:.*}", p, v.as_f64().unwrap()),
            (Some(p), _) => v.to_string().chars().take(p).collect(),
            (None, _) => v.to_string(),
        },
        "?" => v.repr(),
        other => return Err(format!("unknown format type '{other}'")),
    };
    if plus && v.as_f64().is_some_and(|f| f >= 0.0) && matches!(v, Value::Int(_) | Value::Float(_))
    {
        body.insert(0, '+');
    }
    let len = body.chars().count();
    if width > len {
        let pad = width - len;
        let numeric = matches!(v, Value::Int(_) | Value::Float(_));
        if zero && align.is_none() {
            let (sign, rest) = if body.starts_with(['-', '+']) {
                body.split_at(1)
            } else {
                ("", body.as_str())
            };
            let (prefix, digits) = if alt && rest.len() > 2 && rest.starts_with('0') {
                rest.split_at(2)
            } else {
                ("", rest)
            };
            return Ok(format!("{sign}{prefix}{}{digits}", "0".repeat(pad)));
        }
        let f = fill.to_string();
        body = match align.unwrap_or(if numeric { '>' } else { '<' }) {
            '<' => format!("{body}{}", f.repeat(pad)),
            '>' => format!("{}{body}", f.repeat(pad)),
            _ => format!("{}{body}{}", f.repeat(pad / 2), f.repeat(pad - pad / 2)),
        };
    }
    Ok(body)
}

/// Rust-style `format!` strings: `{}`, `{:?}`, `{:.2}`, `{:#04x}`, `{0}`, `{{`/`}}`.
pub fn rust_format(fmt: &str, args: &[Value]) -> Result<String, String> {
    let mut out = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let (mut i, mut next) = (0, 0);
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                out.push('{');
                i += 2;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                out.push('}');
                i += 2;
            }
            '{' => {
                let end = chars[i..]
                    .iter()
                    .position(|c| *c == '}')
                    .ok_or("unclosed '{' in format string")?
                    + i;
                let inner: String = chars[i + 1..end].iter().collect();
                let (pos, spec) = match inner.split_once(':') {
                    Some((p, s)) => (p.to_string(), s.to_string()),
                    None => (inner.clone(), String::new()),
                };
                let idx = if pos.is_empty() {
                    next += 1;
                    next - 1
                } else {
                    pos.parse::<usize>().map_err(|_| {
                        format!("named format arguments are not supported: {{{inner}}}")
                    })?
                };
                let v = args.get(idx).ok_or_else(|| {
                    format!(
                        "format string refers to argument {idx}, but only {} given",
                        args.len()
                    )
                })?;
                out.push_str(&format_spec(v, &spec)?);
                i = end + 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------- methods

pub fn method(i: I, recv: &Value, name: &str, a: Vec<Value>) -> Res {
    match recv {
        Value::Str(s) => str_method(i, s, name, a),
        Value::List(l) => list_method(i, l, name, a),
        Value::Dict(d) => dict_method(i, d, name, a),
        Value::Range(..) => match name {
            "list" | "to_list" => Ok(Value::list(items(i, recv)?)),
            "contains" => {
                crate::interp::binary(a.first().ok_or("contains() needs an argument")?, "in", recv)
            }
            "len" => b_len(i, vec![recv.clone()]),
            _ => {
                let v = items(i, recv)?;
                list_method(i, &Rc::new(std::cell::RefCell::new(v)), name, a)
            }
        },
        Value::Int(_) | Value::Float(_) => match name {
            "abs" => b_abs(i, vec![recv.clone()]),
            "round" => {
                let mut v = vec![recv.clone()];
                v.extend(a);
                b_round(i, v)
            }
            "format" => {
                let mut v = vec![recv.clone()];
                v.extend(a);
                b_format(i, v)
            }
            "clamp" => {
                let mut v = vec![recv.clone()];
                v.extend(a);
                b_clamp(i, v)
            }
            "to_hex" | "hex" => Ok(Value::str(format!("0x{:x}", int("hex", recv)?))),
            "bit" => {
                let b = int("bit", a.first().ok_or("bit() needs an index")?)?;
                Ok(Value::Int((int("bit", recv)? >> b) & 1))
            }
            _ => Err(format!("{} has no method '{name}'", recv.type_name())),
        },
        Value::Task(_) => match name {
            "done" => Ok(Value::Bool(matches!(
                &*match recv {
                    Value::Task(t) => t.borrow(),
                    _ => unreachable!(),
                },
                Task::Done(_) | Task::Failed(_)
            ))),
            "result" | "await" => unwrap_ctrl(i.await_value(recv.clone())),
            _ => Err(format!("task has no method '{name}'")),
        },
        Value::Func(_) | Value::Builtin(..) => match name {
            "call" => call(i, recv, a),
            _ => Err(format!("fn has no method '{name}'")),
        },
        other => Err(format!("{} has no method '{name}'", other.type_name())),
    }
}

fn str_method(i: I, s: &Rc<str>, name: &str, a: Vec<Value>) -> Res {
    let arg_s = |k: usize| -> Result<String, String> {
        a.get(k)
            .map(|v| s_arg(name, v))
            .unwrap_or_else(|| Err(format!("{name}() needs a string argument")))
    };
    Ok(match name {
        "len" => Value::Int(s.chars().count() as i64),
        "upper" => Value::str(s.to_uppercase()),
        "lower" => Value::str(s.to_lowercase()),
        "strip" | "trim" => Value::str(s.trim()),
        "lstrip" => Value::str(s.trim_start()),
        "rstrip" => Value::str(s.trim_end()),
        "split" => {
            let parts: Vec<Value> = match a.first() {
                Some(_) => s.split(arg_s(0)?.as_str()).map(Value::str).collect(),
                None => s.split_whitespace().map(Value::str).collect(),
            };
            Value::list(parts)
        }
        "lines" => Value::list(s.lines().map(Value::str).collect()),
        "chars" => Value::list(s.chars().map(|c| Value::str(c.to_string())).collect()),
        "replace" => Value::str(s.replace(arg_s(0)?.as_str(), arg_s(1)?.as_str())),
        "contains" => Value::Bool(s.contains(arg_s(0)?.as_str())),
        "starts_with" | "startswith" => Value::Bool(s.starts_with(arg_s(0)?.as_str())),
        "ends_with" | "endswith" => Value::Bool(s.ends_with(arg_s(0)?.as_str())),
        "find" => {
            let sub = arg_s(0)?;
            match s.find(sub.as_str()) {
                Some(b) => Value::Int(s[..b].chars().count() as i64),
                None => Value::Int(-1),
            }
        }
        "count" => Value::Int(s.matches(arg_s(0)?.as_str()).count() as i64),
        "join" => {
            let parts: Vec<String> = items(i, a.first().ok_or("join() needs a list")?)?
                .iter()
                .map(|v| v.to_string())
                .collect();
            Value::str(parts.join(s))
        }
        "repeat" => Value::str(
            s.repeat(int("repeat", a.first().ok_or("repeat() needs a count")?)?.max(0) as usize),
        ),
        "format" => Value::str(rust_format(s, &a)?),
        "bytes" | "encode" => Value::from_bytes(s.as_bytes()),
        "is_digit" | "isdigit" => {
            Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        }
        "is_alpha" | "isalpha" => Value::Bool(!s.is_empty() && s.chars().all(char::is_alphabetic)),
        "to_int" => b_int(i, vec![Value::Str(s.clone())])?,
        "to_float" => b_float(i, vec![Value::Str(s.clone())])?,
        "pad_left" | "pad_right" => {
            let w = int(name, a.first().ok_or("pad needs a width")?)? as usize;
            let f = a
                .get(1)
                .map(|v| v.to_string())
                .unwrap_or_else(|| " ".into());
            let n = s.chars().count();
            let pad = f.repeat(w.saturating_sub(n));
            Value::str(if name == "pad_left" {
                format!("{pad}{s}")
            } else {
                format!("{s}{pad}")
            })
        }
        _ => return Err(format!("str has no method '{name}'")),
    })
}

fn list_method(i: I, l: &Rc<std::cell::RefCell<Vec<Value>>>, name: &str, a: Vec<Value>) -> Res {
    let first = || {
        a.first()
            .cloned()
            .ok_or_else(|| format!("{name}() needs an argument"))
    };
    Ok(match name {
        "len" => Value::Int(l.borrow().len() as i64),
        "push" | "append" => {
            l.borrow_mut().push(first()?);
            Value::Nil
        }
        "extend" => {
            let more = items(i, &first()?)?;
            l.borrow_mut().extend(more);
            Value::Nil
        }
        "pop" => {
            let mut v = l.borrow_mut();
            match a.first() {
                None => v.pop().ok_or("pop() from empty list")?,
                Some(k) => {
                    let n = v.len() as i64;
                    let mut k = int("pop", k)?;
                    if k < 0 {
                        k += n;
                    }
                    if k < 0 || k >= n {
                        return Err("pop() index out of range".into());
                    }
                    v.remove(k as usize)
                }
            }
        }
        "insert" => {
            let k = int("insert", &first()?)?;
            let v = a.get(1).cloned().ok_or("insert() needs (index, value)")?;
            let mut lst = l.borrow_mut();
            let n = lst.len() as i64;
            let k = if k < 0 { (n + k).max(0) } else { k.min(n) } as usize;
            lst.insert(k, v);
            Value::Nil
        }
        "remove" => {
            let x = first()?;
            let mut lst = l.borrow_mut();
            let pos = lst
                .iter()
                .position(|v| v.equals(&x))
                .ok_or_else(|| format!("remove(): {} not in list", x.repr()))?;
            lst.remove(pos)
        }
        "clear" => {
            l.borrow_mut().clear();
            Value::Nil
        }
        "contains" => Value::Bool(
            l.borrow()
                .iter()
                .any(|v| v.equals(&a.first().cloned().unwrap_or(Value::Nil))),
        ),
        "index" => {
            let x = first()?;
            Value::Int(
                l.borrow()
                    .iter()
                    .position(|v| v.equals(&x))
                    .map(|p| p as i64)
                    .unwrap_or(-1),
            )
        }
        "count" => {
            let x = first()?;
            Value::Int(l.borrow().iter().filter(|v| v.equals(&x)).count() as i64)
        }
        "first" => l.borrow().first().cloned().unwrap_or(Value::Nil),
        "last" => l.borrow().last().cloned().unwrap_or(Value::Nil),
        "reverse" => {
            l.borrow_mut().reverse();
            Value::Nil
        }
        "sort" => {
            let v = l.borrow().clone();
            let sorted = sort_values(i, v, a.first())?;
            *l.borrow_mut() = sorted;
            Value::Nil
        }
        "sorted" => Value::list(sort_values(i, l.borrow().clone(), a.first())?),
        "copy" => Value::list(l.borrow().clone()),
        "join" => {
            let sep = a.first().map(|v| v.to_string()).unwrap_or_default();
            Value::str(
                l.borrow()
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(&sep),
            )
        }
        "slice" => {
            let n = l.borrow().len() as i64;
            let s = a.first().map(|v| int("slice", v)).transpose()?.unwrap_or(0);
            let e = a.get(1).map(|v| int("slice", v)).transpose()?.unwrap_or(n);
            crate::interp::index(&Value::List(l.clone()), &Value::Range(s, e, false))?
        }
        "map" => b_map(i, vec![first()?, Value::List(l.clone())])?,
        "filter" => b_filter(i, vec![first()?, Value::List(l.clone())])?,
        "reduce" => {
            let mut v = vec![first()?, Value::List(l.clone())];
            v.extend(a.get(1).cloned());
            b_reduce(i, v)?
        }
        "each" | "for_each" => {
            let f = first()?;
            for v in l.borrow().clone() {
                call(i, &f, vec![v])?;
            }
            Value::Nil
        }
        "any" => {
            let f = first()?;
            for v in l.borrow().clone() {
                if call(i, &f, vec![v])?.truthy() {
                    return Ok(Value::Bool(true));
                }
            }
            Value::Bool(false)
        }
        "all" => {
            let f = first()?;
            for v in l.borrow().clone() {
                if !call(i, &f, vec![v])?.truthy() {
                    return Ok(Value::Bool(false));
                }
            }
            Value::Bool(true)
        }
        "sum" => b_sum(i, vec![Value::List(l.clone())])?,
        "min" => b_min(i, vec![Value::List(l.clone())])?,
        "max" => b_max(i, vec![Value::List(l.clone())])?,
        "enumerate" => b_enumerate(i, vec![Value::List(l.clone())])?,
        "hex" => Value::str(
            l.borrow()
                .iter()
                .map(|v| {
                    v.as_i64()
                        .map(|n| format!("{:02x}", n))
                        .unwrap_or_else(|| "??".into())
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => return Err(format!("list has no method '{name}'")),
    })
}

fn dict_method(i: I, d: &Rc<std::cell::RefCell<Dict>>, name: &str, a: Vec<Value>) -> Res {
    let first = || {
        a.first()
            .cloned()
            .ok_or_else(|| format!("{name}() needs an argument"))
    };
    Ok(match name {
        "len" => Value::Int(d.borrow().len() as i64),
        "get" => d
            .borrow()
            .get(&first()?)?
            .or_else(|| a.get(1).cloned())
            .unwrap_or(Value::Nil),
        "set" | "insert" => {
            d.borrow_mut()
                .set(first()?, a.get(1).cloned().unwrap_or(Value::Nil))?;
            Value::Nil
        }
        "has" | "contains" | "contains_key" => Value::Bool(d.borrow().get(&first()?)?.is_some()),
        "remove" | "pop" => d
            .borrow_mut()
            .remove(&first()?)?
            .or_else(|| a.get(1).cloned())
            .unwrap_or(Value::Nil),
        "keys" => Value::list(d.borrow().entries.iter().map(|(k, _)| k.clone()).collect()),
        "values" => Value::list(d.borrow().entries.iter().map(|(_, v)| v.clone()).collect()),
        "items" => Value::list(
            d.borrow()
                .entries
                .iter()
                .map(|(k, v)| Value::list(vec![k.clone(), v.clone()]))
                .collect(),
        ),
        "copy" => Value::dict(d.borrow().clone()),
        "clear" => {
            *d.borrow_mut() = Dict::default();
            Value::Nil
        }
        "update" => {
            if let Value::Dict(o) = first()? {
                let entries = o.borrow().entries.clone();
                for (k, v) in entries {
                    d.borrow_mut().set(k, v)?;
                }
            }
            Value::Nil
        }
        _ => {
            let _ = i;
            return Err(format!("dict has no method '{name}'"));
        }
    })
}
