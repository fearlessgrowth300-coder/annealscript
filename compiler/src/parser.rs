//! Recursive-descent parser producing the AST in `ast.rs`.

use crate::ast::*;
use crate::lexer::{Spanned, Token};

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
}

pub struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
}

type PResult<T> = Result<T, ParseError>;

impl Parser {
    pub fn new(tokens: Vec<Spanned>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos].0
    }

    fn cur_line(&self) -> usize {
        self.tokens[self.pos].1
    }

    fn err(&self, message: String) -> ParseError {
        ParseError { message, line: self.cur_line() }
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens[self.pos].0.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &Token) -> PResult<Token> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            Ok(self.advance())
        } else {
            Err(self.err(format!("expected {:?}, got {:?}", expected, self.peek())))
        }
    }

    fn expect_ident(&mut self) -> PResult<String> {
        let line = self.cur_line();
        match self.advance() {
            Token::Ident(s) => Ok(s),
            other => Err(ParseError { message: format!("expected identifier, got {other:?}"), line }),
        }
    }

    fn skip_newlines(&mut self) {
        while *self.peek() == Token::Newline {
            self.advance();
        }
    }

    pub fn parse_program(&mut self) -> PResult<Program> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while *self.peek() != Token::Eof {
            stmts.push(self.parse_statement()?);
            self.skip_newlines();
        }
        Ok(stmts)
    }

    fn parse_statement(&mut self) -> PResult<Stmt> {
        match self.peek() {
            Token::Struct => self.parse_struct(),
            Token::Let => self.parse_let(),
            Token::Set => self.parse_set(),
            Token::Intent => self.parse_intent(),
            Token::Bound => self.parse_bound(),
            Token::Print => self.parse_print(),
            other => Err(self.err(format!("unexpected token {other:?} at statement start"))),
        }
    }

    fn parse_struct(&mut self) -> PResult<Stmt> {
        self.expect(&Token::Struct)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;
        self.skip_newlines();
        let mut fields = Vec::new();
        while *self.peek() != Token::RBrace {
            let fname = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;
            let default = if *self.peek() == Token::Default {
                self.advance();
                Some(self.parse_literal()?)
            } else {
                None
            };
            fields.push(Field { name: fname, ty, default });
            if *self.peek() == Token::Comma {
                self.advance();
            }
            self.skip_newlines();
        }
        self.expect(&Token::RBrace)?;
        Ok(Stmt::StructDef { name, fields })
    }

    fn parse_type(&mut self) -> PResult<Type> {
        let name = self.expect_ident()?;
        match name.as_str() {
            "int" => Ok(Type::Int),
            "float" => Ok(Type::Float),
            "bool" => Ok(Type::Bool),
            "string" => Ok(Type::String),
            "Probability" => {
                self.expect(&Token::LAngle)?;
                let inner = self.parse_type()?;
                self.expect(&Token::RAngle)?;
                Ok(Type::Probability(Box::new(inner)))
            }
            other => Ok(Type::Named(other.to_string())),
        }
    }

    fn parse_literal(&mut self) -> PResult<Literal> {
        let line = self.cur_line();
        match self.advance() {
            Token::Str(s) => Ok(Literal::Str(s)),
            Token::Number(n) => Ok(Literal::Num(n)),
            Token::True => Ok(Literal::Bool(true)),
            Token::False => Ok(Literal::Bool(false)),
            other => Err(ParseError { message: format!("expected literal, got {other:?}"), line }),
        }
    }

    fn parse_let(&mut self) -> PResult<Stmt> {
        self.expect(&Token::Let)?;
        let name = self.expect_ident()?;
        let ty = if *self.peek() == Token::Colon {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&Token::Equals)?;
        let expr = self.parse_expr()?;
        Ok(Stmt::Let { name, ty, expr })
    }

    fn parse_set(&mut self) -> PResult<Stmt> {
        self.expect(&Token::Set)?;
        let name = self.expect_ident()?;
        self.expect(&Token::Equals)?;
        let expr = self.parse_expr()?;
        Ok(Stmt::Set { name, expr })
    }

    fn parse_intent(&mut self) -> PResult<Stmt> {
        self.expect(&Token::Intent)?;
        let source = self.parse_expr()?;
        self.expect(&Token::Arrow)?;
        let struct_name = self.expect_ident()?;
        self.expect(&Token::As)?;
        let var = self.expect_ident()?;
        Ok(Stmt::Intent { source, struct_name, var })
    }

    fn parse_bound(&mut self) -> PResult<Stmt> {
        self.expect(&Token::Bound)?;
        let var = self.expect_ident()?;
        let op_line = self.cur_line();
        let op = match self.advance() {
            Token::Le => Comparator::Le,
            Token::Ge => Comparator::Ge,
            other => return Err(ParseError { message: format!("expected <= or >=, got {other:?}"), line: op_line }),
        };
        let limit_line = self.cur_line();
        let limit = match self.advance() {
            Token::Number(n) => n,
            other => return Err(ParseError { message: format!("expected number, got {other:?}"), line: limit_line }),
        };
        self.expect(&Token::LBrace)?;
        self.skip_newlines();
        let mut body = Vec::new();
        while *self.peek() != Token::RBrace {
            body.push(self.parse_statement()?);
            self.skip_newlines();
        }
        self.expect(&Token::RBrace)?;
        Ok(Stmt::Bound { var, op, limit, body })
    }

    fn parse_print(&mut self) -> PResult<Stmt> {
        self.expect(&Token::Print)?;
        self.expect(&Token::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(&Token::RParen)?;
        Ok(Stmt::Print(expr))
    }

    fn parse_expr(&mut self) -> PResult<Expr> {
        match self.peek().clone() {
            Token::LBrace => self.parse_dict(),
            Token::Resolve => self.parse_resolve(),
            Token::Str(s) => {
                self.advance();
                Ok(Expr::Literal(Literal::Str(s)))
            }
            Token::Number(n) => {
                self.advance();
                Ok(Expr::Literal(Literal::Num(n)))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(true)))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(false)))
            }
            Token::Ident(name) => {
                self.advance();
                if *self.peek() == Token::Dot {
                    self.advance();
                    let field = self.expect_ident()?;
                    Ok(Expr::FieldAccess(name, field))
                } else if *self.peek() == Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    while *self.peek() != Token::RParen {
                        args.push(self.parse_expr()?);
                        if *self.peek() == Token::Comma {
                            self.advance();
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            other => Err(self.err(format!("unexpected token {other:?} in expression"))),
        }
    }

    fn parse_resolve(&mut self) -> PResult<Expr> {
        self.expect(&Token::Resolve)?;
        let subject = self.parse_expr()?;
        self.expect(&Token::LBrace)?;
        self.skip_newlines();
        self.expect(&Token::Above)?;
        let threshold_line = self.cur_line();
        let threshold = match self.advance() {
            Token::Number(n) => n,
            other => return Err(ParseError { message: format!("expected number, got {other:?}"), line: threshold_line }),
        };
        self.expect(&Token::FatArrow)?;
        let above = self.parse_expr()?;
        self.expect(&Token::Comma)?;
        self.skip_newlines();
        self.expect(&Token::Else)?;
        self.expect(&Token::FatArrow)?;
        let else_branch = self.parse_expr()?;
        self.skip_newlines();
        self.expect(&Token::RBrace)?;
        Ok(Expr::Resolve {
            subject: Box::new(subject),
            threshold,
            above: Box::new(above),
            else_branch: Box::new(else_branch),
        })
    }

    fn parse_dict(&mut self) -> PResult<Expr> {
        self.expect(&Token::LBrace)?;
        self.skip_newlines();
        let mut pairs = Vec::new();
        while *self.peek() != Token::RBrace {
            let key_line = self.cur_line();
            let key = match self.advance() {
                Token::Str(s) => s,
                other => return Err(ParseError { message: format!("expected string key, got {other:?}"), line: key_line }),
            };
            self.expect(&Token::Colon)?;
            let value = self.parse_literal()?;
            pairs.push((key, value));
            if *self.peek() == Token::Comma {
                self.advance();
            }
            self.skip_newlines();
        }
        self.expect(&Token::RBrace)?;
        Ok(Expr::Dict(pairs))
    }
}

