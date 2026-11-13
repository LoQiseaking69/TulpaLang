//! `nn::` — tensors with reverse-mode automatic differentiation, layers and optimisers.
//!
//! Tensors are dense, row-major, 2-D `f64` matrices (a vector is `1×n`, a scalar `1×1`).
//! Every operation on a tensor that requires gradients records itself on an implicit tape
//! (parent links), so `loss.backward()` fills `.grad()` on every parameter that fed into it.
//!
//! ```tulpa
//! let net = nn::MLP([4, 32, 2], "tanh")
//! let opt = nn::Adam(net.params(), 0.01)
//! let x = nn::tensor([[0.1, 0.2, 0.3, 0.4]])
//! let loss = (net.forward(x) - nn::tensor([[1.0, 0.0]])).square().mean()
//! opt.zero_grad(); loss.backward(); opt.step()
//! ```

use crate::value::{Native, NativeFn, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

type Res = Result<Value, String>;

// ---------------------------------------------------------------------------- RNG

thread_local! {
    static RNG: Cell<u64> = const { Cell::new(0x9E37_79B9_7F4A_7C15) };
}

fn next_u64() -> u64 {
    RNG.with(|r| {
        // xorshift64*
        let mut x = r.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        r.set(x);
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    })
}

/// Uniform in [0, 1).
pub fn uniform() -> f64 {
    (next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// Standard normal via Box–Muller.
pub fn gauss() -> f64 {
    let u1 = uniform().max(1e-300);
    let u2 = uniform();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

fn seed(s: u64) {
    RNG.with(|r| r.set(if s == 0 { 0x9E37_79B9_7F4A_7C15 } else { s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1 }));
    for _ in 0..8 {
        next_u64();
    }
}

// ---------------------------------------------------------------------------- graph

#[derive(Clone)]
enum Op {
    Leaf,
    MatMul,
    Add,
    Sub,
    Mul,
    Div,
    #[allow(dead_code)]
    AddS(f64),
    MulS(f64),
    PowS(f64),
    Neg,
    Tanh,
    Relu,
    Sigmoid,
    Exp,
    Log,
    Sqrt,
    Abs,
    Softplus,
    Clamp(f64, f64),
    Sum,
    SumDim(usize),
    Transpose,
    SliceCols(usize),
    SliceRows(usize),
    Cat(usize),
    Mask(Rc<Vec<f64>>),
}

pub struct Node {
    rows: usize,
    cols: usize,
    data: RefCell<Vec<f64>>,
    grad: RefCell<Vec<f64>>,
    op: Op,
    parents: Vec<Rc<Node>>,
    requires_grad: bool,
}

pub type T = Rc<Node>;

impl Node {
    fn leaf(rows: usize, cols: usize, data: Vec<f64>, requires_grad: bool) -> T {
        debug_assert_eq!(rows * cols, data.len());
        Rc::new(Node {
            rows,
            cols,
            data: RefCell::new(data),
            grad: RefCell::new(Vec::new()),
            op: Op::Leaf,
            parents: Vec::new(),
            requires_grad,
        })
    }

    fn derived(rows: usize, cols: usize, data: Vec<f64>, op: Op, parents: Vec<T>) -> T {
        let requires_grad = parents.iter().any(|p| p.requires_grad);
        Rc::new(Node {
            rows,
            cols,
            data: RefCell::new(data),
            grad: RefCell::new(Vec::new()),
            op: if requires_grad { op } else { Op::Leaf },
            parents: if requires_grad { parents } else { Vec::new() },
            requires_grad,
        })
    }

    fn len(&self) -> usize {
        self.rows * self.cols
    }

    fn add_grad(&self, g: &[f64]) {
        let mut mine = self.grad.borrow_mut();
        if mine.is_empty() {
            *mine = g.to_vec();
        } else {
            for (m, x) in mine.iter_mut().zip(g) {
                *m += x;
            }
        }
    }
}

fn shape_err(op: &str, a: &T, b: &T) -> String {
    format!(
        "nn: shape mismatch in {op}: {}×{} vs {}×{}",
        a.rows, a.cols, b.rows, b.cols
    )
}

/// Broadcast shape for element-wise ops (each dim equal, or one of them 1).
fn bshape(op: &str, a: &T, b: &T) -> Result<(usize, usize), String> {
    let dim = |x: usize, y: usize| {
        if x == y {
            Some(x)
        } else if x == 1 {
            Some(y)
        } else if y == 1 {
            Some(x)
        } else {
            None
        }
    };
    match (dim(a.rows, b.rows), dim(a.cols, b.cols)) {
        (Some(r), Some(c)) => Ok((r, c)),
        _ => Err(shape_err(op, a, b)),
    }
}

#[inline]
fn bidx(t: &Node, i: usize, j: usize) -> usize {
    (if t.rows == 1 { 0 } else { i }) * t.cols + if t.cols == 1 { 0 } else { j }
}

fn elementwise(op: Op, a: &T, b: &T, f: impl Fn(f64, f64) -> f64, name: &str) -> Result<T, String> {
    let (r, c) = bshape(name, a, b)?;
    let (ad, bd) = (a.data.borrow(), b.data.borrow());
    let mut out = Vec::with_capacity(r * c);
    for i in 0..r {
        for j in 0..c {
            out.push(f(ad[bidx(a, i, j)], bd[bidx(b, i, j)]));
        }
    }
    drop((ad, bd));
    Ok(Node::derived(r, c, out, op, vec![a.clone(), b.clone()]))
}

fn unary(op: Op, a: &T, f: impl Fn(f64) -> f64) -> T {
    let out: Vec<f64> = a.data.borrow().iter().map(|&x| f(x)).collect();
    Node::derived(a.rows, a.cols, out, op, vec![a.clone()])
}

pub fn matmul(a: &T, b: &T) -> Result<T, String> {
    if a.cols != b.rows {
        return Err(shape_err("matmul", a, b));
    }
    let (n, k, m) = (a.rows, a.cols, b.cols);
    let (ad, bd) = (a.data.borrow(), b.data.borrow());
    let mut out = vec![0.0; n * m];
    for i in 0..n {
        for p in 0..k {
            let av = ad[i * k + p];
            if av == 0.0 {
                continue;
            }
            let brow = &bd[p * m..(p + 1) * m];
            let orow = &mut out[i * m..(i + 1) * m];
            for (o, bv) in orow.iter_mut().zip(brow) {
                *o += av * bv;
            }
        }
    }
    drop((ad, bd));
    Ok(Node::derived(n, m, out, Op::MatMul, vec![a.clone(), b.clone()]))
}

fn sum_dim(a: &T, dim: usize) -> Result<T, String> {
    let d = a.data.borrow();
    let (r, c) = (a.rows, a.cols);
    let out = match dim {
        0 => {
            let mut o = vec![0.0; c];
            for i in 0..r {
                for j in 0..c {
                    o[j] += d[i * c + j];
                }
            }
            (1, c, o)
        }
        1 => {
            let mut o = vec![0.0; r];
            for i in 0..r {
                o[i] = d[i * c..(i + 1) * c].iter().sum();
            }
            (r, 1, o)
        }
        _ => return Err("nn: sum/mean dim must be 0 (over rows) or 1 (over columns)".into()),
    };
    drop(d);
    Ok(Node::derived(out.0, out.1, out.2, Op::SumDim(dim), vec![a.clone()]))
}

fn scale(a: &T, k: f64) -> T {
    unary(Op::MulS(k), a, |x| x * k)
}

fn slice_cols(a: &T, s: usize, e: usize) -> Result<T, String> {
    if s > e || e > a.cols {
        return Err(format!("nn: column slice {s}..{e} out of range for {} columns", a.cols));
    }
    let d = a.data.borrow();
    let mut out = Vec::with_capacity(a.rows * (e - s));
    for i in 0..a.rows {
        out.extend_from_slice(&d[i * a.cols + s..i * a.cols + e]);
    }
    drop(d);
    Ok(Node::derived(a.rows, e - s, out, Op::SliceCols(s), vec![a.clone()]))
}

fn slice_rows(a: &T, s: usize, e: usize) -> Result<T, String> {
    if s > e || e > a.rows {
        return Err(format!("nn: row slice {s}..{e} out of range for {} rows", a.rows));
    }
    let out = a.data.borrow()[s * a.cols..e * a.cols].to_vec();
    Ok(Node::derived(e - s, a.cols, out, Op::SliceRows(s), vec![a.clone()]))
}

fn cat(ts: &[T], dim: usize) -> Result<T, String> {
    if ts.is_empty() {
        return Err("nn::cat: empty list".into());
    }
    match dim {
        1 => {
            let r = ts[0].rows;
            if ts.iter().any(|t| t.rows != r) {
                return Err("nn::cat(dim=1): all tensors need the same number of rows".into());
            }
            let c: usize = ts.iter().map(|t| t.cols).sum();
            let mut out = Vec::with_capacity(r * c);
            for i in 0..r {
                for t in ts {
                    let d = t.data.borrow();
                    out.extend_from_slice(&d[i * t.cols..(i + 1) * t.cols]);
                }
            }
            Ok(Node::derived(r, c, out, Op::Cat(1), ts.to_vec()))
        }
        0 => {
            let c = ts[0].cols;
            if ts.iter().any(|t| t.cols != c) {
                return Err("nn::cat(dim=0): all tensors need the same number of columns".into());
            }
            let r: usize = ts.iter().map(|t| t.rows).sum();
            let mut out = Vec::with_capacity(r * c);
            for t in ts {
                out.extend_from_slice(&t.data.borrow());
            }
            Ok(Node::derived(r, c, out, Op::Cat(0), ts.to_vec()))
        }
        _ => Err("nn::cat: dim must be 0 or 1".into()),
    }
}

/// Reverse-mode sweep from `root` (seed gradient = ones).
pub fn backward(root: &T) -> Result<(), String> {
    if !root.requires_grad {
        return Err("nn: backward() on a tensor that does not depend on any parameter".into());
    }
    // Iterative post-order DFS for a topological order.
    let mut order: Vec<T> = Vec::new();
    let mut seen: HashSet<*const Node> = HashSet::new();
    let mut stack: Vec<(T, bool)> = vec![(root.clone(), false)];
    while let Some((n, expanded)) = stack.pop() {
        if expanded {
            order.push(n);
            continue;
        }
        if !seen.insert(Rc::as_ptr(&n)) {
            continue;
        }
        stack.push((n.clone(), true));
        for p in &n.parents {
            if p.requires_grad && !seen.contains(&Rc::as_ptr(p)) {
                stack.push((p.clone(), false));
            }
        }
    }
    // Intermediate gradients are per-sweep; leaf (parameter) gradients accumulate.
    for n in &order {
        if !matches!(n.op, Op::Leaf) {
            n.grad.borrow_mut().clear();
        }
    }
    root.add_grad(&vec![1.0; root.len()]);
    for n in order.iter().rev() {
        if matches!(n.op, Op::Leaf) {
            continue;
        }
        let g = n.grad.borrow().clone();
        if g.is_empty() {
            continue;
        }
        propagate(n, &g)?;
    }
    Ok(())
}

fn propagate(n: &Node, g: &[f64]) -> Result<(), String> {
    let p = &n.parents;
    let out = n.data.borrow();
    let (r, c) = (n.rows, n.cols);
    // Accumulate a broadcast gradient back into parent `t`.
    let reduce_into = |t: &T, f: &dyn Fn(usize, usize, usize) -> f64| {
        if !t.requires_grad {
            return;
        }
        let mut acc = vec![0.0; t.len()];
        for i in 0..r {
            for j in 0..c {
                let k = i * c + j;
                acc[bidx(t, i, j)] += f(i, j, k);
            }
        }
        t.add_grad(&acc);
    };
    match &n.op {
        Op::Leaf => {}
        Op::MatMul => {
            let (a, b) = (&p[0], &p[1]);
            let (ad, bd) = (a.data.borrow(), b.data.borrow());
            let (nn_, k, m) = (a.rows, a.cols, b.cols);
            if a.requires_grad {
                // dA = G · Bᵀ
                let mut ga = vec![0.0; nn_ * k];
                for i in 0..nn_ {
                    for q in 0..k {
                        let mut s = 0.0;
                        for j in 0..m {
                            s += g[i * m + j] * bd[q * m + j];
                        }
                        ga[i * k + q] = s;
                    }
                }
                a.add_grad(&ga);
            }
            if b.requires_grad {
                // dB = Aᵀ · G
                let mut gb = vec![0.0; k * m];
                for i in 0..nn_ {
                    for q in 0..k {
                        let av = ad[i * k + q];
                        if av == 0.0 {
                            continue;
                        }
                        for j in 0..m {
                            gb[q * m + j] += av * g[i * m + j];
                        }
                    }
                }
                b.add_grad(&gb);
            }
        }
        Op::Add => {
            reduce_into(&p[0], &|_, _, k| g[k]);
            reduce_into(&p[1], &|_, _, k| g[k]);
        }
        Op::Sub => {
            reduce_into(&p[0], &|_, _, k| g[k]);
            reduce_into(&p[1], &|_, _, k| -g[k]);
        }
        Op::Mul => {
            let (ad, bd) = (p[0].data.borrow(), p[1].data.borrow());
            reduce_into(&p[0], &|i, j, k| g[k] * bd[bidx(&p[1], i, j)]);
            reduce_into(&p[1], &|i, j, k| g[k] * ad[bidx(&p[0], i, j)]);
        }
        Op::Div => {
            let (ad, bd) = (p[0].data.borrow(), p[1].data.borrow());
            reduce_into(&p[0], &|i, j, k| g[k] / bd[bidx(&p[1], i, j)]);
            reduce_into(&p[1], &|i, j, k| {
                let bv = bd[bidx(&p[1], i, j)];
                -g[k] * ad[bidx(&p[0], i, j)] / (bv * bv)
            });
        }
        Op::AddS(_) => p[0].add_grad(g),
        Op::MulS(s) => p[0].add_grad(&g.iter().map(|x| x * s).collect::<Vec<_>>()),
        Op::Neg => p[0].add_grad(&g.iter().map(|x| -x).collect::<Vec<_>>()),
        Op::PowS(e) => {
            let x = p[0].data.borrow();
            let gr: Vec<f64> = g.iter().zip(x.iter()).map(|(g, x)| g * e * x.powf(e - 1.0)).collect();
            p[0].add_grad(&gr);
        }
        Op::Tanh => p[0].add_grad(&g.iter().zip(out.iter()).map(|(g, y)| g * (1.0 - y * y)).collect::<Vec<_>>()),
        Op::Sigmoid => p[0].add_grad(&g.iter().zip(out.iter()).map(|(g, y)| g * y * (1.0 - y)).collect::<Vec<_>>()),
        Op::Exp => p[0].add_grad(&g.iter().zip(out.iter()).map(|(g, y)| g * y).collect::<Vec<_>>()),
        Op::Sqrt => p[0].add_grad(&g.iter().zip(out.iter()).map(|(g, y)| g * 0.5 / y.max(1e-300)).collect::<Vec<_>>()),
        Op::Relu => {
            let x = p[0].data.borrow();
            p[0].add_grad(&g.iter().zip(x.iter()).map(|(g, x)| if *x > 0.0 { *g } else { 0.0 }).collect::<Vec<_>>());
        }
        Op::Log => {
            let x = p[0].data.borrow();
            p[0].add_grad(&g.iter().zip(x.iter()).map(|(g, x)| g / x).collect::<Vec<_>>());
        }
        Op::Abs => {
            let x = p[0].data.borrow();
            p[0].add_grad(&g.iter().zip(x.iter()).map(|(g, x)| g * x.signum()).collect::<Vec<_>>());
        }
        Op::Softplus => {
            let x = p[0].data.borrow();
            p[0].add_grad(&g.iter().zip(x.iter()).map(|(g, x)| g / (1.0 + (-x).exp())).collect::<Vec<_>>());
        }
        Op::Clamp(lo, hi) => {
            let x = p[0].data.borrow();
            p[0].add_grad(
                &g.iter()
                    .zip(x.iter())
                    .map(|(g, x)| if x >= lo && x <= hi { *g } else { 0.0 })
                    .collect::<Vec<_>>(),
            );
        }
        Op::Sum => p[0].add_grad(&vec![g[0]; p[0].len()]),
        Op::SumDim(dim) => {
            let t = &p[0];
            let mut gr = vec![0.0; t.len()];
            for i in 0..t.rows {
                for j in 0..t.cols {
                    gr[i * t.cols + j] = if *dim == 0 { g[j] } else { g[i] };
                }
            }
            t.add_grad(&gr);
        }
        Op::Transpose => {
            let t = &p[0];
            let mut gr = vec![0.0; t.len()];
            for i in 0..t.rows {
                for j in 0..t.cols {
                    gr[i * t.cols + j] = g[j * t.rows + i];
                }
            }
            t.add_grad(&gr);
        }
        Op::SliceCols(s) => {
            let t = &p[0];
            let mut gr = vec![0.0; t.len()];
            for i in 0..r {
                for j in 0..c {
                    gr[i * t.cols + s + j] = g[i * c + j];
                }
            }
            t.add_grad(&gr);
        }
        Op::SliceRows(s) => {
            let t = &p[0];
            let mut gr = vec![0.0; t.len()];
            gr[s * t.cols..s * t.cols + g.len()].copy_from_slice(g);
            t.add_grad(&gr);
        }
        Op::Cat(dim) => {
            if *dim == 1 {
                let mut off = 0;
                for t in p {
                    if t.requires_grad {
                        let mut gr = vec![0.0; t.len()];
                        for i in 0..r {
                            gr[i * t.cols..(i + 1) * t.cols]
                                .copy_from_slice(&g[i * c + off..i * c + off + t.cols]);
                        }
                        t.add_grad(&gr);
                    }
                    off += t.cols;
                }
            } else {
                let mut off = 0;
                for t in p {
                    if t.requires_grad {
                        t.add_grad(&g[off..off + t.len()]);
                    }
                    off += t.len();
                }
            }
        }
        Op::Mask(m) => p[0].add_grad(&g.iter().zip(m.iter()).map(|(g, m)| g * m).collect::<Vec<_>>()),
    }
    Ok(())
}

// ---------------------------------------------------------------------------- conversions

fn floats_of(v: &Value, what: &str) -> Result<Vec<f64>, String> {
    match v {
        Value::List(l) => l
            .borrow()
            .iter()
            .map(|x| scalar(x).ok_or_else(|| format!("{what}: expected numbers, got {}", x.type_name())))
            .collect(),
        Value::Native(_) => Ok(tensor_of(v, None).ok_or_else(|| format!("{what}: expected a list or tensor"))?.data.borrow().clone()),
        _ => Err(format!("{what}: expected a list, got {}", v.type_name())),
    }
}

fn scalar(v: &Value) -> Option<f64> {
    match v {
        Value::Quantity(q) => Some(q.si),
        other => other.as_f64(),
    }
}

/// Builds a tensor from a number, a flat list (→ 1×n) or a list of rows.
pub fn from_value(v: &Value) -> Result<T, String> {
    if let Some(x) = scalar(v) {
        return Ok(Node::leaf(1, 1, vec![x], false));
    }
    if let Some(t) = tensor_of(v, None) {
        return Ok(t);
    }
    let Value::List(l) = v else {
        return Err(format!("nn::tensor: expected a number, list or tensor, got {}", v.type_name()));
    };
    let l = l.borrow();
    if l.is_empty() {
        return Ok(Node::leaf(1, 0, vec![], false));
    }
    if matches!(l[0], Value::List(_)) {
        let rows: Vec<Vec<f64>> = l.iter().map(|r| floats_of(r, "nn::tensor")).collect::<Result<_, _>>()?;
        let c = rows[0].len();
        if rows.iter().any(|r| r.len() != c) {
            return Err("nn::tensor: rows have different lengths".into());
        }
        Ok(Node::leaf(rows.len(), c, rows.concat(), false))
    } else {
        let d = l.iter().map(|x| scalar(x).ok_or("nn::tensor: expected numbers")).collect::<Result<Vec<_>, _>>()?;
        Ok(Node::leaf(1, d.len(), d, false))
    }
}

/// Extracts the tensor node from a value. `me` resolves the self-aliasing case where the
/// argument is the very object whose method is running (its RefCell is already borrowed).
fn tensor_of(v: &Value, me: Option<&T>) -> Option<T> {
    let Value::Native(rc) = v else { return None };
    match rc.try_borrow() {
        Ok(b) => b.as_any().and_then(|a| a.downcast_ref::<Tensor>()).map(|t| t.0.clone()),
        Err(_) => me.cloned(),
    }
}

fn operand(v: &Value, me: Option<&T>) -> Result<T, String> {
    if let Some(t) = tensor_of(v, me) {
        return Ok(t);
    }
    if let Some(x) = scalar(v) {
        return Ok(Node::leaf(1, 1, vec![x], false));
    }
    if matches!(v, Value::List(_)) {
        return from_value(v);
    }
    Err(format!("nn: expected a tensor or number, got {}", v.type_name()))
}

pub fn wrap(t: T) -> Value {
    Value::native(Tensor(t))
}

fn to_list(t: &Node) -> Value {
    let d = t.data.borrow();
    Value::list(
        (0..t.rows)
            .map(|i| Value::list(d[i * t.cols..(i + 1) * t.cols].iter().map(|x| Value::Float(*x)).collect()))
            .collect(),
    )
}

fn flat_list(t: &Node) -> Value {
    Value::list(t.data.borrow().iter().map(|x| Value::Float(*x)).collect())
}

fn uarg(a: &[Value], i: usize, what: &str) -> Result<usize, String> {
    a.get(i)
        .and_then(|v| v.as_i64())
        .filter(|n| *n >= 0)
        .map(|n| n as usize)
        .ok_or_else(|| format!("{what}: argument {} must be a non-negative integer", i + 1))
}

fn farg(a: &[Value], i: usize, what: &str) -> Result<f64, String> {
    a.get(i)
        .and_then(scalar)
        .ok_or_else(|| format!("{what}: argument {} must be a number", i + 1))
}

// ---------------------------------------------------------------------------- Tensor

pub struct Tensor(pub T);

impl Native for Tensor {
    fn type_name(&self) -> &'static str {
        "Tensor"
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn describe(&self) -> String {
        let t = &self.0;
        let d = t.data.borrow();
        let grad = if t.requires_grad { ", grad" } else { "" };
        if t.len() <= 8 {
            let body: Vec<String> = (0..t.rows)
                .map(|i| {
                    let r: Vec<String> = d[i * t.cols..(i + 1) * t.cols].iter().map(|x| format!("{x:.4}")).collect();
                    format!("[{}]", r.join(", "))
                })
                .collect();
            format!("tensor({}{grad})", body.join(", "))
        } else {
            format!("<Tensor {}×{}{grad}>", t.rows, t.cols)
        }
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        let me = &self.0;
        let bin = |a: &[Value], op: &str| -> Result<T, String> {
            let b = operand(a.first().ok_or_else(|| format!("Tensor.{op}() needs an argument"))?, Some(me))?;
            Ok(b)
        };
        let t = match m {
            "shape" => return Ok(Value::list(vec![Value::Int(me.rows as i64), Value::Int(me.cols as i64)])),
            "rows" => return Ok(Value::Int(me.rows as i64)),
            "cols" => return Ok(Value::Int(me.cols as i64)),
            "numel" | "len" => return Ok(Value::Int(me.len() as i64)),
            "requires_grad" => return Ok(Value::Bool(me.requires_grad)),
            "item" => {
                if me.len() != 1 {
                    return Err(format!("Tensor.item(): tensor is {}×{}, not 1×1", me.rows, me.cols));
                }
                return Ok(Value::Float(me.data.borrow()[0]));
            }
            "tolist" => return Ok(to_list(me)),
            "flat" | "flatten" => return Ok(flat_list(me)),
            "row" => {
                let i = uarg(a, 0, "Tensor.row")?;
                if i >= me.rows {
                    return Err(format!("Tensor.row({i}): only {} rows", me.rows));
                }
                let d = me.data.borrow();
                return Ok(Value::list(d[i * me.cols..(i + 1) * me.cols].iter().map(|x| Value::Float(*x)).collect()));
            }
            "get" => {
                let (i, j) = (uarg(a, 0, "Tensor.get")?, uarg(a, 1, "Tensor.get")?);
                if i >= me.rows || j >= me.cols {
                    return Err(format!("Tensor.get({i}, {j}) out of range for {}×{}", me.rows, me.cols));
                }
                return Ok(Value::Float(me.data.borrow()[i * me.cols + j]));
            }
            "set" => {
                let (i, j, v) = (uarg(a, 0, "Tensor.set")?, uarg(a, 1, "Tensor.set")?, farg(a, 2, "Tensor.set")?);
                if i >= me.rows || j >= me.cols {
                    return Err(format!("Tensor.set({i}, {j}) out of range for {}×{}", me.rows, me.cols));
                }
                me.data.borrow_mut()[i * me.cols + j] = v;
                return Ok(Value::Nil);
            }
            "copy_from" | "assign" => {
                let src = bin(a, m)?;
                if src.rows != me.rows || src.cols != me.cols {
                    return Err(shape_err("copy_from", me, &src));
                }
                let d = src.data.borrow().clone();
                *me.data.borrow_mut() = d;
                return Ok(Value::Nil);
            }
            "lerp_from" => {
                // Polyak averaging for target networks: self ← (1-τ)·self + τ·src
                let src = bin(a, m)?;
                let tau = farg(a, 1, "Tensor.lerp_from")?;
                if src.rows != me.rows || src.cols != me.cols {
                    return Err(shape_err("lerp_from", me, &src));
                }
                let s = src.data.borrow().clone();
                for (x, y) in me.data.borrow_mut().iter_mut().zip(s) {
                    *x = (1.0 - tau) * *x + tau * y;
                }
                return Ok(Value::Nil);
            }
            "backward" => {
                backward(me)?;
                return Ok(Value::Nil);
            }
            "grad" => {
                let g = me.grad.borrow();
                let d = if g.is_empty() { vec![0.0; me.len()] } else { g.clone() };
                return Ok(wrap(Node::leaf(me.rows, me.cols, d, false)));
            }
            "zero_grad" => {
                me.grad.borrow_mut().clear();
                return Ok(Value::Nil);
            }
            "norm" => return Ok(Value::Float(me.data.borrow().iter().map(|x| x * x).sum::<f64>().sqrt())),
            "max" => return Ok(Value::Float(me.data.borrow().iter().cloned().fold(f64::NEG_INFINITY, f64::max))),
            "min" => return Ok(Value::Float(me.data.borrow().iter().cloned().fold(f64::INFINITY, f64::min))),
            "argmax" => {
                let d = me.data.borrow();
                let (i, _) = d.iter().enumerate().fold((0, f64::NEG_INFINITY), |b, (i, &x)| if x > b.1 { (i, x) } else { b });
                return Ok(Value::Int(i as i64));
            }
            "isfinite" => return Ok(Value::Bool(me.data.borrow().iter().all(|x| x.is_finite()))),
            "detach" => Node::leaf(me.rows, me.cols, me.data.borrow().clone(), false),
            "param" | "requires_grad_" => Node::leaf(me.rows, me.cols, me.data.borrow().clone(), true),
            "clone" => Node::leaf(me.rows, me.cols, me.data.borrow().clone(), me.requires_grad),
            "matmul" | "mm" | "dot" => matmul(me, &bin(a, m)?)?,
            "op_add" | "op_radd" | "add" => elementwise(Op::Add, me, &bin(a, m)?, |x, y| x + y, "add")?,
            "op_sub" | "sub" => elementwise(Op::Sub, me, &bin(a, m)?, |x, y| x - y, "sub")?,
            "op_rsub" => elementwise(Op::Sub, &bin(a, m)?, me, |x, y| x - y, "sub")?,
            "op_mul" | "op_rmul" | "mul" => {
                if let Some(k) = a.first().and_then(scalar) {
                    scale(me, k)
                } else {
                    elementwise(Op::Mul, me, &bin(a, m)?, |x, y| x * y, "mul")?
                }
            }
            "op_div" | "div" => {
                if let Some(k) = a.first().and_then(scalar) {
                    scale(me, 1.0 / k)
                } else {
                    elementwise(Op::Div, me, &bin(a, m)?, |x, y| x / y, "div")?
                }
            }
            "op_rdiv" => elementwise(Op::Div, &bin(a, m)?, me, |x, y| x / y, "div")?,
            "add_scalar" => {
                let k = farg(a, 0, "Tensor.add_scalar")?;
                unary(Op::AddS(k), me, |x| x + k)
            }
            "scale" => scale(me, farg(a, 0, "Tensor.scale")?),
            "pow" => {
                let e = farg(a, 0, "Tensor.pow")?;
                unary(Op::PowS(e), me, |x| x.powf(e))
            }
            "square" => unary(Op::PowS(2.0), me, |x| x * x),
            "neg" => unary(Op::Neg, me, |x| -x),
            "tanh" => unary(Op::Tanh, me, f64::tanh),
            "relu" => unary(Op::Relu, me, |x| x.max(0.0)),
            "sigmoid" => unary(Op::Sigmoid, me, |x| 1.0 / (1.0 + (-x).exp())),
            "exp" => unary(Op::Exp, me, f64::exp),
            "log" => unary(Op::Log, me, f64::ln),
            "sqrt" => unary(Op::Sqrt, me, f64::sqrt),
            "abs" => unary(Op::Abs, me, f64::abs),
            "softplus" => unary(Op::Softplus, me, |x| if x > 30.0 { x } else { x.exp().ln_1p() }),
            "clamp" | "clip" => {
                let (lo, hi) = (farg(a, 0, "Tensor.clamp")?, farg(a, 1, "Tensor.clamp")?);
                unary(Op::Clamp(lo, hi), me, |x| x.clamp(lo, hi))
            }
            "sum" | "mean" => {
                let mut s = match a.first() {
                    None | Some(Value::Nil) => {
                        let total: f64 = me.data.borrow().iter().sum();
                        Node::derived(1, 1, vec![total], Op::Sum, vec![me.clone()])
                    }
                    Some(d) => sum_dim(me, d.as_i64().ok_or("Tensor.sum: dim must be 0 or 1")? as usize)?,
                };
                if m == "mean" {
                    let n = match a.first() {
                        None | Some(Value::Nil) => me.len(),
                        Some(d) if d.as_i64() == Some(0) => me.rows,
                        _ => me.cols,
                    };
                    s = scale(&s, 1.0 / n.max(1) as f64);
                }
                s
            }
            "t" | "transpose" | "T" => {
                let d = me.data.borrow();
                let mut o = vec![0.0; me.len()];
                for i in 0..me.rows {
                    for j in 0..me.cols {
                        o[j * me.rows + i] = d[i * me.cols + j];
                    }
                }
                drop(d);
                Node::derived(me.cols, me.rows, o, Op::Transpose, vec![me.clone()])
            }
            "cols_slice" | "slice_cols" => slice_cols(me, uarg(a, 0, "Tensor.slice_cols")?, uarg(a, 1, "Tensor.slice_cols")?)?,
            "rows_slice" | "slice_rows" => slice_rows(me, uarg(a, 0, "Tensor.slice_rows")?, uarg(a, 1, "Tensor.slice_rows")?)?,
            "dropout" => {
                let p = farg(a, 0, "Tensor.dropout")?;
                let training = a.get(1).map_or(true, |v| v.truthy());
                if !training || p <= 0.0 {
                    return Ok(wrap(me.clone()));
                }
                if p >= 1.0 {
                    return Err("Tensor.dropout: p must be < 1".into());
                }
                let keep = 1.0 / (1.0 - p);
                let mask: Vec<f64> = (0..me.len()).map(|_| if uniform() < p { 0.0 } else { keep }).collect();
                let out: Vec<f64> = me.data.borrow().iter().zip(&mask).map(|(x, m)| x * m).collect();
                Node::derived(me.rows, me.cols, out, Op::Mask(Rc::new(mask)), vec![me.clone()])
            }
            "op_eq" => {
                let b = bin(a, m)?;
                return Ok(Value::Bool(Rc::ptr_eq(me, &b) || (b.rows == me.rows && b.cols == me.cols && *b.data.borrow() == *me.data.borrow())));
            }
            _ => return Err(format!("Tensor has no method '{m}'")),
        };
        Ok(wrap(t))
    }
}

// ---------------------------------------------------------------------------- layers

fn init_matrix(rows: usize, cols: usize, gain: f64) -> Vec<f64> {
    // Xavier/Glorot normal
    let std = gain * (2.0 / (rows + cols) as f64).sqrt();
    (0..rows * cols).map(|_| gauss() * std).collect()
}

pub struct Linear {
    w: T,
    b: T,
}

impl Linear {
    fn new(inp: usize, out: usize, gain: f64) -> Self {
        Linear {
            w: Node::leaf(inp, out, init_matrix(inp, out, gain), true),
            b: Node::leaf(1, out, vec![0.0; out], true),
        }
    }
    fn forward(&self, x: &T) -> Result<T, String> {
        let y = matmul(x, &self.w)?;
        elementwise(Op::Add, &y, &self.b, |p, q| p + q, "linear bias")
    }
}

impl Native for Linear {
    fn type_name(&self) -> &'static str {
        "Linear"
    }
    fn describe(&self) -> String {
        format!("<Linear {}→{}>", self.w.rows, self.w.cols)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "forward" | "call" => Ok(wrap(self.forward(&operand(a.first().ok_or("Linear.forward(x)")?, None)?)?)),
            "params" | "parameters" => Ok(Value::list(vec![wrap(self.w.clone()), wrap(self.b.clone())])),
            "weight" => Ok(wrap(self.w.clone())),
            "bias" => Ok(wrap(self.b.clone())),
            _ => Err(format!("Linear has no method '{m}'")),
        }
    }
}

#[derive(Clone, Copy)]
enum Act {
    Tanh,
    Relu,
    Sigmoid,
    Linear,
    Elu,
}

impl Act {
    fn parse(s: &str) -> Result<Act, String> {
        Ok(match s {
            "tanh" => Act::Tanh,
            "relu" => Act::Relu,
            "sigmoid" => Act::Sigmoid,
            "linear" | "none" | "identity" => Act::Linear,
            "elu" | "softplus" => Act::Elu,
            _ => return Err(format!("nn: unknown activation '{s}' (tanh, relu, sigmoid, softplus, linear)")),
        })
    }
    fn name(self) -> &'static str {
        match self {
            Act::Tanh => "tanh",
            Act::Relu => "relu",
            Act::Sigmoid => "sigmoid",
            Act::Linear => "linear",
            Act::Elu => "softplus",
        }
    }
    fn apply(self, x: &T) -> T {
        match self {
            Act::Tanh => unary(Op::Tanh, x, f64::tanh),
            Act::Relu => unary(Op::Relu, x, |v| v.max(0.0)),
            Act::Sigmoid => unary(Op::Sigmoid, x, |v| 1.0 / (1.0 + (-v).exp())),
            Act::Linear => x.clone(),
            Act::Elu => unary(Op::Softplus, x, |v| if v > 30.0 { v } else { v.exp().ln_1p() }),
        }
    }
}

/// Multi-layer perceptron: `sizes[0] → … → sizes[n]`, hidden activation, optional dropout.
pub struct Mlp {
    layers: Vec<Linear>,
    act: Act,
    out_act: Act,
    dropout: f64,
    training: bool,
}

impl Mlp {
    fn params(&self) -> Vec<T> {
        self.layers.iter().flat_map(|l| [l.w.clone(), l.b.clone()]).collect()
    }
    fn forward(&self, x: &T) -> Result<T, String> {
        let mut h = x.clone();
        let n = self.layers.len();
        for (i, l) in self.layers.iter().enumerate() {
            h = l.forward(&h)?;
            if i + 1 < n {
                h = self.act.apply(&h);
                if self.training && self.dropout > 0.0 {
                    let keep = 1.0 / (1.0 - self.dropout);
                    let mask: Vec<f64> = (0..h.len()).map(|_| if uniform() < self.dropout { 0.0 } else { keep }).collect();
                    let out: Vec<f64> = h.data.borrow().iter().zip(&mask).map(|(x, m)| x * m).collect();
                    h = Node::derived(h.rows, h.cols, out, Op::Mask(Rc::new(mask)), vec![h.clone()]);
                }
            } else {
                h = self.out_act.apply(&h);
            }
        }
        Ok(h)
    }
}

impl Native for Mlp {
    fn type_name(&self) -> &'static str {
        "MLP"
    }
    fn describe(&self) -> String {
        let mut sizes: Vec<String> = vec![self.layers[0].w.rows.to_string()];
        sizes.extend(self.layers.iter().map(|l| l.w.cols.to_string()));
        format!("<MLP {} {}>", sizes.join("→"), self.act.name())
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "forward" | "call" | "predict" => {
                let x = operand(a.first().ok_or("MLP.forward(x)")?, None)?;
                Ok(wrap(self.forward(&x)?))
            }
            "params" | "parameters" => Ok(Value::list(self.params().into_iter().map(wrap).collect())),
            "n_params" => Ok(Value::Int(self.params().iter().map(|p| p.len() as i64).sum())),
            "train" => {
                self.training = a.first().map_or(true, |v| v.truthy());
                Ok(Value::Nil)
            }
            "eval" => {
                self.training = false;
                Ok(Value::Nil)
            }
            "training" => Ok(Value::Bool(self.training)),
            "layers" => Ok(Value::Int(self.layers.len() as i64)),
            "save" => {
                let path = str_arg(a, 0, "MLP.save")?;
                save_params(&path, &self.params())?;
                Ok(Value::Nil)
            }
            "load" => {
                let path = str_arg(a, 0, "MLP.load")?;
                load_params(&path, &self.params())?;
                Ok(Value::Nil)
            }
            "copy_from" => {
                let src = match a.first() {
                    Some(Value::Native(rc)) => {
                        let b = rc.try_borrow().map_err(|_| "MLP.copy_from: cannot copy from itself")?;
                        let any = b.as_any().ok_or("MLP.copy_from: expected an MLP")?;
                        let other = any.downcast_ref::<Mlp>().ok_or("MLP.copy_from: expected an MLP")?;
                        other.params()
                    }
                    _ => return Err("MLP.copy_from: expected an MLP".into()),
                };
                let tau = a.get(1).and_then(scalar).unwrap_or(1.0);
                let mine = self.params();
                if mine.len() != src.len() || mine.iter().zip(&src).any(|(x, y)| x.len() != y.len()) {
                    return Err("MLP.copy_from: architectures differ".into());
                }
                for (dst, s) in mine.iter().zip(&src) {
                    let s = s.data.borrow().clone();
                    for (x, y) in dst.data.borrow_mut().iter_mut().zip(s) {
                        *x = (1.0 - tau) * *x + tau * y;
                    }
                }
                Ok(Value::Nil)
            }
            _ => Err(format!("MLP has no method '{m}'")),
        }
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

fn str_arg(a: &[Value], i: usize, what: &str) -> Result<String, String> {
    match a.get(i) {
        Some(Value::Str(s)) => Ok(s.to_string()),
        _ => Err(format!("{what}: argument {} must be a string", i + 1)),
    }
}

fn save_params(path: &str, ps: &[T]) -> Result<(), String> {
    let mut s = String::from("tulpa-nn 1\n");
    s.push_str(&format!("{}\n", ps.len()));
    for p in ps {
        s.push_str(&format!("{} {}\n", p.rows, p.cols));
        let d = p.data.borrow();
        let line: Vec<String> = d.iter().map(|x| format!("{x:e}")).collect();
        s.push_str(&line.join(" "));
        s.push('\n');
    }
    std::fs::write(path, s).map_err(|e| format!("nn: cannot write {path}: {e}"))
}

fn load_params(path: &str, ps: &[T]) -> Result<(), String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("nn: cannot read {path}: {e}"))?;
    let mut lines = s.lines();
    if lines.next() != Some("tulpa-nn 1") {
        return Err(format!("nn: {path} is not a tulpa-nn weight file"));
    }
    let n: usize = lines.next().and_then(|l| l.trim().parse().ok()).ok_or("nn: corrupt weight file")?;
    if n != ps.len() {
        return Err(format!("nn: {path} holds {n} tensors, model has {}", ps.len()));
    }
    for p in ps {
        let shape: Vec<usize> = lines
            .next()
            .ok_or("nn: truncated weight file")?
            .split_whitespace()
            .map(|x| x.parse().map_err(|_| "nn: corrupt shape"))
            .collect::<Result<_, _>>()?;
        if shape != [p.rows, p.cols] {
            return Err(format!("nn: shape {:?} in file vs {}×{} in model", shape, p.rows, p.cols));
        }
        let vals: Vec<f64> = lines
            .next()
            .unwrap_or("")
            .split_whitespace()
            .map(|x| x.parse().map_err(|_| "nn: corrupt value"))
            .collect::<Result<_, _>>()?;
        if vals.len() != p.len() {
            return Err("nn: value count mismatch".into());
        }
        *p.data.borrow_mut() = vals;
    }
    Ok(())
}

