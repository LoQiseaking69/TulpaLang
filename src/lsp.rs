//! `tulpa lsp`: a Language Server Protocol implementation over stdio.
//!
//! Provides diagnostics (syntax errors, unit mismatches between constant quantities, names that
//! are never defined, with "did you mean" suggestions), hover (function signatures with types
//! and contracts, struct fields, machine states/events, unit dimensions), completion (keywords,
//! builtins, document symbols, unit names inside `[...]`, methods after `.`), document symbols
//! and go-to-definition.

use crate::ast::*;
use crate::lexer::{lex, Tok, Token, KEYWORDS};
use serde_json::{json, Value as J};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};

#[derive(Default)]
pub struct State {
    docs: HashMap<String, String>,
    pub shutdown: bool,
}

/// Handles one JSON-RPC message; returns the messages to send back (responses/notifications).
pub fn handle(st: &mut State, msg: &J) -> Vec<J> {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(J::Null);
    let reply = |result: J| json!({"jsonrpc": "2.0", "id": id.clone().unwrap_or(J::Null), "result": result});
    match method {
        "initialize" => vec![reply(json!({
            "capabilities": {
                "textDocumentSync": 1,
                "hoverProvider": true,
                "completionProvider": {"triggerCharacters": [".", "[", ":"]},
                "documentSymbolProvider": true,
                "definitionProvider": true
            },
            "serverInfo": {"name": "tulpa-lsp", "version": env!("CARGO_PKG_VERSION")}
        }))],
        "shutdown" => {
            st.shutdown = true;
            vec![reply(J::Null)]
        }
        "textDocument/didOpen" => {
            let uri = params["textDocument"]["uri"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let text = params["textDocument"]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            st.docs.insert(uri.clone(), text);
            vec![publish(st, &uri)]
        }
        "textDocument/didChange" => {
            let uri = params["textDocument"]["uri"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if let Some(text) = params["contentChanges"]
                .as_array()
                .and_then(|c| c.last())
                .and_then(|c| c["text"].as_str())
            {
                st.docs.insert(uri.clone(), text.to_string());
            }
            vec![publish(st, &uri)]
        }
        "textDocument/didClose" => {
            let uri = params["textDocument"]["uri"]
                .as_str()
                .unwrap_or("")
                .to_string();
            st.docs.remove(&uri);
            vec![
                json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": []}}),
            ]
        }
        "textDocument/hover" => {
            let (text, line, col) = position(st, &params);
            vec![reply(match hover(&text, line, col) {
                Some(md) => json!({"contents": {"kind": "markdown", "value": md}}),
                None => J::Null,
            })]
        }
        "textDocument/completion" => {
            let (text, line, col) = position(st, &params);
            vec![reply(json!(completion(&text, line, col)))]
        }
        "textDocument/documentSymbol" => {
            let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
            let text = st.docs.get(uri).cloned().unwrap_or_default();
            vec![reply(json!(symbols(&text)))]
        }
        "textDocument/definition" => {
            let uri = params["textDocument"]["uri"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let (text, line, col) = position(st, &params);
            vec![reply(match definition(&text, line, col) {
                Some((l, c, len)) => json!({"uri": uri, "range": range(l, c, len)}),
                None => J::Null,
            })]
        }
        _ => match id {
            // unknown request: answer so clients don't hang; notifications are ignored
            Some(_) if !method.is_empty() => vec![
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}}),
            ],
            _ => vec![],
        },
    }
}

fn position(st: &State, params: &J) -> (String, usize, usize) {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let text = st.docs.get(uri).cloned().unwrap_or_default();
    let line = params["position"]["line"].as_u64().unwrap_or(0) as usize + 1;
    let col = params["position"]["character"].as_u64().unwrap_or(0) as usize + 1;
    (text, line, col)
}

/// LSP range from 1-based line/col.
fn range(line: usize, col: usize, len: usize) -> J {
    let l = line.saturating_sub(1);
    let c = col.saturating_sub(1);
    json!({"start": {"line": l, "character": c}, "end": {"line": l, "character": c + len.max(1)}})
}

