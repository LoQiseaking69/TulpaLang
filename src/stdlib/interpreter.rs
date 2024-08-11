use crate::parser::{ASTNode};

pub struct Interpreter;

impl Interpreter {
    pub fn new() -> Self {
        Interpreter
    }

    pub fn interpret(&self, ast: ASTNode) -> Result<(), String> {
        self.eval(ast)?;
        Ok(())
    }

    fn eval(&self, node: ASTNode) -> Result<(), String> {
        match node {
            ASTNode::Program(nodes) => {
                for node in nodes {
                    self.eval(node)?;
                }
            }
            ASTNode::Function { name, params, body } => {
                println!("Function: {} with params: {:?}", name, params);
                for statement in body {
                    self.eval(statement)?;
                }
            }
            ASTNode::VariableDeclaration { name, value } => {
                let evaluated_value = self.eval(*value)?;
                println!("Variable {} = {:?}", name, evaluated_value);
            }
            ASTNode::BinaryOperation { left, operator, right } => {
                let left_val = self.eval(*left)?;
                let right_val = self.eval(*right)?;
                println!("Operation {} {} {}", left_val, operator, right_val);
            }
            ASTNode::Number(n) => {
                println!("Number: {}", n);
            }
            ASTNode::StringLiteral(s) => {
                println!("String: {}", s);
            }
            ASTNode::Identifier(name) => {
                println!("Identifier: {}", name);
            }
            _ => return Err(format!("Unknown AST Node")),
        }
        Ok(())
    }
}
