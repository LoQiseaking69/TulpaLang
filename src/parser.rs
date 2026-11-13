//! Recursive-descent / Pratt parser producing the Tulpa AST.

use crate::ast::*;
use crate::lexer::{lex, Tok, Token};
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
}

type PResult<T> = Result<T, ParseError>;

pub fn parse(src: &str) -> PResult<Vec<Stmt>> {
    let tokens = lex(src).map_err(|e| ParseError {
        msg: e.msg,
        line: e.line,
        col: e.col,
    })?;
    let mut p = Parser {
        toks: tokens,
        pos: 0,
    };
    let mut out = Vec::new();
    p.skip_semis();
    while !p.at_eof() {
        out.push(p.statement()?);
        p.skip_semis();
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }
    fn tok(&self) -> &Token {
        &self.toks[self.pos]
    }
    fn line(&self) -> usize {
        self.toks[self.pos].line
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn is_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(x) if *x == s)
    }
    fn is_kw(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Kw(x) if *x == s)
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if self.is_sym(s) {
            self.next();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, s: &str) -> bool {
        if self.is_kw(s) {
            self.next();
            true
        } else {
            false
        }
    }
    fn err<T>(&self, msg: impl Into<String>) -> PResult<T> {
        let t = self.tok();
        Err(ParseError {
            msg: msg.into(),
            line: t.line,
            col: t.col,
        })
    }
    fn expect_sym(&mut self, s: &str) -> PResult<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            let found = self.peek().clone();
            self.err(format!("expected '{s}', found '{found}'"))
        }
    }
    fn ident(&mut self) -> PResult<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                Ok(s)
            }
            t => self.err(format!("expected identifier, found '{t}'")),
        }
    }
    /// True when the token just consumed is directly followed by `[` (no whitespace).
    fn unit_follows(&self) -> bool {
        if !self.is_sym("[") || self.tok().nl_before {
            return false;
        }
        let prev = &self.toks[self.pos - 1];
        prev.line == self.tok().line && prev.end == self.tok().col
    }

    fn skip_semis(&mut self) {
        while self.eat_sym(";") {}
    }

    // ---------------------------------------------------------------- statements

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect_sym("{")?;
        let mut body = Vec::new();
        self.skip_semis();
        while !self.is_sym("}") {
            if self.at_eof() {
                return self.err("unexpected end of file; missing '}'");
            }
            body.push(self.statement()?);
            self.skip_semis();
        }
        self.next();
        Ok(body)
    }

    fn statement(&mut self) -> PResult<Stmt> {
        let line = self.line();
        match self.peek().clone() {
            Tok::Kw("let") | Tok::Kw("const") => {
                let is_const = matches!(self.next(), Tok::Kw("const"));
                self.eat_kw("mut");
                let mut names = vec![self.ident()?];
                while self.eat_sym(",") {
                    names.push(self.ident()?);
                }
                self.expect_sym("=")?;
                let e = self.expr()?;
                Ok(Stmt::Let(names, e, is_const, line))
            }
            Tok::Kw("fn") | Tok::Kw("def") if matches!(self.peek_at(1), Tok::Ident(_)) => {
                self.next();
                let name = self.ident()?;
                Ok(Stmt::Fn(Rc::new(self.fn_rest(name)?)))
            }
            Tok::Kw("return") => {
                self.next();
                let has_value = !self.tok().nl_before
                    && !self.is_sym("}")
                    && !self.is_sym(";")
                    && !self.at_eof();
                let e = if has_value { Some(self.expr()?) } else { None };
                Ok(Stmt::Return(e, line))
            }
            Tok::Kw("if") => {
                self.next();
                self.if_rest()
            }
            Tok::Kw("while") => {
                self.next();
                let cond = self.expr()?;
                let mut invs = Vec::new();
                while self.eat_kw("invariant") {
                    invs.push(self.contract()?);
                }
                let body = self.block()?;
                Ok(Stmt::While(cond, invs, body, line))
            }
            Tok::Kw("loop") => {
                self.next();
                Ok(Stmt::Loop(self.block()?))
            }
            Tok::Kw("for") => {
                self.next();
                let mut names = vec![self.ident()?];
                while self.eat_sym(",") {
                    names.push(self.ident()?);
                }
                if !self.eat_kw("in") {
                    return self.err("expected 'in' in for loop");
                }
                let it = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::For(names, it, body, line))
            }
            Tok::Kw("break") => {
                self.next();
                Ok(Stmt::Break)
            }
            Tok::Kw("continue") => {
                self.next();
                Ok(Stmt::Continue)
            }
            Tok::Kw("struct") => {
                self.next();
                Ok(Stmt::Struct(Rc::new(self.struct_rest()?)))
            }
            Tok::Kw("machine") => {
                self.next();
                Ok(Stmt::Machine(Rc::new(self.machine_rest()?)))
            }
            Tok::Kw("when") => {
                self.next();
                let cond = self.expr()?;
                let hold = if self.eat_kw("for") {
                    Some(self.expr()?)
                } else {
                    None
                };
                let body = self.block()?;
                let els = if self.eat_kw("else") {
                    Some(self.block()?)
                } else {
                    None
                };
                Ok(Stmt::When(cond, hold, body, els, line))
            }
            Tok::Kw("always") => {
                self.next();
                let c = self.contract()?;
                let handler = if self.eat_kw("else") {
                    Some(self.block()?)
                } else {
                    None
                };
                Ok(Stmt::Always(c, handler, line))
            }
            Tok::Ident(ref a)
                if a == "after"
                    && !matches!(
                        self.peek_at(1),
                        Tok::Sym("=" | "+=" | "-=" | "." | "(" | "[" | ",")
                    )
                    && !self.peek_at(1).eq(&Tok::Eof) =>
            {
                self.next();
                let trigger = self.expr()?;
                if !self.is_ident("within") {
                    return self.err("expected 'within <duration>' after the trigger");
                }
                self.next();
                let within = self.expr()?;
                if !self.eat_kw("ensure") {
                    return self.err("expected 'ensure <response>' after the deadline");
                }
                let response = self.contract()?;
                let handler = if self.eat_kw("else") {
                    Some(self.block()?)
                } else {
                    None
                };
                Ok(Stmt::Deadline(trigger, within, response, handler, line))
            }
            Tok::Kw("import") => {
                self.next();
                let path = match self.next() {
                    Tok::Str(p) => p,
                    t => {
                        return self.err(format!(
                            "expected a file path string after import, found '{t}'"
                        ))
                    }
                };
                let alias = if matches!(self.peek(), Tok::Ident(a) if a == "as") {
                    self.next();
                    Some(self.ident()?)
                } else {
                    None
                };
                Ok(Stmt::Import(path, alias, vec![], line))
            }
            Tok::Ident(ref f) if f == "from" && matches!(self.peek_at(1), Tok::Str(_)) => {
                self.next();
                let path = match self.next() {
                    Tok::Str(p) => p,
                    _ => unreachable!(),
                };
                if !self.eat_kw("import") {
                    return self.err("expected 'import' after from \"...\"");
                }
                let mut names = vec![self.ident()?];
                while self.eat_sym(",") {
                    names.push(self.ident()?);
                }
                Ok(Stmt::Import(path, None, names, line))
            }
            Tok::Kw("use") => {
                self.next();
                let mut path = vec![self.ident()?];
                while self.eat_sym("::") {
                    if self.eat_sym("*") {
                        path.push("*".into());
                        break;
                    }
                    path.push(self.ident()?);
                }
                Ok(Stmt::Use(path))
            }
            Tok::Kw("assert") => {
                self.next();
                let c = self.contract()?;
                let msg = if self.eat_sym(",") {
                    Some(self.expr()?)
                } else {
                    None
                };
                Ok(Stmt::Assert(c.expr, msg, c.text, line))
            }
            Tok::Kw("try") => {
                self.next();
                let body = self.block()?;
                if !self.eat_kw("catch") {
                    return self.err("expected 'catch' after try block");
                }
                let name = if let Tok::Ident(n) = self.peek().clone() {
                    self.next();
                    Some(n)
                } else {
                    None
                };
                let handler = self.block()?;
                Ok(Stmt::Try(body, name, handler))
            }
            Tok::Ident(ref n)
                if n == "define_macro" && matches!(self.peek_at(1), Tok::Sym("!")) =>
            {
                self.next();
                self.next();
                self.expect_sym("(")?;
                let name = self.ident()?;
                self.expect_sym(",")?;
                self.expect_sym("(")?;
                let mut params = Vec::new();
                while !self.is_sym(")") {
                    params.push(self.ident()?);
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym(")")?;
                let body = self.block()?;
                self.expect_sym(")")?;
                Ok(Stmt::MacroDef(name, params, body))
            }
            _ => {
                let e = self.expr()?;
                let op = match self.peek() {
                    Tok::Sym("=") => Some(None),
                    Tok::Sym(s @ ("+=" | "-=" | "*=" | "/=" | "%=")) => {
                        Some(Some(s[..1].to_string()))
                    }
                    _ => None,
                };
                if let Some(op) = op {
                    self.next();
                    let target = match e {
                        Expr::Ident(n, _) => Target::Name(n),
                        Expr::Index(o, i, _) => Target::Index(*o, *i),
                        Expr::Field(o, f, _) => Target::Field(*o, f),
                        _ => return self.err("invalid assignment target"),
                    };
                    let v = self.expr()?;
                    return Ok(Stmt::Assign(target, op, v, line));
                }
                Ok(Stmt::Expr(e, line))
            }
        }
    }

    fn is_ident(&self, name: &str) -> bool {
        matches!(self.peek(), Tok::Ident(n) if n == name)
    }

    fn struct_rest(&mut self) -> PResult<StructDef> {
        let name = self.ident()?;
        self.expect_sym("{")?;
        let (mut fields, mut methods, mut invariants) = (Vec::new(), Vec::new(), Vec::new());
        loop {
            while self.eat_sym(";") || self.eat_sym(",") {}
            if self.eat_sym("}") {
                break;
            }
            if self.at_eof() {
                return self.err("unexpected end of file in struct; missing '}'");
            }
            if self.is_kw("fn") || self.is_kw("def") {
                self.next();
                let m = self.ident()?;
                methods.push(Rc::new(self.fn_rest(m)?));
            } else if self.eat_kw("invariant") {
                invariants.push(self.contract()?);
            } else {
                let fname = self.ident()?;
                let ty = if self.eat_sym(":") {
                    Some(self.type_ann()?)
                } else {
                    None
                };
                let default = if self.eat_sym("=") {
                    Some(self.expr()?)
                } else {
                    None
                };
                fields.push(Param {
                    name: fname,
                    default,
                    ty,
                });
            }
        }
        Ok(StructDef {
            name,
            fields,
            methods,
            invariants,
        })
    }

    fn transition(&mut self) -> PResult<Transition> {
        let line = self.line();
        // `on` already consumed
        let event = self.ident()?;
        let mut params = Vec::new();
        if self.eat_sym("(") {
            while !self.is_sym(")") {
                params.push(self.ident()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.expect_sym(")")?;
        }
        let guard = if self.eat_kw("if") {
            Some(self.expr()?)
        } else {
            None
        };
        let target = if self.eat_sym("=>") {
            Some(self.ident()?)
        } else {
            None
        };
        let action = if self.is_sym("{") && !self.tok().nl_before
            || (target.is_none() && self.is_sym("{"))
        {
            Some(self.block()?)
        } else {
            None
        };
        if target.is_none() && action.is_none() {
            return self.err("transition needs '=> State' and/or an action block");
        }
        Ok(Transition {
            event,
            params,
            guard,
            target,
            action,
            line,
        })
    }

    fn machine_rest(&mut self) -> PResult<MachineDef> {
        let name = self.ident()?;
        let params = if self.is_sym("(") {
            self.params()?
        } else {
            vec![]
        };
        self.expect_sym("{")?;
        let (mut states, mut global) = (Vec::new(), Vec::new());
        loop {
            self.skip_semis();
            if self.eat_sym("}") {
                break;
            }
            if self.at_eof() {
                return self.err("unexpected end of file in machine; missing '}'");
            }
            if self.is_ident("on") {
                self.next();
                global.push(self.transition()?);
            } else if self.is_ident("state") {
                let line = self.line();
                self.next();
                let sname = self.ident()?;
                self.expect_sym("{")?;
                let (mut enter, mut exit, mut transitions) = (None, None, Vec::new());
                loop {
                    self.skip_semis();
                    if self.eat_sym("}") {
                        break;
                    }
                    if self.is_ident("enter") {
                        self.next();
                        enter = Some(self.block()?);
                    } else if self.is_ident("exit") {
                        self.next();
                        exit = Some(self.block()?);
                    } else if self.is_ident("on") {
                        self.next();
                        transitions.push(self.transition()?);
                    } else {
                        let t = self.peek().clone();
                        return self.err(format!(
                            "expected 'enter', 'exit' or 'on' in state, found '{t}'"
                        ));
                    }
                }
                states.push(StateDef {
                    name: sname,
                    enter,
                    exit,
                    transitions,
                    line,
                });
            } else {
                let t = self.peek().clone();
                return self.err(format!("expected 'state' or 'on' in machine, found '{t}'"));
            }
        }
        if states.is_empty() {
            return self.err(format!("machine {name} has no states"));
        }
        let known: Vec<&str> = states.iter().map(|s| s.name.as_str()).collect();
        for t in states
            .iter()
            .flat_map(|s| s.transitions.iter())
            .chain(global.iter())
        {
            if let Some(tg) = &t.target {
                if !known.contains(&tg.as_str()) {
                    return Err(ParseError {
                        msg: format!(
                            "machine {name}: transition '{}' targets unknown state '{tg}'",
                            t.event
                        ),
                        line: t.line,
                        col: 1,
                    });
                }
            }
        }
        Ok(MachineDef {
            name,
            params,
            states,
            global,
        })
    }

    fn contract(&mut self) -> PResult<Contract> {
        let start = self.pos;
        let expr = self.expr()?;
        let text = render_tokens(&self.toks[start..self.pos]);
        Ok(Contract { expr, text })
    }

    fn if_rest(&mut self) -> PResult<Stmt> {
        let cond = self.expr()?;
        let body = self.block()?;
        let els = if self.eat_kw("elif")
            || (self.is_kw("else") && matches!(self.peek_at(1), Tok::Kw("if")))
        {
            if self.is_kw("else") {
                self.next();
                self.next();
            }
            Some(vec![self.if_rest()?])
        } else if self.eat_kw("else") {
            Some(self.block()?)
        } else {
            None
        };
        Ok(Stmt::If(cond, body, els))
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect_sym("(")?;
        let mut ps = Vec::new();
        while !self.is_sym(")") {
            self.eat_kw("mut");
            let name = self.ident()?;
            let ty = if self.eat_sym(":") {
                Some(self.type_ann()?)
            } else {
                None
            };
            let default = if self.eat_sym("=") {
                Some(self.expr()?)
            } else {
                None
            };
            ps.push(Param { name, default, ty });
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(ps)
    }

    /// `int`, `str?`, `Motor`, `[m/s]`, `[deg]?`
    fn type_ann(&mut self) -> PResult<TypeAnn> {
        if self.is_sym("[") {
            let u = self.unit_bracket()?;
            let opt = self.eat_sym("?");
            return Ok(TypeAnn::Unit(u, opt));
        }
        let name = match self.next() {
            Tok::Ident(n) => n,
            Tok::Kw("fn") => "fn".into(),
            Tok::Kw("nil") => "nil".into(),
            t => return self.err(format!("expected a type, found '{t}'")),
        };
        let opt = self.eat_sym("?");
        Ok(TypeAnn::Name(name, opt))
    }

    /// Parses `[unit expr]` starting at `[`.
    fn unit_bracket(&mut self) -> PResult<Rc<crate::units::Unit>> {
        self.expect_sym("[")?;
        let mut text = String::new();
        while !self.is_sym("]") {
            if self.at_eof() {
                return self.err("unterminated unit, expected ']'");
            }
            let t = self.next();
            text.push_str(&t.to_string());
        }
        self.next();
        crate::units::parse(&text)
            .map(Rc::new)
            .or_else(|m| self.err(m))
    }

    fn fn_rest(&mut self, name: String) -> PResult<FnDef> {
        let params = self.params()?;
        let ret = if self.eat_sym("->") {
            Some(self.type_ann()?)
        } else {
            None
        };
        let (mut requires, mut ensures) = (Vec::new(), Vec::new());
        loop {
            if self.eat_kw("require") {
                requires.push(self.contract()?);
            } else if self.eat_kw("ensure") {
                ensures.push(self.contract()?);
            } else {
                break;
            }
        }
        if self.eat_sym("=>") {
            let line = self.line();
            let e = self.expr()?;
            return Ok(FnDef {
                name,
                params,
                requires,
                ensures,
                body: vec![Stmt::Return(Some(e), line)],
                is_expr: true,
                ret,
            });
        }
        let body = self.block()?;
        Ok(FnDef {
            name,
            params,
            requires,
            ensures,
            body,
            is_expr: false,
            ret,
        })
    }

    // ---------------------------------------------------------------- expressions

    pub fn expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.or_expr()?;
        // `a |> f(b)` == `f(a, b)`;  `a |> f` == `f(a)`;  `a |> obj.m(b)` == `obj.m(a, b)`
        while self.is_sym("|>") {
            let line = self.line();
            self.next();
            let rhs = self.or_expr()?;
            lhs = match rhs {
                Expr::Call(f, mut args, l) => {
                    args.insert(0, lhs);
                    Expr::Call(f, args, l)
                }
                Expr::MethodCall(o, m, mut args, l) => {
                    args.insert(0, lhs);
                    Expr::MethodCall(o, m, args, l)
                }
                Expr::MacroCall(n, mut args, l) => {
                    args.insert(0, MacroArg::Expr(lhs));
                    Expr::MacroCall(n, args, l)
                }
                Expr::Field(o, m, l) => Expr::MethodCall(o, m, vec![lhs], l),
                other => Expr::Call(Box::new(other), vec![lhs], line),
            };
        }
        Ok(lhs)
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let mut l = self.and_expr()?;
        while self.eat_kw("or") || self.eat_sym("||") {
            let r = self.and_expr()?;
            l = Expr::Logic(Box::new(l), false, Box::new(r));
        }
        Ok(l)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let mut l = self.not_expr()?;
        while self.eat_kw("and") || self.eat_sym("&&") {
            let r = self.not_expr()?;
            l = Expr::Logic(Box::new(l), true, Box::new(r));
        }
        Ok(l)
    }

    fn not_expr(&mut self) -> PResult<Expr> {
        let line = self.line();
        if self.eat_kw("not") {
            let e = self.not_expr()?;
            return Ok(Expr::Unary("!".into(), Box::new(e), line));
        }
        self.cmp_expr()
    }

    fn cmp_expr(&mut self) -> PResult<Expr> {
        let mut l = self.range_expr()?;
        loop {
            let line = self.line();
            let op = match self.peek() {
                Tok::Sym(s @ ("==" | "!=" | "<" | "<=" | ">" | ">=")) => s.to_string(),
                Tok::Kw("in") => "in".to_string(),
                Tok::Kw("not") if matches!(self.peek_at(1), Tok::Kw("in")) => {
                    self.next();
                    "not in".to_string()
                }
                _ => break,
            };
            self.next();
            let r = self.range_expr()?;
            l = Expr::Binary(Box::new(l), op, Box::new(r), line);
        }
        Ok(l)
    }

    fn range_expr(&mut self) -> PResult<Expr> {
        let l = self.bin_expr(0)?;
        if self.is_sym("..") || self.is_sym("..=") {
            let incl = matches!(self.next(), Tok::Sym("..="));
            let r = self.bin_expr(0)?;
            return Ok(Expr::Range(Box::new(l), Box::new(r), incl));
        }
        Ok(l)
    }

    fn bin_prec(t: &Tok) -> Option<(u8, &'static str)> {
        Some(match t {
            Tok::Sym("|") => (1, "|"),
            Tok::Sym("^") => (2, "^"),
            Tok::Sym("&") => (3, "&"),
            Tok::Sym("<<") => (4, "<<"),
            Tok::Sym(">>") => (4, ">>"),
            Tok::Sym("+") => (5, "+"),
            Tok::Sym("-") => (5, "-"),
            Tok::Sym("*") => (6, "*"),
            Tok::Sym("/") => (6, "/"),
            Tok::Sym("%") => (6, "%"),
            _ => return None,
        })
    }

    fn bin_expr(&mut self, min: u8) -> PResult<Expr> {
        let mut l = self.unary()?;
        while let Some((prec, op)) = Self::bin_prec(self.peek()) {
            if prec < min {
                break;
            }
            let line = self.line();
            self.next();
            let r = self.bin_expr(prec + 1)?;
            l = Expr::Binary(Box::new(l), op.into(), Box::new(r), line);
        }
        Ok(l)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let line = self.line();
        for op in ["-", "!", "~"] {
            if self.eat_sym(op) {
                let e = self.unary()?;
                return Ok(Expr::Unary(op.into(), Box::new(e), line));
            }
        }
        if self.eat_kw("await") {
            let e = self.unary()?;
            return Ok(Expr::Await(Box::new(e), line));
        }
        if self.eat_kw("spawn") {
            let e = self.unary()?;
            return Ok(Expr::Spawn(Box::new(e), line));
        }
        self.power()
    }

    fn power(&mut self) -> PResult<Expr> {
        let base = self.postfix()?;
        if self.is_sym("**") {
            let line = self.line();
            self.next();
            let exp = self.unary()?; // right associative
            return Ok(Expr::Binary(
                Box::new(base),
                "**".into(),
                Box::new(exp),
                line,
            ));
        }
        Ok(base)
    }

    fn args(&mut self) -> PResult<Vec<Expr>> {
        let mut a = Vec::new();
        while !self.is_sym(")") {
            if let (Tok::Ident(n), Tok::Sym(":")) = (self.peek().clone(), self.peek_at(1).clone()) {
                if !matches!(self.peek_at(2), Tok::Sym(":")) {
                    self.next();
                    self.next();
                    let v = self.expr()?;
                    a.push(Expr::Named(n, Box::new(v)));
                    if !self.eat_sym(",") {
                        break;
                    }
                    continue;
                }
            }
            a.push(self.expr()?);
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(a)
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            let line = self.line();
            let nl = self.tok().nl_before;
            if !nl && self.eat_sym("(") {
                let a = self.args()?;
                e = Expr::Call(Box::new(e), a, line);
            } else if !nl && self.eat_sym("[") {
                let i = self.expr()?;
                self.expect_sym("]")?;
                e = Expr::Index(Box::new(e), Box::new(i), line);
            } else if self.is_sym(".") {
                self.next();
                let name = match self.next() {
                    Tok::Ident(n) => n,
                    Tok::Int(n) => n.to_string(),
                    Tok::Kw(k) => k.to_string(), // `.in(...)`, `.loop`, ...
                    t => return self.err(format!("expected field name, found '{t}'")),
                };
                if !self.tok().nl_before && self.eat_sym("(") {
                    let a = self.args()?;
                    e = Expr::MethodCall(Box::new(e), name, a, line);
                } else {
                    e = Expr::Field(Box::new(e), name, line);
                }
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let line = self.line();
        match self.next() {
            Tok::Int(n) if self.unit_follows() => {
                let u = self.unit_bracket()?;
                Ok(Expr::Quantity(n as f64, u))
            }
            Tok::Float(n) if self.unit_follows() => {
                let u = self.unit_bracket()?;
                Ok(Expr::Quantity(n, u))
            }
            Tok::Int(n) => Ok(Expr::Int(n)),
            Tok::Float(n) => Ok(Expr::Float(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::FStr(s) => self.fstring(&s),
            Tok::Kw("true") => Ok(Expr::Bool(true)),
            Tok::Kw("false") => Ok(Expr::Bool(false)),
            Tok::Kw("nil") => Ok(Expr::Nil),
            Tok::Ident(name) => {
                if self.is_sym("!")
                    && !self.tok().nl_before
                    && matches!(self.peek_at(1), Tok::Sym("("))
                {
                    self.next();
                    self.next();
                    let mut args = Vec::new();
                    while !self.is_sym(")") {
                        if self.is_sym("{") {
                            args.push(MacroArg::Block(self.block()?));
                        } else {
                            args.push(MacroArg::Expr(self.expr()?));
                        }
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                    return Ok(Expr::MacroCall(name, args, line));
                }
                if self.is_sym("::") {
                    self.next();
                    let member = self.ident()?;
                    return Ok(Expr::Path(name, member, line));
                }
                Ok(Expr::Ident(name, line))
            }
            Tok::Sym("(") => {
                let e = self.expr()?;
                if self.eat_sym(",") {
                    // tuple → list
                    let mut items = vec![e];
                    while !self.is_sym(")") {
                        items.push(self.expr()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                    return Ok(Expr::List(items));
                }
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Sym("[") => {
                let mut items = Vec::new();
                if self.eat_sym("]") {
                    return Ok(Expr::List(items));
                }
                let first = self.expr()?;
                if self.eat_sym(";") {
                    let n = self.expr()?;
                    self.expect_sym("]")?;
                    return Ok(Expr::Repeat(Box::new(first), Box::new(n)));
                }
                items.push(first);
                while self.eat_sym(",") {
                    if self.is_sym("]") {
                        break;
                    }
                    items.push(self.expr()?);
                }
                self.expect_sym("]")?;
                Ok(Expr::List(items))
            }
            Tok::Sym("{") => {
                let mut pairs = Vec::new();
                while !self.is_sym("}") {
                    let k = match self.peek().clone() {
                        Tok::Ident(n) if matches!(self.peek_at(1), Tok::Sym(":")) => {
                            self.next();
                            Expr::Str(n)
                        }
                        _ => self.expr()?,
                    };
                    self.expect_sym(":")?;
                    let v = self.expr()?;
                    pairs.push((k, v));
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym("}")?;
                Ok(Expr::Dict(pairs))
            }
            Tok::Kw("fn") | Tok::Kw("def") => {
                let f = self.fn_rest("<lambda>".into())?;
                Ok(Expr::Lambda(Rc::new(f)))
            }
            Tok::Sym("|") | Tok::Sym("||") => {
                let mut params = Vec::new();
                if matches!(self.toks[self.pos - 1].tok, Tok::Sym("|")) {
                    while !self.is_sym("|") {
                        params.push(Param {
                            name: self.ident()?,
                            default: None,
                            ty: None,
                        });
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym("|")?;
                }
                let l = self.line();
                let body = if self.is_sym("{") {
                    self.block()?
                } else {
                    vec![Stmt::Return(Some(self.expr()?), l)]
                };
                Ok(Expr::Lambda(Rc::new(FnDef {
                    name: "<lambda>".into(),
                    params,
                    requires: vec![],
                    ensures: vec![],
                    body,
                    is_expr: true,
                    ret: None,
                })))
            }
            Tok::Kw("if") => {
                let c = self.expr()?;
                let a = self.block()?;
                let b = if self.eat_kw("else") {
                    if self.is_kw("if") {
                        let l = self.line();
                        Some(vec![Stmt::Expr(self.primary()?, l)])
                    } else {
                        Some(self.block()?)
                    }
                } else {
                    None
                };
                Ok(Expr::IfExpr(Box::new(c), a, b))
            }
            Tok::Kw("match") => self.match_rest(line),
            t => {
                self.pos -= 1;
                self.err(format!("unexpected '{t}'"))
            }
        }
    }

    fn match_rest(&mut self, line: usize) -> PResult<Expr> {
        let subject = self.expr()?;
        self.expect_sym("{")?;
        let mut arms = Vec::new();
        while !self.is_sym("}") {
            let mut patterns = vec![self.pattern()?];
            while self.eat_sym("|") {
                patterns.push(self.pattern()?);
            }
            let guard = if self.eat_kw("if") {
                Some(self.expr()?)
            } else {
                None
            };
            self.expect_sym("=>")?;
            let body = if self.is_sym("{") {
                self.block()?
            } else {
                let l = self.line();
                vec![self.statement_in_arm(l)?]
            };
            arms.push(MatchArm {
                patterns,
                guard,
                body,
            });
            self.eat_sym(",");
            self.skip_semis();
        }
        self.next();
        Ok(Expr::Match(Box::new(subject), arms, line))
    }

    fn statement_in_arm(&mut self, l: usize) -> PResult<Stmt> {
        match self.peek() {
            Tok::Kw("return") | Tok::Kw("break") | Tok::Kw("continue") => self.statement(),
            _ => Ok(Stmt::Expr(self.expr()?, l)),
        }
    }

    fn pattern(&mut self) -> PResult<Pattern> {
        let lit = |p: &mut Self| -> PResult<Expr> {
            let neg = p.eat_sym("-");
            let e = match p.next() {
                Tok::Int(n) => Expr::Int(if neg { -n } else { n }),
                Tok::Float(n) => Expr::Float(if neg { -n } else { n }),
                Tok::Str(s) if !neg => Expr::Str(s),
                Tok::Kw("true") => Expr::Bool(true),
                Tok::Kw("false") => Expr::Bool(false),
                Tok::Kw("nil") => Expr::Nil,
                t => {
                    p.pos -= 1;
                    return p.err(format!("invalid pattern '{t}'"));
                }
            };
            Ok(e)
        };
        match self.peek().clone() {
            Tok::Ident(n) if n == "_" => {
                self.next();
                Ok(Pattern::Wildcard)
            }
            Tok::Ident(n) => {
                self.next();
                Ok(Pattern::Bind(n))
            }
            Tok::Sym("[") => {
                self.next();
                let mut items = Vec::new();
                let mut rest = None;
                while !self.is_sym("]") {
                    if self.eat_sym("..") {
                        rest = Some(match self.peek().clone() {
                            Tok::Ident(n) => {
                                self.next();
                                n
                            }
                            _ => "_".into(),
                        });
                    } else {
                        items.push(self.pattern()?);
                    }
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym("]")?;
                Ok(Pattern::List(items, rest))
            }
            _ => {
                let a = lit(self)?;
                if self.is_sym("..") || self.is_sym("..=") {
                    let incl = matches!(self.next(), Tok::Sym("..="));
                    let b = lit(self)?;
                    return Ok(Pattern::Range(a, b, incl));
                }
                Ok(Pattern::Literal(a))
            }
        }
    }

    fn fstring(&mut self, raw: &str) -> PResult<Expr> {
        let mut parts = Vec::new();
        let mut lit = String::new();
        let chars: Vec<char> = raw.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '{' && chars.get(i + 1) == Some(&'{') {
                lit.push('{');
                i += 2;
            } else if c == '}' && chars.get(i + 1) == Some(&'}') {
                lit.push('}');
                i += 2;
            } else if c == '{' {
                let mut depth = 1;
                let start = i + 1;
                i += 1;
                while i < chars.len() && depth > 0 {
                    match chars[i] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                if depth != 0 {
                    return self.err("unterminated '{' in f-string");
                }
                let inner: String = chars[start..i - 1].iter().collect();
                // Support Python-style format spec: {expr:.2f} / {expr:x}
                let (src, spec) = split_spec(&inner);
                let toks = lex(src).map_err(|e| ParseError {
                    msg: format!("in f-string: {}", e.msg),
                    line: self.line(),
                    col: 0,
                })?;
                let mut sub = Parser { toks, pos: 0 };
                let mut e = sub.expr()?;
                if !sub.at_eof() {
                    return self.err(format!("invalid expression in f-string: {{{inner}}}"));
                }
                if let Some(spec) = spec {
                    e = Expr::Call(
                        Box::new(Expr::Ident("format".into(), self.line())),
                        vec![e, Expr::Str(spec.to_string())],
                        self.line(),
                    );
                }
                if !lit.is_empty() {
                    parts.push(FPart::Lit(std::mem::take(&mut lit)));
                }
                parts.push(FPart::Expr(e));
            } else {
                lit.push(c);
                i += 1;
            }
        }
        if !lit.is_empty() {
            parts.push(FPart::Lit(lit));
        }
        Ok(Expr::FStr(parts))
    }
}

/// Splits `expr:spec` at a top-level colon that isn't part of `::`.
fn split_spec(s: &str) -> (&str, Option<&str>) {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    for i in 0..b.len() {
        let c = b[i];
        if let Some(q) = in_str {
            if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => in_str = Some(c),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b':' if depth == 0 => {
                let prev = i > 0 && b[i - 1] == b':';
                let next = i + 1 < b.len() && b[i + 1] == b':';
                if !prev && !next {
                    return (&s[..i], Some(&s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    (s, None)
}

/// Re-renders a token slice as compact, readable source (used in contract messages).
fn render_tokens(toks: &[Token]) -> String {
    let mut out = String::new();
    let mut prev: Option<&Tok> = None;
    let mut prev2: Option<&Tok> = None;
    for t in toks {
        let text = match &t.tok {
            Tok::Str(s) => format!("{s:?}"),
            other => other.to_string(),
        };
        let no_space_before = matches!(&t.tok, Tok::Sym(")" | "]" | "," | "." | "::"))
            || (matches!(&t.tok, Tok::Sym("(" | "["))
                && matches!(prev, Some(Tok::Ident(_)) | Some(Tok::Sym(")" | "]"))));
        let unary_minus = matches!(prev, Some(Tok::Sym("-")))
            && !matches!(
                prev2,
                Some(Tok::Ident(_) | Tok::Int(_) | Tok::Float(_) | Tok::Sym(")" | "]"))
            );
        let no_space_after_prev =
            unary_minus || matches!(prev, None | Some(Tok::Sym("(" | "[" | "." | "::" | "!")));
        if !no_space_before && !no_space_after_prev {
            out.push(' ');
        }
        out.push_str(&text);
        prev2 = prev;
        prev = Some(&t.tok);
    }
    out
}