fn publish(st: &State, uri: &str) -> J {
    let text = st.docs.get(uri).cloned().unwrap_or_default();
    json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": diagnostics(&text)}})
}

// ---------------------------------------------------------------------------- diagnostics

pub fn diagnostics(text: &str) -> Vec<J> {
    let prog = match crate::parser::parse(text) {
        Ok(p) => p,
        Err(e) => {
            return vec![
                json!({"range": range(e.line, e.col, 1), "severity": 1, "source": "tulpa", "message": e.msg}),
            ];
        }
    };
    let mut out = Vec::new();
    let toks = lex(text).unwrap_or_default();
    // 1. names that are never defined anywhere
    let defined = defined_names(&toks);
    let known: HashSet<&str> = crate::builtins::global_names()
        .into_iter()
        .chain(crate::builtins::module_names().iter().copied())
        .collect();
    let mut reported = HashSet::new();
    for (i, t) in toks.iter().enumerate() {
        let Tok::Ident(n) = &t.tok else { continue };
        if n.ends_with('\'') || n == "_" || n == "self" {
            continue;
        }
        let prev = i.checked_sub(1).map(|k| &toks[k].tok);
        let next = toks.get(i + 1).map(|t| &t.tok);
        // skip field/method names, keyword args, module paths, unit names, macro names, contextual words
        if matches!(prev, Some(Tok::Sym("." | "::" | ":" | "->")))
            || matches!(next, Some(Tok::Sym(":" | "!" | "::")))
        {
            continue; // fields/methods, kwargs, paths, macro names, type annotations
        }
        // define_test!(name, ...) names a test, not a variable
        if i >= 3
            && matches!(&toks[i - 3].tok, Tok::Ident(m) if m == "define_test")
            && matches!(toks[i - 1].tok, Tok::Sym("("))
        {
            continue;
        }
        if inside_unit_bracket(&toks, i)
            || matches!(
                n.as_str(),
                "on" | "state" | "enter" | "exit" | "within" | "after" | "from" | "as"
            )
        {
            continue;
        }
        if defined.contains(n.as_str()) || known.contains(n.as_str()) || reported.contains(n) {
            continue;
        }
        reported.insert(n.clone());
        let pool: Vec<&str> = defined
            .iter()
            .map(|s| s.as_str())
            .chain(known.iter().copied())
            .collect();
        let hint = suggest(n, &pool)
            .map(|s| format!(" (did you mean '{s}'?)"))
            .unwrap_or_default();
        out.push(json!({"range": range(t.line, t.col, n.chars().count()), "severity": 2, "source": "tulpa",
            "message": format!("'{n}' is never defined{hint}")}));
    }
    // 2. unit mismatches between constant expressions, e.g. `2[m] + 3[s]`
    let mut lines = Vec::new();
    walk_stmts(&prog, &mut |e| check_const_units(e, &mut lines));
    for (line, msg) in lines {
        out.push(json!({"range": range(line, 1, line_len(text, line)), "severity": 1, "source": "tulpa", "message": msg}));
    }
    out
}

fn line_len(text: &str, line: usize) -> usize {
    text.lines()
        .nth(line.saturating_sub(1))
        .map(|l| l.chars().count())
        .unwrap_or(1)
}

fn inside_unit_bracket(toks: &[Token], i: usize) -> bool {
    // walk back to an unmatched '[' that directly follows a number token
    let mut k = i;
    while k > 0 {
        k -= 1;
        match &toks[k].tok {
            Tok::Sym("]") | Tok::Sym("(") | Tok::Sym(")") | Tok::Sym(",") => return false,
            Tok::Sym("[") => {
                return k > 0
                    && matches!(toks[k - 1].tok, Tok::Int(_) | Tok::Float(_))
                    && toks[k - 1].end == toks[k].col
                    || k > 0 && matches!(toks[k - 1].tok, Tok::Sym(":") | Tok::Sym("->"));
            }
            _ => {}
        }
    }
    false
}

