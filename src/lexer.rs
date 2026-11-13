//! Tokenizer for Tulpa source.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    /// f-string: raw text, interpolations parsed later by the parser.
    FStr(String),
    /// Identifier. A trailing `'` (Hehner prime, e.g. `result'`) is kept in the name.
    Ident(String),
    Kw(&'static str),
    Sym(&'static str),
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Tok::Int(n) => write!(f, "{n}"),
            Tok::Float(n) => write!(f, "{n}"),
            Tok::Str(s) | Tok::FStr(s) => write!(f, "\"{s}\""),
            Tok::Ident(s) => write!(f, "{s}"),
            Tok::Kw(s) | Tok::Sym(s) => write!(f, "{s}"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
    /// True when a line break separates this token from the previous one.
    pub nl_before: bool,
    /// Column just past the token's last character.
    pub end: usize,
}

pub const KEYWORDS: &[&str] = &[
    "let",
    "mut",
    "fn",
    "def",
    "return",
    "if",
    "elif",
    "else",
    "while",
    "for",
    "in",
    "break",
    "continue",
    "match",
    "use",
    "true",
    "false",
    "nil",
    "and",
    "or",
    "not",
    "spawn",
    "await",
    "require",
    "ensure",
    "invariant",
    "assert",
    "loop",
    "try",
    "catch",
    "const",
    "struct",
    "machine",
    "when",
    "always",
    "import",
];

// Longest first so greedy matching works.
const SYMBOLS: &[&str] = &[
    "..=", "|>", "**", "==", "!=", "<=", ">=", "&&", "||", "::", "=>", "->", "..", "+=", "-=",
    "*=", "/=", "%=", "<<", ">>", "+", "-", "*", "/", "%", "=", "<", ">", "!", "(", ")", "{", "}",
    "[", "]", ",", ";", ":", ".", "|", "&", "^", "~", "?",
];

#[derive(Debug, Clone)]
pub struct LexError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
}

pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let (mut line, mut col) = (1usize, 1usize);
    let mut out = Vec::new();
    let mut nl = true;

    macro_rules! bump {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }
    let err = |msg: String, line, col| LexError { msg, line, col };

    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            nl = true;
            bump!();
            continue;
        }
        if c.is_whitespace() {
            bump!();
            continue;
        }
        // Comments: `# ...` and `// ...`, block `/* ... */`
        if c == '#' || (c == '/' && chars.get(i + 1) == Some(&'/')) {
            while i < chars.len() && chars[i] != '\n' {
                bump!();
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let (l0, c0) = (line, col);
            bump!();
            bump!();
            loop {
                if i + 1 >= chars.len() {
                    return Err(err("unterminated block comment".into(), l0, c0));
                }
                if chars[i] == '*' && chars[i + 1] == '/' {
                    bump!();
                    bump!();
                    break;
                }
                bump!();
            }
            continue;
        }

        let (tl, tc) = (line, col);
        let push = |out: &mut Vec<Token>, tok, nl: &mut bool, end: usize| {
            out.push(Token {
                tok,
                line: tl,
                col: tc,
                nl_before: *nl,
                end,
            });
            *nl = false;
        };

        // f-strings
        if c == 'f' && matches!(chars.get(i + 1), Some('"') | Some('\'')) {
            bump!();
            let s = read_string(&chars, &mut i, &mut line, &mut col, true)
                .map_err(|m| err(m, tl, tc))?;
            push(&mut out, Tok::FStr(s), &mut nl, col);
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                bump!();
            }
            let mut word: String = chars[start..i].iter().collect();
            if i < chars.len() && chars[i] == '\'' {
                word.push('\'');
                bump!();
            }
            let tok = match KEYWORDS.iter().find(|k| **k == word) {
                Some(k) => Tok::Kw(k),
                None => Tok::Ident(word),
            };
            push(&mut out, tok, &mut nl, col);
            continue;
        }
        if c.is_ascii_digit() {
            let tok = read_number(&chars, &mut i, &mut col).map_err(|m| err(m, tl, tc))?;
            push(&mut out, tok, &mut nl, col);
            continue;
        }
        if c == '"' || c == '\'' {
            let s = read_string(&chars, &mut i, &mut line, &mut col, false)
                .map_err(|m| err(m, tl, tc))?;
            push(&mut out, Tok::Str(s), &mut nl, col);
            continue;
        }
        let mut matched = None;
        for s in SYMBOLS {
            let n = s.chars().count();
            if i + n <= chars.len() && chars[i..i + n].iter().copied().eq(s.chars()) {
                matched = Some(*s);
                break;
            }
        }
        match matched {
            Some(s) => {
                for _ in 0..s.len() {
                    bump!();
                }
                push(&mut out, Tok::Sym(s), &mut nl, col);
            }
            None => return Err(err(format!("unexpected character '{c}'"), tl, tc)),
        }
    }
    out.push(Token {
        tok: Tok::Eof,
        line,
        col,
        nl_before: true,
        end: col,
    });
    Ok(out)
}

