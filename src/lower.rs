//! Lowering passes that run before native code generation.
//!
//! `desugar_matches` rewrites `match` over scalars into `if` chains, which the native
//! compiler already understands:
//!
//! ```text
//! match code { 0 => a(), 1 | 2 => b(), 3..=9 => c(), n if n > 100 => d(n), _ => e() }
//! ```
//! becomes `let __m0 = code` followed by `if __m0 == 0 { a() } elif ...`. Matches with list
//! patterns are left alone (and stay on the interpreter).

use crate::ast::*;
use std::rc::Rc;

fn rename(e: &Expr, from: &str, to: &str) -> Expr {
    let r = |x: &Expr| Box::new(rename(x, from, to));
    match e {
        Expr::Ident(n, l) if n == from => Expr::Ident(to.to_string(), *l),
        Expr::Unary(op, a, l) => Expr::Unary(op.clone(), r(a), *l),
        Expr::Binary(a, op, b, l) => Expr::Binary(r(a), op.clone(), r(b), *l),
        Expr::Logic(a, k, b) => Expr::Logic(r(a), *k, r(b)),
        Expr::Call(f, args, l) => {
            Expr::Call(r(f), args.iter().map(|a| rename(a, from, to)).collect(), *l)
        }
        Expr::MethodCall(o, m, args, l) => Expr::MethodCall(
            r(o),
            m.clone(),
            args.iter().map(|a| rename(a, from, to)).collect(),
            *l,
        ),
        Expr::Field(o, f, l) => Expr::Field(r(o), f.clone(), *l),
        Expr::Index(o, i, l) => Expr::Index(r(o), r(i), *l),
        other => other.clone(),
    }
}

struct Lower {
    n: usize,
}

impl Lower {
    /// The `if` chain for a match, or None if a pattern cannot be lowered.
    fn chain(
        &mut self,
        tmp: &str,
        arms: &[MatchArm],
        line: usize,
    ) -> Option<(Expr, Vec<Stmt>, Option<Vec<Stmt>>)> {
        let var = || Expr::Ident(tmp.to_string(), line);
        let mut parts: Vec<(Expr, Vec<Stmt>)> = Vec::new();
        for arm in arms {
            let mut cond: Option<Expr> = None;
            let mut binds = Vec::new();
            for p in &arm.patterns {
                let c = match p {
                    Pattern::Wildcard => Expr::Bool(true),
                    Pattern::Bind(n) => {
                        binds.push(n.clone());
                        Expr::Bool(true)
                    }
                    Pattern::Literal(e) => {
                        Expr::Binary(Box::new(var()), "==".into(), Box::new(e.clone()), line)
                    }
                    Pattern::Range(a, b, incl) => Expr::Logic(
                        Box::new(Expr::Binary(
                            Box::new(var()),
                            ">=".into(),
                            Box::new(a.clone()),
                            line,
                        )),
                        true,
                        Box::new(Expr::Binary(
                            Box::new(var()),
                            if *incl { "<=" } else { "<" }.into(),
                            Box::new(b.clone()),
                            line,
                        )),
                    ),
                    Pattern::List(..) => return None,
                };
                cond = Some(match cond {
                    None => c,
                    Some(prev) => Expr::Logic(Box::new(prev), false, Box::new(c)),
                });
            }
            let mut cond = cond.unwrap_or(Expr::Bool(true));
            if let Some(g) = &arm.guard {
                let g = binds.iter().fold(g.clone(), |g, b| rename(&g, b, tmp));
                cond = match cond {
                    Expr::Bool(true) => g,
                    c => Expr::Logic(Box::new(c), true, Box::new(g)),
                };
            }
            let mut body: Vec<Stmt> = binds
                .iter()
                .map(|b| Stmt::Let(vec![b.clone()], var(), false, line))
                .collect();
            body.extend(self.block(&arm.body));
            parts.push((cond, body));
        }
        // fold from the last arm backwards into if / else-if
        let mut tail: Option<Vec<Stmt>> = match parts.last() {
            Some((Expr::Bool(true), _)) => Some(parts.pop().unwrap().1),
            _ => Some(vec![Stmt::Expr(
                Expr::Call(
                    Box::new(Expr::Ident("error".into(), line)),
                    vec![Expr::Str("no match arm matched".into())],
                    line,
                ),
                line,
            )]),
        };
        let (first_cond, first_body) = if parts.is_empty() {
            (Expr::Bool(true), tail.take().unwrap())
        } else {
            let first = parts.remove(0);
            for (c, b) in parts.into_iter().rev() {
                tail = Some(vec![Stmt::If(c, b, tail)]);
            }
            first
        };
        Some((first_cond, first_body, tail))
    }

