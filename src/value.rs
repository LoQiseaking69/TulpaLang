//! Runtime values.

use crate::ast::{FnDef, MachineDef, MacroArg, StructDef};
use crate::interp::Env;
use crate::units::Quantity;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

pub type NativeFn = fn(&mut crate::interp::Interpreter, Vec<Value>) -> Result<Value, String>;

/// Objects implemented in Rust (devices, PID controllers, ...).
pub trait Native {
    fn type_name(&self) -> &'static str;
    fn call_method(&mut self, method: &str, args: &[Value]) -> Result<Value, String>;
    fn describe(&self) -> String {
        format!("<{}>", self.type_name())
    }
}

#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    Dict(Rc<RefCell<Dict>>),
    Range(i64, i64, bool),
    Func(Rc<Closure>),
    Builtin(&'static str, NativeFn),
    Native(Rc<RefCell<Box<dyn Native>>>),
    Task(Rc<RefCell<Task>>),
    /// Call-by-name macro parameter (expression or block + defining scope).
    Thunk(Rc<(MacroArg, Env)>),
    /// Number with physical dimension.
    Quantity(Quantity),
    StructType(Rc<StructType>),
    Object(Rc<RefCell<Object>>),
    MachineType(Rc<MachineType>),
    Machine(Rc<RefCell<MachineInst>>),
    Signal(Rc<RefCell<Signal>>),
}

pub struct Closure {
    pub def: Rc<FnDef>,
    pub env: Env,
}

pub enum Task {
    Pending(Value, Vec<Value>),
    Running,
    Done(Value),
    Failed(String),
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Nil,
    Bool(bool),
    Int(i64),
    Str(Rc<str>),
}

#[derive(Default, Clone)]
pub struct Dict {
    index: HashMap<Key, usize>,
    pub entries: Vec<(Value, Value)>,
}

