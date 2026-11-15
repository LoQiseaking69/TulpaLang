//! Host modules: native `Module::name` functions registered by an embedding application.
//!
//! An app that embeds Tulpa (the Android IDE, a test rig, a robot runtime) can add its own
//! module without forking the standard library:
//!
//! ```
//! use tulpa::{host, Value};
//! host::register("rig", "volts", |_, _| Ok(Value::Float(12.1)));
//! assert_eq!(tulpa::run_captured("print(rig::volts())").unwrap().trim(), "12.1");
//! ```
//!
//! Registrations are process-wide and may be added from any thread before or between runs.

use crate::value::{NativeFn, Value};
use std::sync::RwLock;

struct Entry {
    module: &'static str,
    name: &'static str,
    f: NativeFn,
}

static REGISTRY: RwLock<Vec<Entry>> = RwLock::new(Vec::new());

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Registers (or replaces) `module::name`.
pub fn register(module: &str, name: &str, f: NativeFn) {
    let mut reg = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = reg
        .iter_mut()
        .find(|e| e.module == module && e.name == name)
    {
        e.f = f;
        return;
    }
    reg.push(Entry {
        module: leak(module),
        name: leak(name),
        f,
    });
}

/// Resolves `module::name` among host registrations.
pub fn lookup(module: &str, name: &str) -> Option<Value> {
    let reg = REGISTRY.read().unwrap_or_else(|e| e.into_inner());
    reg.iter()
        .find(|e| e.module == module && e.name == name)
        .map(|e| Value::Builtin(e.name, e.f))
}

pub fn has_module(module: &str) -> bool {
    let reg = REGISTRY.read().unwrap_or_else(|e| e.into_inner());
    reg.iter().any(|e| e.module == module)
}

/// Registered module names (deduplicated), for editor completion and diagnostics.
pub fn modules() -> Vec<&'static str> {
    let reg = REGISTRY.read().unwrap_or_else(|e| e.into_inner());
    let mut m: Vec<&'static str> = reg.iter().map(|e| e.module).collect();
    m.sort_unstable();
    m.dedup();
    m
}

/// Functions registered under `module`.
pub fn names(module: &str) -> Vec<&'static str> {
    let reg = REGISTRY.read().unwrap_or_else(|e| e.into_inner());
    reg.iter()
        .filter(|e| e.module == module)
        .map(|e| e.name)
        .collect()
}
