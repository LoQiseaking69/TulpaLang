//! Tulpa — a Python-flavoured, Rust-powered language for device integration and robotics.
//!
//! ```
//! let (mut interp, out) = tulpa::Interpreter::captured();
//! let prog = tulpa::parse("let x = 6 * 7\nprint(f\"answer = {x}\")").unwrap();
//! interp.run(&prog).unwrap();
//! assert_eq!(out.borrow().as_str(), "answer = 42\n");
//! ```

pub mod ast;
pub mod builtins;
pub mod devices;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod value;

pub use devices::Backend;
pub use interp::{Interpreter, RuntimeError};
pub use parser::{parse, ParseError};
pub use value::Value;

/// Parses and runs a source string, returning captured output (simulator backend).
///
/// Runs on a dedicated thread with a large stack, because the evaluator recurses on the
/// native stack (deep Tulpa recursion needs more than the default 2 MiB).
pub fn run_captured(src: &str) -> Result<String, String> {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || run_captured_here(&src))
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "interpreter thread panicked".to_string())?
}

fn run_captured_here(src: &str) -> Result<String, String> {
    let prog = parse(src).map_err(|e| format!("parse error at {}:{}: {}", e.line, e.col, e.msg))?;
    let (mut i, out) = Interpreter::captured();
    i.run(&prog).map_err(|e| e.to_string())?;
    let s = out.borrow().clone();
    Ok(s)
}