    /// Lowers a match in value position: returns the hoisted `let` and the replacement.
    fn value(&mut self, e: &Expr) -> Option<(Stmt, Expr)> {
        let Expr::Match(scrut, arms, line) = e else {
            return None;
        };
        let tmp = format!("__m{}", self.n);
        self.n += 1;
        let (c, a, b) = self.chain(&tmp, arms, *line)?;
        Some((
            Stmt::Let(vec![tmp], (**scrut).clone(), false, *line),
            Expr::IfExpr(Box::new(c), a, b),
        ))
    }

    fn func(&mut self, d: &Rc<FnDef>) -> Rc<FnDef> {
        let mut f = (**d).clone();
        f.body = self.block(&d.body);
        Rc::new(f)
    }

    fn block(&mut self, stmts: &[Stmt]) -> Vec<Stmt> {
        let mut out = Vec::new();
        for s in stmts {
            match s {
                Stmt::Expr(Expr::Match(scrut, arms, line), l) => {
                    let tmp = format!("__m{}", self.n);
                    self.n += 1;
                    match self.chain(&tmp, arms, *line) {
                        Some((c, a, b)) => {
                            out.push(Stmt::Let(vec![tmp], (**scrut).clone(), false, *line));
                            out.push(Stmt::If(c, a, b));
                        }
                        None => out.push(Stmt::Expr(
                            Expr::Match(scrut.clone(), arms.clone(), *line),
                            *l,
                        )),
                    }
                }
                Stmt::Let(names, e, c, l) => match self.value(e) {
                    Some((hoist, v)) => {
                        out.push(hoist);
                        out.push(Stmt::Let(names.clone(), v, *c, *l));
                    }
                    None => out.push(s.clone()),
                },
                Stmt::Assign(t, op, e, l) => match self.value(e) {
                    Some((hoist, v)) => {
                        out.push(hoist);
                        out.push(Stmt::Assign(t.clone(), op.clone(), v, *l));
                    }
                    None => out.push(s.clone()),
                },
                Stmt::Return(Some(e), l) => match self.value(e) {
                    Some((hoist, v)) => {
                        out.push(hoist);
                        out.push(Stmt::Return(Some(v), *l));
                    }
                    None => out.push(s.clone()),
                },
                Stmt::Fn(d) => out.push(Stmt::Fn(self.func(d))),
                Stmt::Struct(sd) => {
                    let mut n = (**sd).clone();
                    n.methods = sd.methods.iter().map(|m| self.func(m)).collect();
                    out.push(Stmt::Struct(Rc::new(n)));
                }
                Stmt::Machine(m) => {
                    let mut n = (**m).clone();
                    let tr = |me: &mut Self, t: &mut Transition| {
                        if let Some(a) = &t.action {
                            t.action = Some(me.block(a));
                        }
                    };
                    for st in &mut n.states {
                        st.enter = st.enter.as_ref().map(|b| self.block(b));
                        st.exit = st.exit.as_ref().map(|b| self.block(b));
                        for t in &mut st.transitions {
                            tr(self, t);
                        }
                    }
                    for t in &mut n.global {
                        tr(self, t);
                    }
                    out.push(Stmt::Machine(Rc::new(n)));
                }
                Stmt::If(c, a, b) => out.push(Stmt::If(
                    c.clone(),
                    self.block(a),
                    b.as_ref().map(|b| self.block(b)),
                )),
                Stmt::While(c, i, b, l) => {
                    out.push(Stmt::While(c.clone(), i.clone(), self.block(b), *l))
                }
                Stmt::Loop(b) => out.push(Stmt::Loop(self.block(b))),
                Stmt::For(n, it, b, l) => {
                    out.push(Stmt::For(n.clone(), it.clone(), self.block(b), *l))
                }
                Stmt::Always(c, h, l) => out.push(Stmt::Always(
                    c.clone(),
                    h.as_ref().map(|b| self.block(b)),
                    *l,
                )),
                other => out.push(other.clone()),
            }
        }
        out
    }
}

pub fn desugar_matches(prog: &[Stmt]) -> Vec<Stmt> {
    Lower { n: 0 }.block(prog)
}