// ---------------------------------------------------------------------------- optimisers

pub struct Adam {
    params: Vec<T>,
    m: Vec<Vec<f64>>,
    v: Vec<Vec<f64>>,
    t: i32,
    lr: f64,
    b1: f64,
    b2: f64,
    eps: f64,
    weight_decay: f64,
    clip: Option<f64>,
    sgd: bool,
}

impl Adam {
    fn grad_norm(&self) -> f64 {
        self.params
            .iter()
            .map(|p| p.grad.borrow().iter().map(|g| g * g).sum::<f64>())
            .sum::<f64>()
            .sqrt()
    }
}

impl Native for Adam {
    fn type_name(&self) -> &'static str {
        if self.sgd {
            "SGD"
        } else {
            "Adam"
        }
    }
    fn describe(&self) -> String {
        format!("<{} lr={} params={}>", self.type_name(), self.lr, self.params.len())
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        match m {
            "zero_grad" => {
                for p in &self.params {
                    p.grad.borrow_mut().clear();
                }
                Ok(Value::Nil)
            }
            "grad_norm" => Ok(Value::Float(self.grad_norm())),
            "step" => {
                let scale = match self.clip {
                    Some(c) => {
                        let n = self.grad_norm();
                        if n > c {
                            c / n
                        } else {
                            1.0
                        }
                    }
                    None => 1.0,
                };
                self.t += 1;
                let (b1, b2) = (self.b1, self.b2);
                let bc1 = 1.0 - b1.powi(self.t);
                let bc2 = 1.0 - b2.powi(self.t);
                for (k, p) in self.params.iter().enumerate() {
                    let g = p.grad.borrow();
                    if g.is_empty() {
                        continue;
                    }
                    let mut d = p.data.borrow_mut();
                    for i in 0..d.len() {
                        let gi = g[i] * scale + self.weight_decay * d[i];
                        if self.sgd {
                            // SGD with momentum b1
                            self.m[k][i] = b1 * self.m[k][i] + gi;
                            d[i] -= self.lr * self.m[k][i];
                        } else {
                            self.m[k][i] = b1 * self.m[k][i] + (1.0 - b1) * gi;
                            self.v[k][i] = b2 * self.v[k][i] + (1.0 - b2) * gi * gi;
                            let mh = self.m[k][i] / bc1;
                            let vh = self.v[k][i] / bc2;
                            d[i] -= self.lr * mh / (vh.sqrt() + self.eps);
                        }
                    }
                }
                Ok(Value::Nil)
            }
            "set_lr" => {
                self.lr = farg(a, 0, "set_lr")?;
                Ok(Value::Nil)
            }
            "lr" => Ok(Value::Float(self.lr)),
            "steps" => Ok(Value::Int(self.t as i64)),
            "clip_grad" | "set_clip" => {
                self.clip = a.first().and_then(scalar);
                Ok(Value::Nil)
            }
            _ => Err(format!("{} has no method '{m}'", self.type_name())),
        }
    }
}