pub fn parse(tokens: Vec<Spanned>) -> PResult<Program> {
    Parser::new(tokens).parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    #[test]
    fn parses_struct_with_default_and_probability() {
        let src = r#"
struct User {
  username: string default "",
  trust: Probability<float>
}
"#;
        let prog = parse(lex(src).unwrap()).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::StructDef { name, fields } => {
                assert_eq!(name, "User");
                assert_eq!(fields[0].name, "username");
                assert_eq!(fields[0].default, Some(Literal::Str("".into())));
                assert_eq!(fields[1].ty, Type::Probability(Box::new(Type::Float)));
            }
            other => panic!("expected StructDef, got {other:?}"),
        }
    }

    #[test]
    fn parses_intent_and_bound() {
        let src = r#"
let raw = {"usr_nm": "alice"}
intent raw -> User as user
bound speed <= 15 {
  set speed = 20
}
"#;
        let prog = parse(lex(src).unwrap()).unwrap();
        assert!(matches!(prog[1], Stmt::Intent { .. }));
        assert!(matches!(prog[2], Stmt::Bound { .. }));
    }

    #[test]
    fn parses_resolve_expression() {
        let src = r#"let v = resolve trust { above 0.8 => trust, else => 0 }"#;
        let prog = parse(lex(src).unwrap()).unwrap();
        match &prog[0] {
            Stmt::Let { expr: Expr::Resolve { threshold, .. }, .. } => {
                assert_eq!(*threshold, 0.8);
            }
            other => panic!("expected Let(Resolve), got {other:?}"),
        }
    }
}