/// Every name bound somewhere in the file (declarations, params, loop vars, patterns, ...).
fn defined_names(toks: &[Token]) -> HashSet<String> {
    let mut d = HashSet::new();
    for (i, t) in toks.iter().enumerate() {
        let next_ident = |k: usize| match toks.get(k).map(|t| &t.tok) {
            Some(Tok::Ident(n)) => Some(n.clone()),
            _ => None,
        };
        match &t.tok {
            Tok::Kw("let" | "const" | "fn" | "def" | "struct" | "machine" | "for" | "catch") => {
                let mut k = i + 1;
                if matches!(toks.get(k).map(|t| &t.tok), Some(Tok::Kw("mut"))) {
                    k += 1;
                }
                while let Some(n) = next_ident(k) {
                    d.insert(n);
                    if matches!(toks.get(k + 1).map(|t| &t.tok), Some(Tok::Sym(","))) {
                        k += 2;
                    } else {
                        break;
                    }
                }
            }
            Tok::Kw("import") => {
                if let Some(Tok::Str(p)) = toks.get(i + 1).map(|t| &t.tok) {
                    let stem = std::path::Path::new(p)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    d.insert(stem);
                }
                // `as alias` or `from "x" import a, b`
                let mut k = i + 1;
                while let Some(tok) = toks.get(k) {
                    match &tok.tok {
                        Tok::Ident(n) if n != "as" => {
                            d.insert(n.clone());
                        }
                        Tok::Ident(_) | Tok::Str(_) | Tok::Sym(",") => {}
                        _ => break,
                    }
                    if tok.nl_before && k > i + 1 {
                        break;
                    }
                    k += 1;
                }
            }
            _ => {}
        }
        // state names, struct fields, match-arm bindings: `Ident =>` / `[a, ..rest]`
        if let Tok::Ident(n) = &t.tok {
            if matches!(
                toks.get(i + 1).map(|t| &t.tok),
                Some(Tok::Sym("=>")) | Some(Tok::Kw("if"))
            ) {
                d.insert(n.clone());
            }
            if matches!(
                toks.get(i.wrapping_sub(1)).map(|t| &t.tok),
                Some(Tok::Sym(".."))
            ) {
                d.insert(n.clone());
            }
            // pattern / destructuring elements `[a, ..rest]`, bare struct fields on their own line
            if matches!(
                toks.get(i.wrapping_sub(1)).map(|t| &t.tok),
                Some(Tok::Sym("[" | ","))
            ) && matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Sym("," | "]")))
            {
                d.insert(n.clone());
            }
            if t.nl_before
                && toks
                    .get(i + 1)
                    .is_none_or(|nx| nx.nl_before || matches!(nx.tok, Tok::Sym(",")))
            {
                d.insert(n.clone());
            }
            if i > 0 && matches!(&toks[i - 1].tok, Tok::Ident(s) if s == "state" || s == "on") {
                d.insert(n.clone());
            }
            if t.nl_before
                && matches!(
                    toks.get(i + 1).map(|t| &t.tok),
                    Some(Tok::Sym(":")) | Some(Tok::Sym("="))
                )
            {
                d.insert(n.clone()); // struct field / reassignment target at line start
            }
            if matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Sym(",")))
                && matches!(
                    toks.get(i.wrapping_sub(1)).map(|t| &t.tok),
                    Some(Tok::Kw("let" | "for"))
                )
            {
                d.insert(n.clone());
            }
        }
    }
    // parameter lists: fn/def NAME(...), fn(...) lambdas, machine NAME(...), `on ev(...)`, |a, b|
    for (i, t) in toks.iter().enumerate() {
        let opens_params = match &t.tok {
            Tok::Kw("fn" | "def" | "machine") => true,
            Tok::Ident(s) if s == "on" => true,
            Tok::Ident(_) => {
                matches!(toks.get(i.wrapping_sub(1)).map(|t| &t.tok), Some(Tok::Ident(s)) if s == "on")
            }
            _ => false,
        };
        if opens_params {
            // find the '(' that follows (skipping the name)
            let mut k = i + 1;
            while k < toks.len() && k <= i + 2 && !matches!(toks[k].tok, Tok::Sym("(")) {
                k += 1;
            }
            if !matches!(toks.get(k).map(|t| &t.tok), Some(Tok::Sym("("))) {
                continue;
            }
            let mut depth = 0;
            while k < toks.len() {
                match &toks[k].tok {
                    Tok::Sym("(") | Tok::Sym("[") => depth += 1,
                    Tok::Sym(")") | Tok::Sym("]") => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Tok::Ident(n)
                        if depth == 1
                            && matches!(toks[k - 1].tok, Tok::Sym("(" | ",") | Tok::Kw("mut")) =>
                    {
                        d.insert(n.clone());
                    }
                    _ => {}
                }
                k += 1;
            }
        }
        // closure params: `|a, b|` where the first '|' starts an expression
        if matches!(t.tok, Tok::Sym("|"))
            && matches!(
                toks.get(i.wrapping_sub(1)).map(|t| &t.tok),
                Some(Tok::Sym("(" | "," | "=" | "|>" | "=>")) | Some(Tok::Kw("return")) | None
            )
        {
            let mut k = i + 1;
            while let Some(tt) = toks.get(k) {
                match &tt.tok {
                    Tok::Ident(n) => {
                        d.insert(n.clone());
                    }
                    Tok::Sym(",") => {}
                    _ => break,
                }
                k += 1;
            }
        }
    }
    // define_macro!(name, (a, b) { ... })
    for w in toks.windows(3) {
        if let (Tok::Ident(m), Tok::Sym("!"), Tok::Sym("(")) = (&w[0].tok, &w[1].tok, &w[2].tok) {
            if m == "define_macro" || m == "define_test" {
                d.insert(m.clone());
            }
        }
    }
    for (i, t) in toks.iter().enumerate() {
        if matches!(&t.tok, Tok::Ident(m) if m == "define_macro") {
            // define_macro!(NAME, (p1, p2) {...})
            let mut k = i + 4;
            if let Some(Tok::Ident(n)) = toks.get(i + 3).map(|t| &t.tok) {
                d.insert(n.clone());
            }
            while let Some(tt) = toks.get(k) {
                match &tt.tok {
                    Tok::Ident(n) => {
                        d.insert(n.clone());
                    }
                    Tok::Sym("," | "(") => {}
                    _ => break,
                }
                k += 1;
            }
        }
    }
    d
}