// ---------------------------------------------------------------------------- running scaler

/// Running per-feature mean/variance (Welford) for online input normalisation.
pub struct Scaler {
    n: f64,
    mean: Vec<f64>,
    m2: Vec<f64>,
    clip: f64,
}

impl Native for Scaler {
    fn type_name(&self) -> &'static str {
        "Scaler"
    }
    fn describe(&self) -> String {
        format!("<Scaler features={} n={}>", self.mean.len(), self.n)
    }
    fn call_method(&mut self, m: &str, a: &[Value]) -> Res {
        let std = |s: &Scaler| -> Vec<f64> {
            s.m2.iter().map(|m2| if s.n > 1.0 { (m2 / (s.n - 1.0)).sqrt().max(1e-6) } else { 1.0 }).collect()
        };
        match m {
            "save" => {
                let path = str_arg(a, 0, "Scaler.save")?;
                let line = |v: &[f64]| v.iter().map(|x| format!("{x:e}")).collect::<Vec<_>>().join(" ");
                let body = format!("tulpa-scaler 1\n{} {} {}\n{}\n{}\n", self.mean.len(), self.n, self.clip, line(&self.mean), line(&self.m2));
                std::fs::write(&path, body).map_err(|e| format!("Scaler.save {path}: {e}"))?;
                Ok(Value::Nil)
            }
            "load" => {
                let path = str_arg(a, 0, "Scaler.load")?;
                let s = std::fs::read_to_string(&path).map_err(|e| format!("Scaler.load {path}: {e}"))?;
                let mut l = s.lines();
                if l.next() != Some("tulpa-scaler 1") {
                    return Err(format!("Scaler.load: {path} is not a scaler file"));
                }
                let head: Vec<f64> = l.next().unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if head.len() != 3 || head[0] as usize != self.mean.len() {
                    return Err(format!("Scaler.load: {path} has a different feature count"));
                }
                let parse = |x: Option<&str>| -> Vec<f64> { x.unwrap_or("").split_whitespace().filter_map(|v| v.parse().ok()).collect() };
                let (mean, m2) = (parse(l.next()), parse(l.next()));
                if mean.len() != self.mean.len() || m2.len() != self.m2.len() {
                    return Err("Scaler.load: corrupt file".into());
                }
                self.n = head[1];
                self.mean = mean;
                self.m2 = m2;
                Ok(Value::Nil)
            }
            "update" | "observe" => {
                let x = operand(a.first().ok_or("Scaler.update(x)")?, None)?;
                if x.cols != self.mean.len() {
                    return Err(format!("Scaler.update: expected {} features, got {}", self.mean.len(), x.cols));
                }
                let d = x.data.borrow();
                for i in 0..x.rows {
                    self.n += 1.0;
                    for j in 0..x.cols {
                        let v = d[i * x.cols + j];
                        let delta = v - self.mean[j];
                        self.mean[j] += delta / self.n;
                        self.m2[j] += delta * (v - self.mean[j]);
                    }
                }
                Ok(Value::Nil)
            }
            "transform" | "normalize" => {
                let x = operand(a.first().ok_or("Scaler.transform(x)")?, None)?;
                if x.cols != self.mean.len() {
                    return Err(format!("Scaler.transform: expected {} features, got {}", self.mean.len(), x.cols));
                }
                let s = std(self);
                let d = x.data.borrow();
                let out: Vec<f64> = d
                    .iter()
                    .enumerate()
                    .map(|(k, v)| ((v - self.mean[k % x.cols]) / s[k % x.cols]).clamp(-self.clip, self.clip))
                    .collect();
                Ok(wrap(Node::leaf(x.rows, x.cols, out, false)))
            }
            "mean" => Ok(Value::list(self.mean.iter().map(|x| Value::Float(*x)).collect())),
            "std" => Ok(Value::list(std(self).into_iter().map(Value::Float).collect())),
            "count" => Ok(Value::Float(self.n)),
            "ready" => Ok(Value::Bool(self.n >= a.first().and_then(scalar).unwrap_or(2.0))),
            _ => Err(format!("Scaler has no method '{m}'")),
        }
    }
}