impl Dict {
    pub fn key(v: &Value) -> Result<Key, String> {
        Ok(match v {
            Value::Nil => Key::Nil,
            Value::Bool(b) => Key::Bool(*b),
            Value::Int(i) => Key::Int(*i),
            Value::Float(f) if f.fract() == 0.0 => Key::Int(*f as i64),
            Value::Str(s) => Key::Str(s.clone()),
            other => {
                return Err(format!(
                    "{} cannot be used as a dict key",
                    other.type_name()
                ))
            }
        })
    }
    pub fn get(&self, k: &Value) -> Result<Option<Value>, String> {
        let key = Self::key(k)?;
        Ok(self.index.get(&key).map(|i| self.entries[*i].1.clone()))
    }
    pub fn set(&mut self, k: Value, v: Value) -> Result<(), String> {
        let key = Self::key(&k)?;
        match self.index.get(&key) {
            Some(&i) => self.entries[i].1 = v,
            None => {
                self.index.insert(key, self.entries.len());
                self.entries.push((k, v));
            }
        }
        Ok(())
    }
    pub fn remove(&mut self, k: &Value) -> Result<Option<Value>, String> {
        let key = Self::key(k)?;
        let Some(i) = self.index.remove(&key) else {
            return Ok(None);
        };
        let (_, v) = self.entries.remove(i);
        for idx in self.index.values_mut() {
            if *idx > i {
                *idx -= 1;
            }
        }
        Ok(Some(v))
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Value {
    pub fn str(s: impl Into<Rc<str>>) -> Value {
        Value::Str(s.into())
    }
    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(v)))
    }
    pub fn dict(d: Dict) -> Value {
        Value::Dict(Rc::new(RefCell::new(d)))
    }
    pub fn native(n: impl Native + 'static) -> Value {
        Value::Native(Rc::new(RefCell::new(Box::new(n))))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::List(_) => "list",
            Value::Dict(_) => "dict",
            Value::Range(..) => "range",
            Value::Func(_) | Value::Builtin(..) => "fn",
            Value::Native(n) => n.borrow().type_name(),
            Value::Task(_) => "task",
            Value::Thunk(_) => "thunk",
            Value::Quantity(_) => "quantity",
            Value::StructType(_) => "struct",
            Value::Object(_) => "object",
            Value::MachineType(_) => "machine",
            Value::Machine(_) => "machine instance",
            Value::Signal(_) => "signal",
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Nil => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Dict(d) => d.borrow().len() > 0,
            Value::Range(a, b, incl) => {
                if *incl {
                    a <= b
                } else {
                    a < b
                }
            }
            _ => true,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Bool(b) => Some(*b as i64 as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) if f.fract() == 0.0 => Some(*f as i64),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    /// Converts a list (or string) into raw bytes, for device I/O.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        match self {
            Value::Str(s) => Ok(s.as_bytes().to_vec()),
            Value::Int(i) => byte(*i).map(|b| vec![b]),
            Value::List(l) => l
                .borrow()
                .iter()
                .map(|v| {
                    v.as_i64()
                        .ok_or_else(|| format!("expected byte, got {}", v.type_name()))
                        .and_then(byte)
                })
                .collect(),
            other => Err(format!(
                "expected bytes (list or str), got {}",
                other.type_name()
            )),
        }
    }

    pub fn from_bytes(b: &[u8]) -> Value {
        Value::list(b.iter().map(|x| Value::Int(*x as i64)).collect())
    }

    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => format!("{:?}", s.as_ref()),
            other => other.to_string(),
        }
    }

    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (a @ (Value::Int(_) | Value::Float(_)), b @ (Value::Int(_) | Value::Float(_))) => {
                a.as_f64() == b.as_f64()
            }
            (Value::List(a), Value::List(b)) => {
                Rc::ptr_eq(a, b) || {
                    let (a, b) = (a.borrow(), b.borrow());
                    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.equals(y))
                }
            }
            (Value::Dict(a), Value::Dict(b)) => {
                Rc::ptr_eq(a, b) || {
                    let (a, b) = (a.borrow(), b.borrow());
                    a.len() == b.len()
                        && a.entries
                            .iter()
                            .all(|(k, v)| matches!(b.get(k), Ok(Some(ref w)) if w.equals(v)))
                }
            }
            (Value::Range(a, b, c), Value::Range(x, y, z)) => a == x && b == y && c == z,
            (Value::Func(a), Value::Func(b)) => Rc::ptr_eq(a, b),
            (Value::Native(a), Value::Native(b)) => Rc::ptr_eq(a, b),
            (Value::Task(a), Value::Task(b)) => Rc::ptr_eq(a, b),
            (Value::Quantity(a), Value::Quantity(b)) => {
                a.dim == b.dim
                    && (a.si - b.si).abs() <= 1e-12 * a.si.abs().max(b.si.abs()).max(1e-300)
            }
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Machine(a), Value::Machine(b)) => Rc::ptr_eq(a, b),
            (Value::Signal(a), Value::Signal(b)) => Rc::ptr_eq(a, b),
            (Value::StructType(a), Value::StructType(b)) => Rc::ptr_eq(a, b),
            (Value::MachineType(a), Value::MachineType(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

pub fn byte(i: i64) -> Result<u8, String> {
    u8::try_from(i).map_err(|_| format!("byte value {i} out of range 0..255"))
}

pub fn fmt_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{}", fmt_float(*x)),
            Value::Str(s) => write!(f, "{s}"),
            Value::List(l) => {
                let items: Vec<String> = l.borrow().iter().map(|v| v.repr()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            Value::Dict(d) => {
                let items: Vec<String> = d
                    .borrow()
                    .entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                    .collect();
                write!(f, "{{{}}}", items.join(", "))
            }
            Value::Range(a, b, incl) => write!(f, "{a}{}{b}", if *incl { "..=" } else { ".." }),
            Value::Func(c) => write!(f, "<fn {}>", c.def.name),
            Value::Builtin(n, _) => write!(f, "<builtin {n}>"),
            Value::Native(n) => write!(f, "{}", n.borrow().describe()),
            Value::Task(t) => match &*t.borrow() {
                Task::Done(_) => write!(f, "<task done>"),
                Task::Failed(_) => write!(f, "<task failed>"),
                _ => write!(f, "<task pending>"),
            },
            Value::Thunk(_) => write!(f, "<thunk>"),
            Value::Quantity(q) => write!(f, "{q}"),
            Value::StructType(t) => write!(f, "<struct {}>", t.def.name),
            Value::Object(o) => {
                let o = o.borrow();
                let fields: Vec<String> = o
                    .fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.repr()))
                    .collect();
                write!(f, "{}({})", o.ty.def.name, fields.join(", "))
            }
            Value::MachineType(t) => write!(f, "<machine {}>", t.def.name),
            Value::Machine(m) => {
                let m = m.borrow();
                write!(
                    f,
                    "<{} in {}>",
                    m.ty.def.name, m.ty.def.states[m.state].name
                )
            }
            Value::Signal(sig) => write!(f, "signal({})", sig.borrow().value.repr()),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.repr())
    }
}

pub struct StructType {
    pub def: Rc<StructDef>,
    pub env: Env,
}

pub struct Object {
    pub ty: Rc<StructType>,
    pub fields: Vec<(String, Value)>,
}

impl Object {
    pub fn get(&self, k: &str) -> Option<Value> {
        self.fields
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
    }
    pub fn set(&mut self, k: &str, v: Value) -> bool {
        match self.fields.iter_mut().find(|(n, _)| n == k) {
            Some(slot) => {
                slot.1 = v;
                true
            }
            None => false,
        }
    }
}

pub struct MachineType {
    pub def: Rc<MachineDef>,
    pub env: Env,
}

pub struct MachineInst {
    pub ty: Rc<MachineType>,
    pub env: Env,
    pub state: usize,
    pub history: Vec<String>,
}

pub struct Signal {
    pub value: Value,
    pub prev: Value,
    pub changed_at: f64,
    pub prev_at: f64,
    pub history: std::collections::VecDeque<Value>,
    pub capacity: usize,
    pub derive: Option<Value>,
    pub subscribers: Vec<Value>,
}