fn suggest<'a>(name: &str, pool: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for &c in pool {
        let d = edit_distance(name, c);
        let limit = (name.len() / 3).max(1);
        if d <= limit && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    best.map(|(c, _)| c)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn const_value(e: &Expr) -> Option<crate::value::Value> {
    use crate::value::Value;
    match e {
        Expr::Int(n) => Some(Value::Int(*n)),
        Expr::Float(f) => Some(Value::Float(*f)),
        Expr::Quantity(v, u) => Some(Value::Quantity(crate::units::Quantity::new(*v, u.clone()))),
        Expr::Unary(op, x, _) if op == "-" => crate::interp::unary("-", &const_value(x)?).ok(),
        Expr::Binary(l, op, r, _) => {
            crate::interp::binary(&const_value(l)?, op, &const_value(r)?).ok()
        }
        _ => None,
    }
}

fn check_const_units(e: &Expr, out: &mut Vec<(usize, String)>) {
    if let Expr::Binary(l, op, r, line) = e {
        if let (Some(a), Some(b)) = (const_value(l), const_value(r)) {
            if let Err(m) = crate::interp::binary(&a, op, &b) {
                if m.contains("unit mismatch") {
                    out.push((*line, m));
                }
            }
        }
    }
}

fn walk_stmts(stmts: &[Stmt], f: &mut dyn FnMut(&Expr)) {
    for s in stmts {
        match s {
            Stmt::Let(_, e, _, _) | Stmt::Expr(e, _) | Stmt::Assign(_, _, e, _) => walk_expr(e, f),
            Stmt::Return(Some(e), _) => walk_expr(e, f),
            Stmt::Fn(d) => {
                for c in d.requires.iter().chain(d.ensures.iter()) {
                    walk_expr(&c.expr, f);
                }
                walk_stmts(&d.body, f)
            }
            Stmt::If(c, a, b) => {
                walk_expr(c, f);
                walk_stmts(a, f);
                if let Some(b) = b {
                    walk_stmts(b, f);
                }
            }
            Stmt::While(c, invs, b, _) => {
                walk_expr(c, f);
                for i in invs {
                    walk_expr(&i.expr, f);
                }
                walk_stmts(b, f)
            }
            Stmt::For(_, e, b, _) => {
                walk_expr(e, f);
                walk_stmts(b, f)
            }
            Stmt::Loop(b) | Stmt::MacroDef(_, _, b) => walk_stmts(b, f),
            Stmt::When(c, _, b, e, _) => {
                walk_expr(c, f);
                walk_stmts(b, f);
                if let Some(e) = e {
                    walk_stmts(e, f);
                }
            }
            Stmt::Always(c, h, _) => {
                walk_expr(&c.expr, f);
                if let Some(h) = h {
                    walk_stmts(h, f);
                }
            }
            Stmt::Assert(e, _, _, _) => walk_expr(e, f),
            Stmt::Try(a, _, b) => {
                walk_stmts(a, f);
                walk_stmts(b, f);
            }
            Stmt::Struct(d) => {
                for m in &d.methods {
                    walk_stmts(&m.body, f);
                }
            }
            _ => {}
        }
    }
}

fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::Binary(l, _, r, _)
        | Expr::Logic(l, _, r)
        | Expr::Range(l, r, _)
        | Expr::Index(l, r, _) => {
            walk_expr(l, f);
            walk_expr(r, f);
        }
        Expr::Unary(_, x, _)
        | Expr::Field(x, _, _)
        | Expr::Spawn(x, _)
        | Expr::Await(x, _)
        | Expr::Named(_, x) => walk_expr(x, f),
        Expr::Call(c, args, _) => {
            walk_expr(c, f);
            for a in args {
                walk_expr(a, f);
            }
        }
        Expr::MethodCall(o, _, args, _) => {
            walk_expr(o, f);
            for a in args {
                walk_expr(a, f);
            }
        }
        Expr::List(items) => {
            for i in items {
                walk_expr(i, f);
            }
        }
        Expr::IfExpr(c, a, b) => {
            walk_expr(c, f);
            walk_stmts(a, f);
            if let Some(b) = b {
                walk_stmts(b, f);
            }
        }
        Expr::Lambda(d) => walk_stmts(&d.body, f),
        _ => {}
    }
}

// ---------------------------------------------------------------------------- navigation

fn token_at(toks: &[Token], line: usize, col: usize) -> Option<(usize, &Token)> {
    toks.iter()
        .enumerate()
        .find(|(_, t)| t.line == line && t.col <= col && col <= t.end.max(t.col + 1))
}

fn ann(t: &Option<TypeAnn>) -> String {
    match t {
        None => String::new(),
        Some(TypeAnn::Name(n, o)) => format!(": {n}{}", if *o { "?" } else { "" }),
        Some(TypeAnn::Unit(u, o)) => format!(": [{}]{}", u.name, if *o { "?" } else { "" }),
    }
}

fn fn_signature(d: &FnDef) -> String {
    let ps: Vec<String> = d
        .params
        .iter()
        .map(|p| {
            format!(
                "{}{}{}",
                p.name,
                ann(&p.ty),
                if p.default.is_some() { " = …" } else { "" }
            )
        })
        .collect();
    let ret = match &d.ret {
        Some(_) => format!(" ->{}", ann(&d.ret).trim_start_matches(':')),
        None => String::new(),
    };
    let mut s = format!("fn {}({}){ret}", d.name, ps.join(", "));
    for r in &d.requires {
        s.push_str(&format!("\n    require {}", r.text));
    }
    for e in &d.ensures {
        s.push_str(&format!("\n    ensure {}", e.text));
    }
    s
}

fn find_decl<'a>(prog: &'a [Stmt], name: &str) -> Option<&'a Stmt> {
    prog.iter().find(|s| match s {
        Stmt::Fn(d) => d.name == name,
        Stmt::Struct(d) => d.name == name,
        Stmt::Machine(d) => d.name == name,
        Stmt::Let(n, _, _, _) => n.iter().any(|x| x == name),
        _ => false,
    })
}