// ---------------------------------------------------------------------------- module functions

fn params_of(v: &Value, what: &str) -> Result<Vec<T>, String> {
    let Value::List(l) = v else {
        return Err(format!("{what}: expected a list of tensors (e.g. net.params())"));
    };
    let mut out = Vec::new();
    for x in l.borrow().iter() {
        match x {
            Value::List(_) => out.extend(params_of(x, what)?),
            _ => out.push(tensor_of(x, None).ok_or_else(|| format!("{what}: list item is a {}, not a tensor", x.type_name()))?),
        }
    }
    Ok(out)
}

fn make_opt(a: &[Value], sgd: bool) -> Res {
    let what = if sgd { "nn::SGD" } else { "nn::Adam" };
    let params = params_of(a.first().ok_or_else(|| format!("{what}(params, lr)"))?, what)?;
    if let Some(p) = params.iter().find(|p| !p.requires_grad) {
        return Err(format!("{what}: a {}×{} tensor does not require gradients (use .param())", p.rows, p.cols));
    }
    let lr = a.get(1).and_then(scalar).unwrap_or(if sgd { 0.01 } else { 1e-3 });
    let zeros: Vec<Vec<f64>> = params.iter().map(|p| vec![0.0; p.len()]).collect();
    Ok(Value::native(Adam {
        m: zeros.clone(),
        v: zeros,
        params,
        t: 0,
        lr,
        b1: if sgd { a.get(2).and_then(scalar).unwrap_or(0.0) } else { 0.9 },
        b2: 0.999,
        eps: 1e-8,
        weight_decay: if sgd { 0.0 } else { a.get(2).and_then(scalar).unwrap_or(0.0) },
        clip: None,
        sgd,
    }))
}

