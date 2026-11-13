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
            if self.eat_sym(":") {
                self.ident()?; // optional type annotation, ignored
            }
            let default = if self.eat_sym("=") {
                Some(self.expr()?)
            } else {
                None
            };
            ps.push(Param { name, default });
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(ps)
    }

    fn fn_rest(&mut self, name: String) -> PResult<FnDef> {
        let params = self.params()?;
        if self.eat_sym("->") {
            self.ident()?; // optional return annotation, ignored
        }
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
        })
    }

    // ---------------------------------------------------------------- expressions

    pub fn expr(&mut self) -> PResult<Expr> {
        self.or_expr()
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
    for t in toks {
        let text = match &t.tok {
            Tok::Str(s) => format!("{s:?}"),
            other => other.to_string(),
        };
        let no_space_before = matches!(&t.tok, Tok::Sym(")" | "]" | "," | "." | "::"))
            || (matches!(&t.tok, Tok::Sym("(" | "["))
                && matches!(prev, Some(Tok::Ident(_)) | Some(Tok::Sym(")" | "]"))));
        let no_space_after_prev =
            matches!(prev, None | Some(Tok::Sym("(" | "[" | "." | "::" | "!")));
        if !no_space_before && !no_space_after_prev {
            out.push(' ');
        }
        out.push_str(&text);
        prev = Some(&t.tok);
    }
    out
}