pub fn hover(text: &str, line: usize, col: usize) -> Option<String> {
    let toks = lex(text).ok()?;
    let (i, t) = token_at(&toks, line, col)?;
    let code = |s: String| format!("```tulpa\n{s}\n```");
    match &t.tok {
        Tok::Ident(n) => {
            if inside_unit_bracket(&toks, i) {
                let u = crate::units::parse(n).ok()?;
                return Some(format!("unit **{n}** — {} (×{} SI)", crate::units::describe(&u.dim), u.scale));
            }
            let prog = crate::parser::parse(text).ok();
            if let Some(s) = prog.as_deref().and_then(|p| find_decl(p, n)) {
                return Some(match s {
                    Stmt::Fn(d) => code(fn_signature(d)),
                    Stmt::Struct(d) => {
                        let fields: Vec<String> = d.fields.iter().map(|p| format!("    {}{}", p.name, ann(&p.ty))).collect();
                        let methods: Vec<String> = d.methods.iter().map(|m| format!("    {}", fn_signature(m))).collect();
                        let inv: Vec<String> = d.invariants.iter().map(|c| format!("    invariant {}", c.text)).collect();
                        code(format!("struct {} {{\n{}\n}}", d.name, [fields, inv, methods].concat().join("\n")))
                    }
                    Stmt::Machine(d) => {
                        let mut evs: Vec<String> = d.states.iter().flat_map(|s| s.transitions.iter()).chain(d.global.iter()).map(|t| t.event.clone()).collect();
                        evs.sort();
                        evs.dedup();
                        let states: Vec<&str> = d.states.iter().map(|s| s.name.as_str()).collect();
                        format!("{}\nstates: {}  \nevents: {}", code(format!("machine {}", d.name)), states.join(" → "), evs.join(", "))
                    }
                    Stmt::Let(_, e, c, _) => code(format!("{} {n}{}", if *c { "const" } else { "let" }, match e {
                        Expr::Int(v) => format!(" = {v}"),
                        Expr::Float(v) => format!(" = {v}"),
                        Expr::Quantity(v, u) => format!(" = {v}[{}]  // {}", u.name, crate::units::describe(&u.dim)),
                        Expr::Str(s) => format!(" = {s:?}"),
                        _ => String::new(),
                    })),
                    _ => return None,
                });
            }
            if crate::builtins::global_names().contains(&n.as_str()) {
                return Some(format!("builtin **{n}**"));
            }
            None
        }
        Tok::Kw(k) => Some(match *k {
            "when" => "**when** cond [for duration] { … } else { … } — edge-triggered reactive rule".into(),
            "always" => "**always** cond [else { handler }] — safety monitor checked after every device call, assignment and transition".into(),
            "require" => "**require** — precondition, checked when the function is called".into(),
            "ensure" => "**ensure** — postcondition; `x` is the initial value, `x'` the final one, `result'` the return value".into(),
            "invariant" => "**invariant** — checked before every loop test, or after every struct method/field write".into(),
            "machine" => "**machine** Name(params) { state … { on event => Target } }".into(),
            other => format!("keyword **{other}**"),
        }),
        _ => None,
    }
}

const METHODS: &[&str] = &[
    "len",
    "push",
    "pop",
    "insert",
    "remove",
    "map",
    "filter",
    "reduce",
    "join",
    "split",
    "upper",
    "lower",
    "strip",
    "replace",
    "contains",
    "keys",
    "values",
    "items",
    "get",
    "set",
    "to",
    "in",
    "value",
    "unit",
    "round",
    "send",
    "can",
    "state",
    "history",
    "events",
    "set_high",
    "set_low",
    "toggle",
    "read",
    "write",
    "write_reg",
    "read_reg",
    "read_u16",
    "read_i16",
    "transfer",
    "send_line",
    "read_line",
    "angle",
    "frequency",
    "duty",
    "pulse",
    "update",
    "subscribe",
    "rate",
    "avg",
    "mermaid",
];

