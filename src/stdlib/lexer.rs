use std::str::Chars;

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    Identifier(String),
    Number(f64),
    Keyword(String),
    Operator(String),
    Symbol(String),
    StringLiteral(String),
    EOF,
}

pub struct Lexer<'a> {
    source: Chars<'a>,
    current_char: Option<char>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut lexer = Lexer {
            source: source.chars(),
            current_char: None,
        };
        lexer.advance();
        lexer
    }

    pub fn advance(&mut self) {
        self.current_char = self.source.next();
    }

    pub fn lex(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        while let Some(current) = self.current_char {
            let token = match current {
                c if c.is_alphabetic() || c == '_' => self.lex_identifier(),
                c if c.is_digit(10) => self.lex_number(),
                '"' => self.lex_string(),
                '+' | '-' | '*' | '/' | '=' | '!' => self.lex_operator(),
                '(' | ')' | '{' | '}' | ',' | ';' => self.lex_symbol(),
                ' ' | '\t' | '\n' | '\r' => {
                    self.advance();
                    continue;
                }
                _ => panic!("Unexpected character: {}", current),
            };
            tokens.push(token);
        }
        tokens.push(Token::EOF);
        tokens
    }

    fn lex_identifier(&mut self) -> Token {
        let mut identifier = String::new();
        while let Some(c) = self.current_char {
            if c.is_alphabetic() || c.is_digit(10) || c == '_' {
                identifier.push(c);
                self.advance();
            } else {
                break;
            }
        }
        match identifier.as_str() {
            "def" | "fn" | "let" | "if" | "else" | "for" | "while" | "return" => {
                Token::Keyword(identifier)
            }
            _ => Token::Identifier(identifier),
        }
    }

    fn lex_number(&mut self) -> Token {
        let mut number = String::new();
        while let Some(c) = self.current_char {
            if c.is_digit(10) || c == '.' {
                number.push(c);
                self.advance();
            } else {
                break;
            }
        }
        Token::Number(number.parse().unwrap())
    }

    fn lex_string(&mut self) -> Token {
        self.advance(); // Skip opening quote
        let mut string = String::new();
        while let Some(c) = self.current_char {
            if c == '"' {
                break;
            }
            string.push(c);
            self.advance();
        }
        self.advance(); // Skip closing quote
        Token::StringLiteral(string)
    }

    fn lex_operator(&mut self) -> Token {
        let mut operator = String::new();
        while let Some(c) = self.current_char {
            if "+-*/=!".contains(c) {
                operator.push(c);
                self.advance();
            } else {
                break;
            }
        }
        Token::Operator(operator)
    }

    fn lex_symbol(&mut self) -> Token {
        let symbol = self.current_char.unwrap().to_string();
        self.advance();
        Token::Symbol(symbol)
    }
}
