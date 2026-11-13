//! Bytecode tier.
//!
//! Function bodies are compiled on their first call into a compact stack-machine bytecode with
//! resolved local slots, then executed by a tight dispatch loop with fast paths for int/float
//! arithmetic. Anything the compiler does not handle (closures, macros, match, try, nested
//! definitions, ...) makes it decline, and that function simply stays on the tree-walking
//! interpreter: both tiers share values, calls, contracts and error reporting, so the choice is
//! invisible to programs except for speed.

use crate::ast::*;
use crate::interp::*;
use crate::value::*;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    In,
    NotIn,
}

impl BinOp {
    fn from_str(s: &str) -> Option<BinOp> {
        Some(match s {
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            "*" => BinOp::Mul,
            "/" => BinOp::Div,
            "%" => BinOp::Mod,
            "**" => BinOp::Pow,
            "==" => BinOp::Eq,
            "!=" => BinOp::Ne,
            "<" => BinOp::Lt,
            "<=" => BinOp::Le,
            ">" => BinOp::Gt,
            ">=" => BinOp::Ge,
            "&" => BinOp::And,
            "|" => BinOp::Or,
            "^" => BinOp::Xor,
            "<<" => BinOp::Shl,
            ">>" => BinOp::Shr,
            "in" => BinOp::In,
            "not in" => BinOp::NotIn,
            _ => return None,
        })
    }
    fn as_str(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::In => "in",
            BinOp::NotIn => "not in",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Const(u32),
    Nil,
    Load(u16),
    Store(u16),
    LoadG(u32),
    StoreG(u32),
    Pop,
    Bin(BinOp),
    /// Unary operator: 0 = neg, 1 = not, 2 = bitwise not
    Un(u8),
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfFalseKeep(u32),
    JumpIfTrueKeep(u32),
    Call(u8),
    CallMethod(u32, u8),
    Index,
    /// stack: obj, idx, value
    SetIndex,
    GetField(u32),
    /// stack: obj, value
    SetField(u32),
    MakeList(u32),
    MakeRange(bool),
    FStr(u32),
    /// pops the iterable, stores it as a list in slot a, sets slot b = 0
    IterInit(u16, u16),
    /// pushes seq[idx] and increments idx, or jumps when exhausted
    IterNext(u16, u16, u32),
    /// pops condition; fails with names[msg] (and an optional popped message) when false
    Assert(u32, bool),
    Return,
}

pub struct Chunk {
    pub code: Vec<Op>,
    pub consts: Vec<Value>,
    pub names: Vec<String>,
    pub lines: Vec<usize>,
    pub nlocals: usize,
}

struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

struct Compiler {
    chunk: Chunk,
    scopes: Vec<HashMap<String, u16>>,
    loops: Vec<LoopCtx>,
    line: usize,
}

type C = Result<(), ()>;

/// Compiles a function body, or returns None when it uses something only the tree-walker handles.
pub fn compile(def: &FnDef) -> Option<Chunk> {
    let mut c = Compiler {
        chunk: Chunk {
            code: vec![],
            consts: vec![],
            names: vec![],
            lines: vec![],
            nlocals: 0,
        },
        scopes: vec![HashMap::new()],
        loops: vec![],
        line: 0,
    };
    for p in &def.params {
        c.declare(&p.name).ok()?;
    }
    c.block(&def.body, true).ok()?;
    c.emit(Op::Return);
    Some(c.chunk)
}

impl Compiler {
    fn emit(&mut self, op: Op) -> usize {
        self.chunk.code.push(op);
        self.chunk.lines.push(self.line);
        self.chunk.code.len() - 1
    }
    fn here(&self) -> u32 {
        self.chunk.code.len() as u32
    }
    fn patch(&mut self, at: usize, target: u32) {
        self.chunk.code[at] = match self.chunk.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfFalseKeep(_) => Op::JumpIfFalseKeep(target),
            Op::JumpIfTrueKeep(_) => Op::JumpIfTrueKeep(target),
            Op::IterNext(a, b, _) => Op::IterNext(a, b, target),
            other => other,
        };
    }
    fn konst(&mut self, v: Value) -> u32 {
        self.chunk.consts.push(v);
        (self.chunk.consts.len() - 1) as u32
    }
    fn name(&mut self, n: &str) -> u32 {
        if let Some(i) = self.chunk.names.iter().position(|x| x == n) {
            return i as u32;
        }
        self.chunk.names.push(n.to_string());
        (self.chunk.names.len() - 1) as u32
    }
    fn declare(&mut self, n: &str) -> Result<u16, ()> {
        if self.chunk.nlocals >= u16::MAX as usize {
            return Err(());
        }
        let slot = self.chunk.nlocals as u16;
        self.chunk.nlocals += 1;
        self.scopes.last_mut().unwrap().insert(n.to_string(), slot);
        Ok(slot)
    }
    fn hidden(&mut self) -> Result<u16, ()> {
        self.declare(&format!("\0{}", self.chunk.nlocals))
    }
    fn resolve(&self, n: &str) -> Option<u16> {
        self.scopes.iter().rev().find_map(|s| s.get(n).copied())
    }