pub fn completion(text: &str, line: usize, col: usize) -> Vec<J> {
    let prefix: String = text
        .lines()
        .nth(line.saturating_sub(1))
        .map(|l| l.chars().take(col.saturating_sub(1)).collect())
        .unwrap_or_default();
    let item = |label: &str, kind: u32, detail: &str| json!({"label": label, "kind": kind, "detail": detail});
    // inside `5[` … → units
    if let Some(open) = prefix.rfind('[') {
        let before = &prefix[..open];
        if !prefix[open..].contains(']')
            && (before.ends_with(|c: char| c.is_ascii_digit())
                || before.trim_end().ends_with(':')
                || before.trim_end().ends_with("->"))
        {
            return crate::units::unit_symbols()
                .into_iter()
                .map(|u| {
                    let d = crate::units::parse(u)
                        .map(|x| crate::units::describe(&x.dim))
                        .unwrap_or_default();
                    item(u, 11, &d)
                })
                .collect();
        }
    }
    // `obj.<partial>` → methods
    let before_word = prefix.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    if before_word.ends_with('.') && !before_word.ends_with("..") {
        return METHODS.iter().map(|m| item(m, 2, "method")).collect();
    }
    let mut out: Vec<J> = KEYWORDS.iter().map(|k| item(k, 14, "keyword")).collect();
    out.extend(
        crate::builtins::global_names()
            .into_iter()
            .map(|b| item(b, 3, "builtin")),
    );
    for s in symbols(text) {
        let name = s["name"].as_str().unwrap_or("").to_string();
        out.push(item(&name, 6, s["detail"].as_str().unwrap_or("")));
    }
    out
}

/// Document symbols: functions, structs, machines (with their states), constants.
pub fn symbols(text: &str) -> Vec<J> {
    let Ok(toks) = lex(text) else { return vec![] };
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let (kind, detail) = match &t.tok {
            Tok::Kw("fn" | "def") => (12, "fn"),
            Tok::Kw("struct") => (23, "struct"),
            Tok::Kw("machine") => (5, "machine"),
            Tok::Kw("const") => (14, "const"),
            Tok::Ident(s) if s == "state" => (22, "state"),
            _ => continue,
        };
        if let Some(Token {
            tok: Tok::Ident(n),
            line,
            col,
            ..
        }) = toks.get(i + 1)
        {
            let r = range(*line, *col, n.chars().count());
            out.push(
                json!({"name": n, "kind": kind, "detail": detail, "range": r, "selectionRange": r}),
            );
        }
    }
    out
}

/// Location (line, col, len) of the declaration of the identifier under the cursor.
pub fn definition(text: &str, line: usize, col: usize) -> Option<(usize, usize, usize)> {
    let toks = lex(text).ok()?;
    let (_, t) = token_at(&toks, line, col)?;
    let Tok::Ident(name) = &t.tok else {
        return None;
    };
    for (i, d) in toks.iter().enumerate() {
        if let Tok::Kw("fn" | "def" | "struct" | "machine" | "let" | "const") = d.tok {
            let mut k = i + 1;
            if matches!(toks.get(k).map(|t| &t.tok), Some(Tok::Kw("mut"))) {
                k += 1;
            }
            if let Some(Token {
                tok: Tok::Ident(n),
                line,
                col,
                ..
            }) = toks.get(k)
            {
                if n == name {
                    return Some((*line, *col, n.chars().count()));
                }
            }
        }
        if let Tok::Ident(s) = &d.tok {
            if s == "state" {
                if let Some(Token {
                    tok: Tok::Ident(n),
                    line,
                    col,
                    ..
                }) = toks.get(i + 1)
                {
                    if n == name {
                        return Some((*line, *col, n.chars().count()));
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------- transport

fn read_message(input: &mut impl BufRead) -> Option<J> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            len = v.trim().parse::<usize>().ok();
        }
    }
    let mut buf = vec![0u8; len?];
    input.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_message(out: &mut impl Write, msg: &J) {
    let body = msg.to_string();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

/// Runs the server on stdin/stdout until `exit`.
pub fn serve() -> std::process::ExitCode {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut st = State::default();
    while let Some(msg) = read_message(&mut input) {
        if msg.get("method").and_then(|m| m.as_str()) == Some("exit") {
            return if st.shutdown {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::from(1)
            };
        }
        for r in handle(&mut st, &msg) {
            write_message(&mut out, &r);
        }
    }
    std::process::ExitCode::SUCCESS
}