pub const NN: &[(&str, NativeFn)] = &[
    ("tensor", |_, a: Vec<Value>| {
        let t = from_value(a.first().ok_or("nn::tensor(data)")?)?;
        let rg = a.get(1).map_or(false, |v| v.truthy());
        let d = t.data.borrow().clone();
        Ok(wrap(Node::leaf(t.rows, t.cols, d, rg)))
    }),
    ("param", |_, a: Vec<Value>| {
        let t = from_value(a.first().ok_or("nn::param(data)")?)?;
        let d = t.data.borrow().clone();
        Ok(wrap(Node::leaf(t.rows, t.cols, d, true)))
    }),
    ("zeros", |_, a: Vec<Value>| {
        let (r, c) = (uarg(&a, 0, "nn::zeros")?, uarg(&a, 1, "nn::zeros")?);
        Ok(wrap(Node::leaf(r, c, vec![0.0; r * c], false)))
    }),
    ("ones", |_, a: Vec<Value>| {
        let (r, c) = (uarg(&a, 0, "nn::ones")?, uarg(&a, 1, "nn::ones")?);
        Ok(wrap(Node::leaf(r, c, vec![1.0; r * c], false)))
    }),
    ("full", |_, a: Vec<Value>| {
        let (r, c, v) = (uarg(&a, 0, "nn::full")?, uarg(&a, 1, "nn::full")?, farg(&a, 2, "nn::full")?);
        Ok(wrap(Node::leaf(r, c, vec![v; r * c], false)))
    }),
    ("randn", |_, a: Vec<Value>| {
        let (r, c) = (uarg(&a, 0, "nn::randn")?, uarg(&a, 1, "nn::randn")?);
        let s = a.get(2).and_then(scalar).unwrap_or(1.0);
        Ok(wrap(Node::leaf(r, c, (0..r * c).map(|_| gauss() * s).collect(), false)))
    }),
    ("rand", |_, a: Vec<Value>| {
        let (r, c) = (uarg(&a, 0, "nn::rand")?, uarg(&a, 1, "nn::rand")?);
        Ok(wrap(Node::leaf(r, c, (0..r * c).map(|_| uniform()).collect(), false)))
    }),
    ("eye", |_, a: Vec<Value>| {
        let n = uarg(&a, 0, "nn::eye")?;
        let mut d = vec![0.0; n * n];
        for i in 0..n {
            d[i * n + i] = 1.0;
        }
        Ok(wrap(Node::leaf(n, n, d, false)))
    }),
    ("seed", |_, a: Vec<Value>| {
        seed(a.first().and_then(|v| v.as_i64()).ok_or("nn::seed(n)")? as u64);
        Ok(Value::Nil)
    }),
    ("gauss", |_, a: Vec<Value>| {
        let (mu, s) = (a.first().and_then(scalar).unwrap_or(0.0), a.get(1).and_then(scalar).unwrap_or(1.0));
        Ok(Value::Float(mu + s * gauss()))
    }),
    ("uniform", |_, _| Ok(Value::Float(uniform()))),
    ("cat", |_, a: Vec<Value>| {
        let ts = params_or_values(a.first().ok_or("nn::cat(list, dim=1)")?)?;
        let dim = a.get(1).and_then(|v| v.as_i64()).unwrap_or(1) as usize;
        Ok(wrap(cat(&ts, dim)?))
    }),
    ("stack", |_, a: Vec<Value>| {
        let ts = params_or_values(a.first().ok_or("nn::stack(rows)")?)?;
        Ok(wrap(cat(&ts, 0)?))
    }),
    ("mse", |_, a: Vec<Value>| {
        let (x, y) = (operand(a.first().ok_or("nn::mse(a, b)")?, None)?, operand(a.get(1).ok_or("nn::mse(a, b)")?, None)?);
        let d = elementwise(Op::Sub, &x, &y, |p, q| p - q, "mse")?;
        let sq = unary(Op::PowS(2.0), &d, |v| v * v);
        let total: f64 = sq.data.borrow().iter().sum();
        let n = sq.len().max(1) as f64;
        Ok(wrap(scale(&Node::derived(1, 1, vec![total], Op::Sum, vec![sq.clone()]), 1.0 / n)))
    }),
    ("gaussian_log_prob", |_, a: Vec<Value>| {
        // Σ over columns of log N(x; μ, exp(log_std)) → R×1
        let x = operand(a.first().ok_or("nn::gaussian_log_prob(x, mu, log_std)")?, None)?;
        let mu = operand(a.get(1).ok_or("nn::gaussian_log_prob(x, mu, log_std)")?, None)?;
        let ls = operand(a.get(2).ok_or("nn::gaussian_log_prob(x, mu, log_std)")?, None)?;
        let diff = elementwise(Op::Sub, &x, &mu, |p, q| p - q, "log_prob")?;
        let inv = unary(Op::Exp, &unary(Op::Neg, &ls, |v| -v), f64::exp);
        let z = elementwise(Op::Mul, &diff, &inv, |p, q| p * q, "log_prob")?;
        let z2 = scale(&unary(Op::PowS(2.0), &z, |v| v * v), -0.5);
        let c = 0.5 * (std::f64::consts::TAU).ln();
        let t = elementwise(Op::Sub, &z2, &ls, |p, q| p - q, "log_prob")?;
        let t = unary(Op::AddS(-c), &t, |v| v - c);
        Ok(wrap(sum_dim(&t, 1)?))
    }),
    ("sample_gaussian", |_, a: Vec<Value>| {
        let mu = operand(a.first().ok_or("nn::sample_gaussian(mu, log_std)")?, None)?;
        let ls = operand(a.get(1).ok_or("nn::sample_gaussian(mu, log_std)")?, None)?;
        let (md, ld) = (mu.data.borrow(), ls.data.borrow());
        let mut out = Vec::with_capacity(mu.len());
        for i in 0..mu.rows {
            for j in 0..mu.cols {
                out.push(md[i * mu.cols + j] + ld[bidx(&ls, i, j)].exp() * gauss());
            }
        }
        Ok(wrap(Node::leaf(mu.rows, mu.cols, out, false)))
    }),
    ("Linear", |_, a: Vec<Value>| {
        let (i, o) = (uarg(&a, 0, "nn::Linear")?, uarg(&a, 1, "nn::Linear")?);
        let gain = a.get(2).and_then(scalar).unwrap_or(1.0);
        Ok(Value::native(Linear::new(i, o, gain)))
    }),
    ("MLP", |_, a: Vec<Value>| {
        let sizes: Vec<usize> = floats_of(a.first().ok_or("nn::MLP(sizes, act=\"tanh\", dropout=0, out_act=\"linear\")")?, "nn::MLP")?
            .into_iter()
            .map(|x| x as usize)
            .collect();
        if sizes.len() < 2 || sizes.contains(&0) {
            return Err("nn::MLP: need at least two positive layer sizes".into());
        }
        let act = match a.get(1) {
            Some(Value::Str(s)) => Act::parse(s)?,
            _ => Act::Tanh,
        };
        let dropout = a.get(2).and_then(scalar).unwrap_or(0.0);
        if !(0.0..1.0).contains(&dropout) {
            return Err("nn::MLP: dropout must be in [0, 1)".into());
        }
        let out_act = match a.get(3) {
            Some(Value::Str(s)) => Act::parse(s)?,
            _ => Act::Linear,
        };
        let n = sizes.len() - 1;
        let layers = (0..n)
            .map(|k| Linear::new(sizes[k], sizes[k + 1], if k + 1 == n { a.get(4).and_then(scalar).unwrap_or(1.0) } else { 1.0 }))
            .collect();
        Ok(Value::native(Mlp { layers, act, out_act, dropout, training: true }))
    }),
    ("Adam", |_, a: Vec<Value>| make_opt(&a, false)),
    ("SGD", |_, a: Vec<Value>| make_opt(&a, true)),
    ("Scaler", |_, a: Vec<Value>| {
        let n = uarg(&a, 0, "nn::Scaler(features)")?;
        let clip = a.get(1).and_then(scalar).unwrap_or(10.0);
        Ok(Value::native(Scaler { n: 0.0, mean: vec![0.0; n], m2: vec![0.0; n], clip }))
    }),
    ("save", |_, a: Vec<Value>| {
        let path = str_arg(&a, 0, "nn::save(path, params)")?;
        save_params(&path, &params_of(a.get(1).ok_or("nn::save(path, params)")?, "nn::save")?)?;
        Ok(Value::Nil)
    }),
    ("load", |_, a: Vec<Value>| {
        let path = str_arg(&a, 0, "nn::load(path, params)")?;
        load_params(&path, &params_of(a.get(1).ok_or("nn::load(path, params)")?, "nn::load")?)?;
        Ok(Value::Nil)
    }),
    ("is_tensor", |_, a: Vec<Value>| Ok(Value::Bool(a.first().and_then(|v| tensor_of(v, None)).is_some()))),
];

fn params_or_values(v: &Value) -> Result<Vec<T>, String> {
    let Value::List(l) = v else {
        return Err("nn: expected a list".into());
    };
    l.borrow().iter().map(|x| operand(x, None)).collect()
}