fn read_number(chars: &[char], i: &mut usize, col: &mut usize) -> Result<Tok, String> {
    let start = *i;
    let adv = |i: &mut usize, col: &mut usize| {
        *i += 1;
        *col += 1;
    };
    if chars[*i] == '0'
        && matches!(
            chars.get(*i + 1),
            Some('x') | Some('X') | Some('b') | Some('B')
        )
    {
        let radix = if matches!(chars[*i + 1], 'x' | 'X') {
            16
        } else {
            2
        };
        adv(i, col);
        adv(i, col);
        let ds = *i;
        while *i < chars.len() && (chars[*i].is_digit(radix) || chars[*i] == '_') {
            adv(i, col);
        }
        let digits: String = chars[ds..*i].iter().filter(|c| **c != '_').collect();
        return i64::from_str_radix(&digits, radix)
            .map(Tok::Int)
            .map_err(|_| "invalid integer literal".to_string());
    }
    let mut is_float = false;
    while *i < chars.len() {
        let c = chars[*i];
        if c.is_ascii_digit() || c == '_' {
            adv(i, col);
        } else if c == '.' && !is_float && chars.get(*i + 1).is_some_and(|d| d.is_ascii_digit()) {
            is_float = true;
            adv(i, col);
        } else if (c == 'e' || c == 'E')
            && chars
                .get(*i + 1)
                .is_some_and(|d| d.is_ascii_digit() || *d == '-' || *d == '+')
        {
            is_float = true;
            adv(i, col);
            if matches!(chars[*i], '-' | '+') {
                adv(i, col);
            }
        } else {
            break;
        }
    }
    let text: String = chars[start..*i].iter().filter(|c| **c != '_').collect();
    if is_float {
        text.parse()
            .map(Tok::Float)
            .map_err(|_| "invalid float literal".into())
    } else {
        text.parse()
            .map(Tok::Int)
            .map_err(|_| "integer literal too large".into())
    }
}

fn read_string(
    chars: &[char],
    i: &mut usize,
    line: &mut usize,
    col: &mut usize,
    raw_braces: bool,
) -> Result<String, String> {
    let quote = chars[*i];
    *i += 1;
    *col += 1;
    let mut s = String::new();
    let mut depth = 0usize; // f-string interpolation depth: quotes inside {...} don't terminate
    loop {
        let Some(&c) = chars.get(*i) else {
            return Err("unterminated string".into());
        };
        *i += 1;
        if c == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
        if raw_braces {
            if c == '{' {
                if depth == 0 && chars.get(*i) == Some(&'{') {
                    s.push_str("{{");
                    *i += 1;
                    *col += 1;
                    continue;
                }
                depth += 1;
            } else if c == '}' && depth > 0 {
                depth -= 1;
            } else if depth > 0 && (c == '"' || c == '\'') {
                // nested string literal inside an interpolation: copy verbatim
                s.push(c);
                loop {
                    let Some(&d) = chars.get(*i) else {
                        return Err("unterminated string".into());
                    };
                    *i += 1;
                    *col += 1;
                    s.push(d);
                    if d == c {
                        break;
                    }
                }
                continue;
            }
        }
        if c == quote && depth == 0 {
            return Ok(s);
        }
        if c == '\\' {
            let Some(&e) = chars.get(*i) else {
                return Err("unterminated string".into());
            };
            *i += 1;
            *col += 1;
            match e {
                'n' => s.push('\n'),
                't' => s.push('\t'),
                'r' => s.push('\r'),
                '0' => s.push('\0'),
                '\\' => s.push('\\'),
                '"' => s.push('"'),
                '\'' => s.push('\''),
                '{' if raw_braces => s.push_str("{{"),
                '}' if raw_braces => s.push_str("}}"),
                other => {
                    s.push('\\');
                    s.push(other)
                }
            }
        } else {
            s.push(c);
        }
    }
}