    /// Compiles a block. With `want`, leaves the block's value (its last statement's) on the stack.
    fn block(&mut self, stmts: &[Stmt], want: bool) -> C {
        self.scopes.push(HashMap::new());
        let r = self.block_in(stmts, want);
        self.scopes.pop();
        r
    }

    fn block_in(&mut self, stmts: &[Stmt], want: bool) -> C {
        if stmts.is_empty() {
            if want {
                self.emit(Op::Nil);
            }
            return Ok(());
        }
        for (i, s) in stmts.iter().enumerate() {
            let last = i + 1 == stmts.len();
            self.stmt(s, want && last)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt, want: bool) -> C {
        let nil_if = |c: &mut Self| {
            if want {
                c.emit(Op::Nil);
            }
        };
        match s {
            Stmt::Expr(e, line) => {
                self.line = *line;
                self.expr(e)?;
                if !want {
                    self.emit(Op::Pop);
                }
            }
            Stmt::Let(names, e, _is_const, line) => {
                if names.len() != 1 {
                    return Err(());
                }
                self.line = *line;
                self.expr(e)?;
                let slot = self.declare(&names[0])?;
                self.emit(Op::Store(slot));
                nil_if(self);
            }
            Stmt::Assign(target, op, e, line) => {
                self.line = *line;
                self.assign(target, op.as_deref(), e)?;
                nil_if(self);
            }
            Stmt::Return(e, line) => {
                self.line = *line;
                match e {
                    Some(e) => self.expr(e)?,
                    None => {
                        self.emit(Op::Nil);
                    }
                }
                self.emit(Op::Return);
            }
            Stmt::If(cond, a, b) => {
                self.expr(cond)?;
                let jf = self.emit(Op::JumpIfFalse(0));
                self.block(a, want)?;
                let jend = self.emit(Op::Jump(0));
                let else_at = self.here();
                self.patch(jf, else_at);
                match b {
                    Some(b) => self.block(b, want)?,
                    None => nil_if(self),
                }
                let end = self.here();
                self.patch(jend, end);
            }
            Stmt::While(cond, invariants, body, line) => {
                if !invariants.is_empty() {
                    return Err(()); // invariant checking stays on the interpreter
                }
                self.line = *line;
                let top = self.here();
                self.expr(cond)?;
                let jf = self.emit(Op::JumpIfFalse(0));
                self.loops.push(LoopCtx {
                    breaks: vec![],
                    continues: vec![],
                });
                self.block(body, false)?;
                let ctx = self.loops.pop().unwrap();
                for c in ctx.continues {
                    self.patch(c, top);
                }
                self.emit(Op::Jump(top));
                let end = self.here();
                self.patch(jf, end);
                for b in ctx.breaks {
                    self.patch(b, end);
                }
                nil_if(self);
            }
            Stmt::Loop(body) => {
                let top = self.here();
                self.loops.push(LoopCtx {
                    breaks: vec![],
                    continues: vec![],
                });
                self.block(body, false)?;
                let ctx = self.loops.pop().unwrap();
                for c in ctx.continues {
                    self.patch(c, top);
                }
                self.emit(Op::Jump(top));
                let end = self.here();
                for b in ctx.breaks {
                    self.patch(b, end);
                }
                nil_if(self);
            }
            Stmt::For(names, it, body, line) => {
                if names.len() != 1 {
                    return Err(());
                }
                self.line = *line;
                self.expr(it)?;
                let seq = self.hidden()?;
                let idx = self.hidden()?;
                self.emit(Op::IterInit(seq, idx));
                let top = self.here();
                let next = self.emit(Op::IterNext(seq, idx, 0));
                self.scopes.push(HashMap::new());
                let var = self.declare(&names[0])?;
                self.emit(Op::Store(var));
                self.loops.push(LoopCtx {
                    breaks: vec![],
                    continues: vec![],
                });
                let r = self.block(body, false);
                self.scopes.pop();
                r?;
                let ctx = self.loops.pop().unwrap();
                for c in ctx.continues {
                    self.patch(c, top);
                }
                self.emit(Op::Jump(top));
                let end = self.here();
                self.patch(next, end);
                for b in ctx.breaks {
                    self.patch(b, end);
                }
                nil_if(self);
            }
            Stmt::Break => {
                let j = self.emit(Op::Jump(0));
                self.loops.last_mut().ok_or(())?.breaks.push(j);
            }
            Stmt::Continue => {
                let j = self.emit(Op::Jump(0));
                self.loops.last_mut().ok_or(())?.continues.push(j);
            }
            Stmt::Assert(cond, msg, text, line) => {
                self.line = *line;
                if let Some(m) = msg {
                    self.expr(m)?;
                }
                self.expr(cond)?;
                let t = self.name(&format!("assertion failed: {text}"));
                self.emit(Op::Assert(t, msg.is_some()));
                nil_if(self);
            }
            Stmt::Use(_) => nil_if(self),
            _ => return Err(()),
        }
        Ok(())
    }

    fn assign(&mut self, target: &Target, op: Option<&str>, e: &Expr) -> C {
        let bin = match op {
            Some(o) => Some(BinOp::from_str(o).ok_or(())?),
            None => None,
        };
        match target {
            Target::Name(n) => {
                let slot = self.resolve(n);
                if let Some(b) = bin {
                    match slot {
                        Some(s) => self.emit(Op::Load(s)),
                        None => {
                            let k = self.name(n);
                            self.emit(Op::LoadG(k))
                        }
                    };
                    self.expr(e)?;
                    self.emit(Op::Bin(b));
                } else {
                    self.expr(e)?;
                }
                match slot {
                    Some(s) => self.emit(Op::Store(s)),
                    None => {
                        let k = self.name(n);
                        self.emit(Op::StoreG(k))
                    }
                };
            }
            Target::Index(o, i) => {
                if bin.is_some() {
                    return Err(()); // keep compound index assignment on the interpreter
                }
                self.expr(o)?;
                self.expr(i)?;
                self.expr(e)?;
                self.emit(Op::SetIndex);
            }
            Target::Field(o, f) => {
                if bin.is_some() {
                    return Err(());
                }
                self.expr(o)?;
                self.expr(e)?;
                let k = self.name(f);
                self.emit(Op::SetField(k));
            }
        }
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> C {
        match e {
            Expr::Int(n) => {
                let k = self.konst(Value::Int(*n));
                self.emit(Op::Const(k));
            }
            Expr::Float(f) => {
                let k = self.konst(Value::Float(*f));
                self.emit(Op::Const(k));
            }
            Expr::Str(s) => {
                let k = self.konst(Value::str(s.as_str()));
                self.emit(Op::Const(k));
            }
            Expr::Bool(b) => {
                let k = self.konst(Value::Bool(*b));
                self.emit(Op::Const(k));
            }
            Expr::Nil => {
                self.emit(Op::Nil);
            }
            Expr::Quantity(v, u) => {
                let k = self.konst(Value::Quantity(crate::units::Quantity::new(*v, u.clone())));
                self.emit(Op::Const(k));
            }
            Expr::Ident(n, line) => {
                self.line = (*line).max(self.line);
                match self.resolve(n) {
                    Some(s) => self.emit(Op::Load(s)),
                    None => {
                        let k = self.name(n);
                        self.emit(Op::LoadG(k))
                    }
                };
            }
            Expr::Path(m, n, _) => {
                let v = crate::builtins::path(m, n).ok_or(())?;
                let k = self.konst(v);
                self.emit(Op::Const(k));
            }
            Expr::FStr(parts) => {
                for p in parts {
                    match p {
                        FPart::Lit(l) => {
                            let k = self.konst(Value::str(l.as_str()));
                            self.emit(Op::Const(k));
                        }
                        FPart::Expr(e) => self.expr(e)?,
                    }
                }
                self.emit(Op::FStr(parts.len() as u32));
            }
            Expr::List(items) => {
                for i in items {
                    self.expr(i)?;
                }
                self.emit(Op::MakeList(items.len() as u32));
            }
            Expr::Unary(op, x, line) => {
                self.line = *line;
                self.expr(x)?;
                let code = match op.as_str() {
                    "-" => 0,
                    "!" => 1,
                    "~" => 2,
                    _ => return Err(()),
                };
                self.emit(Op::Un(code));
            }
            Expr::Binary(l, op, r, line) => {
                let b = BinOp::from_str(op).ok_or(())?;
                self.expr(l)?;
                self.expr(r)?;
                self.line = *line;
                self.emit(Op::Bin(b));
            }
            Expr::Logic(l, is_and, r) => {
                self.expr(l)?;
                let j = if *is_and {
                    self.emit(Op::JumpIfFalseKeep(0))
                } else {
                    self.emit(Op::JumpIfTrueKeep(0))
                };
                self.emit(Op::Pop);
                self.expr(r)?;
                let end = self.here();
                self.patch(j, end);
            }
            Expr::Range(a, b, incl) => {
                self.expr(a)?;
                self.expr(b)?;
                self.emit(Op::MakeRange(*incl));
            }
            Expr::Call(f, args, line) => {
                if args.len() > 255 || args.iter().any(|a| matches!(a, Expr::Named(..))) {
                    return Err(());
                }
                self.expr(f)?;
                for a in args {
                    self.expr(a)?;
                }
                self.line = *line;
                self.emit(Op::Call(args.len() as u8));
            }
            Expr::MethodCall(o, m, args, line) => {
                if args.len() > 255 || args.iter().any(|a| matches!(a, Expr::Named(..))) {
                    return Err(());
                }
                self.expr(o)?;
                for a in args {
                    self.expr(a)?;
                }
                self.line = *line;
                let k = self.name(m);
                self.emit(Op::CallMethod(k, args.len() as u8));
            }
            Expr::Index(o, i, line) => {
                self.expr(o)?;
                self.expr(i)?;
                self.line = *line;
                self.emit(Op::Index);
            }
            Expr::Field(o, f, line) => {
                self.expr(o)?;
                self.line = *line;
                let k = self.name(f);
                self.emit(Op::GetField(k));
            }
            Expr::IfExpr(c, a, b) => {
                self.expr(c)?;
                let jf = self.emit(Op::JumpIfFalse(0));
                self.block(a, true)?;
                let jend = self.emit(Op::Jump(0));
                let else_at = self.here();
                self.patch(jf, else_at);
                match b {
                    Some(b) => self.block(b, true)?,
                    None => {
                        self.emit(Op::Nil);
                    }
                }
                let end = self.here();
                self.patch(jend, end);
            }
            _ => return Err(()),
        }
        Ok(())
    }
}

#[inline]
fn fast_bin(op: BinOp, a: &Value, b: &Value) -> Option<Value> {
    use Value::{Bool, Float, Int};
    Some(match (a, b) {
        (Int(x), Int(y)) => match op {
            BinOp::Add => Int(x.checked_add(*y)?),
            BinOp::Sub => Int(x.checked_sub(*y)?),
            BinOp::Mul => Int(x.checked_mul(*y)?),
            BinOp::Lt => Bool(x < y),
            BinOp::Le => Bool(x <= y),
            BinOp::Gt => Bool(x > y),
            BinOp::Ge => Bool(x >= y),
            BinOp::Eq => Bool(x == y),
            BinOp::Ne => Bool(x != y),
            BinOp::And => Int(x & y),
            BinOp::Or => Int(x | y),
            BinOp::Xor => Int(x ^ y),
            _ => return None,
        },
        (Float(x), Float(y)) => match op {
            BinOp::Add => Float(x + y),
            BinOp::Sub => Float(x - y),
            BinOp::Mul => Float(x * y),
            BinOp::Div if *y != 0.0 => Float(x / y),
            BinOp::Lt => Bool(x < y),
            BinOp::Le => Bool(x <= y),
            BinOp::Gt => Bool(x > y),
            BinOp::Ge => Bool(x >= y),
            _ => return None,
        },
        _ => return None,
    })
}

impl Interpreter {
    /// Returns the compiled chunk for a function (compiling on first use), or None to interpret.
    pub(crate) fn chunk_for(&mut self, def: &Rc<FnDef>) -> Option<Rc<Chunk>> {
        if !self.use_vm {
            return None;
        }
        let key = Rc::as_ptr(def) as usize;
        if let Some((_, c)) = self.compiled.get(&key) {
            return c.clone();
        }
        let c = compile(def).map(Rc::new);
        self.compiled.insert(key, (def.clone(), c.clone()));
        c
    }

    /// Executes a chunk. Returns the result and the final local slots (for `x'` in ensures).
    pub(crate) fn run_chunk(
        &mut self,
        chunk: &Chunk,
        env: &Env,
        mut locals: Vec<Value>,
    ) -> R<(Value, Vec<Value>)> {
        locals.resize(chunk.nlocals, Value::Nil);
        let mut stack: Vec<Value> = Vec::with_capacity(16);
        let mut pc = 0usize;
        macro_rules! fail {
            ($e:expr) => {
                return Err(at_line($e, chunk.lines[pc]))
            };
        }
        macro_rules! pop {
            () => {
                stack.pop().unwrap()
            };
        }
        loop {
            let op = chunk.code[pc];
            match op {
                Op::Const(k) => stack.push(chunk.consts[k as usize].clone()),
                Op::Nil => stack.push(Value::Nil),
                Op::Load(s) => stack.push(locals[s as usize].clone()),
                Op::Store(s) => locals[s as usize] = pop!(),
                Op::LoadG(k) => {
                    let n = &chunk.names[k as usize];
                    match lookup(env, n) {
                        Some(Value::Thunk(t)) => match self.force_thunk(&t) {
                            Ok(v) => stack.push(v),
                            Err(e) => fail!(e),
                        },
                        Some(v) => stack.push(v),
                        None => fail!(Ctrl::from(format!("undefined variable '{n}'"))),
                    }
                }
                Op::StoreG(k) => {
                    let n = &chunk.names[k as usize];
                    let v = pop!();
                    match assign_var(env, n, v) {
                        Ok(true) => {}
                        Ok(false) => fail!(Ctrl::from(format!(
                            "assignment to undeclared variable '{n}' (use 'let {n} = ...')"
                        ))),
                        Err(m) => fail!(Ctrl::from(m)),
                    }
                    if self.has_reactive() {
                        if let Err(e) = self.react() {
                            fail!(e)
                        }
                    }
                }
                Op::Pop => {
                    stack.pop();
                }
                Op::Bin(b) => {
                    let r = pop!();
                    let l = pop!();
                    match fast_bin(b, &l, &r) {
                        Some(v) => stack.push(v),
                        None => match self.binary_op(&l, b.as_str(), &r) {
                            Ok(v) => stack.push(v),
                            Err(e) => fail!(e),
                        },
                    }
                }
                Op::Un(code) => {
                    let v = pop!();
                    let r = match (code, &v) {
                        (0, Value::Int(i)) if *i != i64::MIN => Ok(Value::Int(-i)),
                        (0, Value::Float(f)) => Ok(Value::Float(-f)),
                        (1, v) => Ok(Value::Bool(!v.truthy())),
                        (0, v) => unary("-", v),
                        (_, v) => unary("~", v),
                    };
                    match r {
                        Ok(x) => stack.push(x),
                        Err(m) => fail!(Ctrl::from(m)),
                    }
                }
                Op::Jump(t) => {
                    pc = t as usize;
                    continue;
                }
                Op::JumpIfFalse(t) => {
                    if !pop!().truthy() {
                        pc = t as usize;
                        continue;
                    }
                }
                Op::JumpIfFalseKeep(t) => {
                    if !stack.last().unwrap().truthy() {
                        pc = t as usize;
                        continue;
                    }
                }
                Op::JumpIfTrueKeep(t) => {
                    if stack.last().unwrap().truthy() {
                        pc = t as usize;
                        continue;
                    }
                }
                Op::Call(argc) => {
                    let args = stack.split_off(stack.len() - argc as usize);
                    let f = pop!();
                    match self.call(&f, args) {
                        Ok(v) => stack.push(v),
                        Err(e) => fail!(e),
                    }
                }
                Op::CallMethod(k, argc) => {
                    let args = stack.split_off(stack.len() - argc as usize);
                    let o = pop!();
                    match self.call_method(&o, &chunk.names[k as usize], args) {
                        Ok(v) => stack.push(v),
                        Err(e) => fail!(e),
                    }
                }
                Op::Index => {
                    let i = pop!();
                    let o = pop!();
                    let fast = match (&o, &i) {
                        (Value::List(l), Value::Int(k)) if *k >= 0 => {
                            l.borrow().get(*k as usize).cloned()
                        }
                        _ => None,
                    };
                    match fast {
                        Some(v) => stack.push(v),
                        None => match index(&o, &i) {
                            Ok(v) => stack.push(v),
                            Err(m) => fail!(Ctrl::from(m)),
                        },
                    }
                }
                Op::SetIndex => {
                    let v = pop!();
                    let i = pop!();
                    let o = pop!();
                    if let Err(m) = set_index(&o, &i, v) {
                        fail!(Ctrl::from(m))
                    }
                }
                Op::GetField(k) => {
                    let o = pop!();
                    match self.get_field(&o, &chunk.names[k as usize]) {
                        Ok(v) => stack.push(v),
                        Err(e) => fail!(e),
                    }
                }
                Op::SetField(k) => {
                    let v = pop!();
                    let o = pop!();
                    if let Err(e) = self.set_field(&o, &chunk.names[k as usize], v) {
                        fail!(e)
                    }
                    if self.has_reactive() {
                        if let Err(e) = self.react() {
                            fail!(e)
                        }
                    }
                }
                Op::MakeList(n) => {
                    let items = stack.split_off(stack.len() - n as usize);
                    stack.push(Value::list(items));
                }
                Op::MakeRange(incl) => {
                    let b = pop!();
                    let a = pop!();
                    match (a.as_i64(), b.as_i64()) {
                        (Some(x), Some(y)) => stack.push(Value::Range(x, y, incl)),
                        _ => fail!(Ctrl::from("range bounds must be integers")),
                    }
                }
                Op::FStr(n) => {
                    let parts = stack.split_off(stack.len() - n as usize);
                    let mut s = String::new();
                    for p in &parts {
                        match self.stringify(p) {
                            Ok(t) => s.push_str(&t),
                            Err(e) => fail!(e),
                        }
                    }
                    stack.push(Value::str(s));
                }
                Op::IterInit(seq, idx) => {
                    let it = pop!();
                    let v = match it {
                        r @ Value::Range(..) => r, // iterate ranges lazily
                        other => match self.iterate(&other, 1) {
                            Ok(items) => Value::list(items),
                            Err(e) => fail!(e),
                        },
                    };
                    locals[seq as usize] = v;
                    locals[idx as usize] = Value::Int(0);
                }
                Op::IterNext(seq, idx, exit) => {
                    let Value::Int(k) = locals[idx as usize] else {
                        unreachable!()
                    };
                    let item = match &locals[seq as usize] {
                        Value::Range(a, b, incl) => {
                            let x = a + k;
                            if (*incl && x <= *b) || (!*incl && x < *b) {
                                Some(Value::Int(x))
                            } else {
                                None
                            }
                        }
                        Value::List(l) => l.borrow().get(k as usize).cloned(),
                        _ => None,
                    };
                    match item {
                        Some(v) => {
                            locals[idx as usize] = Value::Int(k + 1);
                            stack.push(v);
                        }
                        None => {
                            pc = exit as usize;
                            continue;
                        }
                    }
                }
                Op::Assert(t, has_msg) => {
                    let ok = pop!().truthy();
                    let msg = if has_msg { Some(pop!()) } else { None };
                    if !ok {
                        let base = chunk.names[t as usize].clone();
                        let full = match msg {
                            Some(m) => format!("{base}: {m}"),
                            None => base,
                        };
                        fail!(Ctrl::from(full))
                    }
                }
                Op::Return => {
                    let v = stack.pop().unwrap_or(Value::Nil);
                    return Ok((v, locals));
                }
            }
            pc += 1;
        }
    }
}

/// Human-readable disassembly (used by `tulpa ast --bytecode` and tests).
pub fn disassemble(name: &str, c: &Chunk) -> String {
    let mut out = format!("fn {name}: {} locals, {} ops\n", c.nlocals, c.code.len());
    for (i, op) in c.code.iter().enumerate() {
        let extra = match op {
            Op::Const(k) => format!("  ; {}", c.consts[*k as usize].repr()),
            Op::LoadG(k)
            | Op::StoreG(k)
            | Op::GetField(k)
            | Op::SetField(k)
            | Op::CallMethod(k, _) => {
                format!("  ; {}", c.names[*k as usize])
            }
            _ => String::new(),
        };
        out.push_str(&format!("  {i:4}  {op:?}{extra}\n"));
    }
    out
}
