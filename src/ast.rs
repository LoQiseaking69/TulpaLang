//! Abstract syntax tree.

use crate::units::Unit;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    /// f-string parts: literal text or embedded expression.
    FStr(Vec<FPart>),
    Bool(bool),
    Nil,
    Ident(String, usize),
    /// `Module::name`
    Path(String, String, usize),
    List(Vec<Expr>),
    /// `[value; count]`
    Repeat(Box<Expr>, Box<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Unary(String, Box<Expr>, usize),
    Binary(Box<Expr>, String, Box<Expr>, usize),
    /// Short-circuit `and` / `or`.
    Logic(Box<Expr>, bool, Box<Expr>),
    Range(Box<Expr>, Box<Expr>, bool),
    Call(Box<Expr>, Vec<Expr>, usize),
    MethodCall(Box<Expr>, String, Vec<Expr>, usize),
    Index(Box<Expr>, Box<Expr>, usize),
    Field(Box<Expr>, String, usize),
    Lambda(Rc<FnDef>),
    /// `name!(args...)` — builtin or user-defined macro.
    MacroCall(String, Vec<MacroArg>, usize),
    /// `if c { a } else { b }` used as a value.
    IfExpr(Box<Expr>, Vec<Stmt>, Option<Vec<Stmt>>),
    Match(Box<Expr>, Vec<MatchArm>, usize),
    /// `{ stmts }` block used as a value (only inside macro args).
    Block(Vec<Stmt>),
    Spawn(Box<Expr>, usize),
    Await(Box<Expr>, usize),
    /// Number with a unit: `9.81[m/s^2]`
    Quantity(f64, Rc<Unit>),
    /// Keyword argument inside a call: `f(x, speed: 3)`
    Named(String, Box<Expr>),
}

/// Runtime-checked type annotation: `x: int`, `v: [m/s]`, `name: str?`
#[derive(Debug, Clone)]
pub enum TypeAnn {
    Name(String, bool),
    Unit(Rc<Unit>, bool),
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Param>,
    pub methods: Vec<Rc<FnDef>>,
    pub invariants: Vec<Contract>,
}

#[derive(Debug, Clone)]
pub struct MachineDef {
    pub name: String,
    pub params: Vec<Param>,
    pub states: Vec<StateDef>,
    /// Transitions valid in every state (e.g. emergency stop).
    pub global: Vec<Transition>,
}

#[derive(Debug, Clone)]
pub struct StateDef {
    pub name: String,
    pub enter: Option<Vec<Stmt>>,
    pub exit: Option<Vec<Stmt>>,
    pub transitions: Vec<Transition>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Transition {
    pub event: String,
    pub params: Vec<String>,
    pub guard: Option<Expr>,
    /// None = internal transition (stay, no enter/exit).
    pub target: Option<String>,
    pub action: Option<Vec<Stmt>>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub enum FPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum MacroArg {
    Expr(Expr),
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Bind(String),
    Literal(Expr),
    Range(Expr, Expr, bool),
    List(Vec<Pattern>, Option<String>),
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub params: Vec<Param>,
    pub requires: Vec<Contract>,
    pub ensures: Vec<Contract>,
    pub body: Vec<Stmt>,
    /// Lambda written as `fn(x) => expr`.
    pub is_expr: bool,
    pub ret: Option<TypeAnn>,
}

/// A Hehner-style specification clause together with its source text.
#[derive(Debug, Clone)]
pub struct Contract {
    pub expr: Expr,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
    pub ty: Option<TypeAnn>,
}

#[derive(Debug, Clone)]
pub enum Target {
    Name(String),
    Index(Expr, Expr),
    Field(Expr, String),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `let a, b = ...` / `const A = ...` (bool = is const)
    Let(Vec<String>, Expr, bool, usize),
    Assign(Target, Option<String>, Expr, usize),
    Expr(Expr, usize),
    Fn(Rc<FnDef>),
    Return(Option<Expr>, usize),
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    While(Expr, Vec<Contract>, Vec<Stmt>, usize),
    Loop(Vec<Stmt>),
    For(Vec<String>, Expr, Vec<Stmt>, usize),
    Break,
    Continue,
    Use(Vec<String>),
    Assert(Expr, Option<Expr>, String, usize),
    Struct(Rc<StructDef>),
    Machine(Rc<MachineDef>),
    /// Edge-triggered reactive rule: `when cond { rising } else { falling }`
    /// `when cond [for duration] { rising } else { falling }`
    When(Expr, Option<Expr>, Vec<Stmt>, Option<Vec<Stmt>>, usize),
    /// Bounded response: `after trigger within duration ensure response else { handler }`
    Deadline(Expr, Expr, Contract, Option<Vec<Stmt>>, usize),
    /// Safety monitor: `always cond else { handler }`
    Always(Contract, Option<Vec<Stmt>>, usize),
    /// `import "path" as name` / `from "path" import a, b`
    Import(String, Option<String>, Vec<String>, usize),
    /// `try { ... } catch err { ... }`
    Try(Vec<Stmt>, Option<String>, Vec<Stmt>),
    /// `define_macro!(name, (params) { body })`
    MacroDef(String, Vec<String>, Vec<Stmt>),
}
