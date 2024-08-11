use crate::lexer::{Lexer, Token};

#[derive(Debug, Clone)]
pub enum ASTNode {
    Program(Vec<ASTNode>),
    Function {
        name: String,
        params: Vec<String>,
        body: Vec<ASTNode>,
    },
    VariableDeclaration {
        name: String,
        value: Box<ASTNode>,
    },
    VariableAssignment {
        name: String,
        value: Box<ASTNode>,
    },
    BinaryOperation {
        left: Box<ASTNode>,
        operator: String,
        right: Box<ASTNode>,
    },
    IfStatement {
        condition: Box<ASTNode>,
        body: Vec<ASTNode>,
        else_body: Option<Vec<ASTNode>>,
    },
    WhileLoop {
        condition: Box<ASTNode>,
        body: Vec<ASTNode>,
    },
    Number(f64),
    StringLiteral(String),
    Identifier(String),
}

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    current_token: Token,
}

impl<'a> Parser<'a> {
    pub fn new(lexer: Lexer<'a>) -> Self {
        let mut parser = Parser {
            lexer,
            current_token: Token::EOF,
        };
        parser.advance();
        parser
    }

    pub fn advance(&mut self) {
        self.current_token = self.lexer.lex().remove(0);
    }

    pub fn parse(&mut self) -> ASTNode {
        self.parse_program()
    }

    fn parse_program(&mut self) -> ASTNode {
        let mut nodes = Vec::new();
        while self.current_token != Token::EOF {
            nodes.push(self.parse_statement());
        }
        ASTNode::Program(nodes)
    }

    fn parse_statement(&mut self) -> ASTNode {
        match &self.current_token {
            Token::Keyword(k) if k == "let" => self.parse_variable_declaration(),
            Token::Keyword(k) if k == "fn" || k == "def" => self.parse_function(),
            Token::Keyword(k) if k == "if" => self.parse_if_statement(),
            Token::Keyword(k) if k == "while" => self.parse_while_loop(),
            Token::Identifier(_) => self.parse_assignment_or_expression(),
            _ => panic!("Unexpected token: {:?}", self.current_token),
        }
    }

    fn parse_variable_declaration(&mut self) -> ASTNode {
        self.advance(); // Skip 'let'
        if let Token::Identifier(name) = &self.current_token {
            let name = name.clone();
            self.advance(); // Skip identifier
            self.advance(); // Skip '='
            let value = self.parse_expression();
            ASTNode::VariableDeclaration {
                name,
                value: Box::new(value),
            }
        } else {
            panic!("Expected identifier after 'let'");
        }
    }

    fn parse_assignment_or_expression(&mut self) -> ASTNode {
        if let Token::Identifier(name) = &self.current_token {
            let name = name.clone();
            self.advance(); // Skip identifier

            if let Token::Operator(op) = &self.current_token {
                if op == "=" {
                    self.advance(); // Skip '='
                    let value = self.parse_expression();
                    return ASTNode::VariableAssignment {
                        name,
                        value: Box::new(value),
                    };
                }
            }

            return ASTNode::Identifier(name);
        }

        self.parse_expression()
    }

    fn parse_function(&mut self) -> ASTNode {
        self.advance(); // Skip 'fn' or 'def'
        if let Token::Identifier(name) = &self.current_token {
            let name = name.clone();
            self.advance(); // Skip function name
            self.advance(); // Skip '('
            let mut params = Vec::new();
            while let Token::Identifier(param) = &self.current_token {
                params.push(param.clone());
                self.advance();
                if self.current_token == Token::Symbol(",".to_string()) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.advance(); // Skip ')'
            self.advance(); // Skip '{'
            let mut body = Vec::new();
            while self.current_token != Token::Symbol("}".to_string()) {
                body.push(self.parse_statement());
            }
            self.advance(); // Skip '}'
            ASTNode::Function {
                name,
                params,
                body,
            }
        } else {
            panic!("Expected identifier after 'fn'");
        }
    }

    fn parse_if_statement(&mut self) -> ASTNode {
        self.advance(); // Skip 'if'
        let condition = self.parse_expression();
        self.advance(); // Skip '{'
        let mut body = Vec::new();
        while self.current_token != Token::Symbol("}".to_string()) {
            body.push(self.parse_statement());
        }
        self.advance(); // Skip '}'

        let else_body = if let Token::Keyword(k) = &self.current_token {
            if k == "else" {
                self.advance(); // Skip 'else'
                self.advance(); // Skip '{'
                let mut else_statements = Vec::new();
                while self.current_token != Token::Symbol("}".to_string()) {
                    else_statements.push(self.parse_statement());
                }
                self.advance(); // Skip '}'
                Some(else_statements)
            } else {
                None
            }
        } else {
            None
        };

        ASTNode::IfStatement {
            condition: Box::new(condition),
            body,
            else_body,
        }
    }

    fn parse_while_loop(&mut self) -> ASTNode {
        self.advance(); // Skip 'while'
        let condition = self.parse_expression();
        self.advance(); // Skip '{'
        let mut body = Vec::new();
        while self.current_token != Token::Symbol("}".to_string()) {
            body.push(self.parse_statement());
        }
        self.advance(); // Skip '}'
        ASTNode::WhileLoop {
            condition: Box::new(condition),
            body,
        }
    }

    fn parse_expression(&mut self) -> ASTNode {
        let mut left = self.parse_term();
        while let Token::Operator(op) = &self.current_token {
            if op == "+" || op == "-" {
                let operator = op.clone();
                self.advance();
                let right = self.parse_term();
                left = ASTNode::BinaryOperation {
                    left: Box::new(left),
                    operator,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        left
    }

    fn parse_term(&mut self) -> ASTNode {
        let mut left = self.parse_factor();
        while let Token::Operator(op) = &self.current_token {
            if op == "*" || op == "/" {
                let operator = op.clone();
                self.advance();
                let right = self.parse_factor();
                left = ASTNode::BinaryOperation {
                    left: Box::new(left),
                    operator,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        left
    }

    fn parse_factor(&mut self) -> ASTNode {
        match &self.current_token {
            Token::Number(n) => {
                let node = ASTNode::Number(*n);
                self.advance();
                node
            }
            Token::StringLiteral(s) => {
                let node = ASTNode::StringLiteral(s.clone());
                self.advance();
                node
            }
            Token::Identifier(name) => {
                let node = ASTNode::Identifier(name.clone());
                self.advance();
                node
            }
            Token::Symbol(s) if s == "(" => {
                self.advance(); // Skip '('
                let expr = self.parse_expression();
                self.advance(); // Skip ')'
                expr
            }
            _ => panic!("Unexpected token: {:?}", self.current_token),
        }
    }
}
