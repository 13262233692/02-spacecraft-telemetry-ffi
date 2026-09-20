//! Small, safe expression language used by fault-tree leaves.
//!
//! Supported:
//! * identifiers (field names from the frame value map),
//! * integer / float literals, double-quoted strings and booleans,
//! * comparison operators `== != < <= > >=`,
//! * logical `and or not`, `&& || !`, and parentheses.
//!
//! Comparison semantics mirror the frame parser's value model: a field with an
//! enum mapping behaves as a string (`sh_eps_mode == "EPS_SUN"`), all other
//! fields behave as numbers.

use serde_json::Value;

use crate::error::{Result, RuleError};

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Number(f64),
    Str(String),
    Bool(bool),
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    LParen,
    RParen,
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                tokens.push(Token::LParen);
                i += 1
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1
            }
            '&' if i + 1 < bytes.len() && bytes[i + 1] == b'&' => {
                tokens.push(Token::And);
                i += 2
            }
            '|' if i + 1 < bytes.len() && bytes[i + 1] == b'|' => {
                tokens.push(Token::Or);
                i += 2
            }
            '!' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                tokens.push(Token::Ne);
                i += 2
            }
            '!' => {
                tokens.push(Token::Not);
                i += 1
            }
            '=' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                tokens.push(Token::Eq);
                i += 2
            }
            '<' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                tokens.push(Token::Le);
                i += 2
            }
            '>' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                tokens.push(Token::Ge);
                i += 2
            }
            '<' => {
                tokens.push(Token::Lt);
                i += 1
            }
            '>' => {
                tokens.push(Token::Gt);
                i += 1
            }
            '"' => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != b'"' {
                    j += 1;
                }
                if j >= bytes.len() {
                    return Err(RuleError::Expression(format!(
                        "unterminated string in `{input}`"
                    )));
                }
                let s = std::str::from_utf8(&bytes[start..j])
                    .map_err(|_| RuleError::Expression("non-UTF8 string".into()))?
                    .to_string();
                tokens.push(Token::Str(s));
                i = j + 1;
            }
            c if c.is_ascii_digit()
                || (c == '-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) =>
            {
                let start = i;
                if c == '-' {
                    i += 1;
                }
                while i < bytes.len() && ((bytes[i] as char).is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                let text = std::str::from_utf8(&bytes[start..i])
                    .map_err(|_| RuleError::Expression("bad number".into()))?;
                let n: f64 = text
                    .parse()
                    .map_err(|_| RuleError::Expression(format!("invalid number `{text}`")))?;
                tokens.push(Token::Number(n));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len()
                    && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                let word = std::str::from_utf8(&bytes[start..i])
                    .map_err(|_| RuleError::Expression("bad identifier".into()))?;
                match word {
                    "and" => tokens.push(Token::And),
                    "or" => tokens.push(Token::Or),
                    "not" => tokens.push(Token::Not),
                    "true" => tokens.push(Token::Bool(true)),
                    "false" => tokens.push(Token::Bool(false)),
                    other => tokens.push(Token::Ident(other.to_string())),
                }
            }
            other => {
                return Err(RuleError::Expression(format!(
                    "unexpected character `{other}` in `{input}`"
                )))
            }
        }
    }
    Ok(tokens)
}

#[derive(Debug, Clone)]
enum Expr {
    Field(String),
    Num(f64),
    Text(String),
    Bool(bool),
    Cmp {
        op: CmpOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn parse(input: &str) -> Result<Expr> {
        let tokens = tokenize(input)?;
        if tokens.is_empty() {
            return Err(RuleError::Expression("empty expression".into()));
        }
        let mut parser = Parser { tokens, pos: 0 };
        let expr = parser.parse_or()?;
        if parser.pos != parser.tokens.len() {
            return Err(RuleError::Expression(format!(
                "unexpected token at position {}",
                parser.pos
            )));
        }
        Ok(expr)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(&Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_not()?;
        while self.peek() == Some(&Token::And) {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr> {
        if self.peek() == Some(&Token::Not) {
            self.advance();
            return Ok(Expr::Not(Box::new(self.parse_not()?)));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let left = self.parse_primary()?;
        let op = match self.peek() {
            Some(Token::Eq) => CmpOp::Eq,
            Some(Token::Ne) => CmpOp::Ne,
            Some(Token::Lt) => CmpOp::Lt,
            Some(Token::Le) => CmpOp::Le,
            Some(Token::Gt) => CmpOp::Gt,
            Some(Token::Ge) => CmpOp::Ge,
            _ => return Ok(left),
        };
        self.advance();
        let right = self.parse_primary()?;
        Ok(Expr::Cmp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        match self.advance() {
            Some(Token::LParen) => {
                let inner = self.parse_or()?;
                if self.advance() != Some(Token::RParen) {
                    return Err(RuleError::Expression("missing closing parenthesis".into()));
                }
                Ok(inner)
            }
            Some(Token::Number(n)) => Ok(Expr::Num(n)),
            Some(Token::Str(s)) => Ok(Expr::Text(s)),
            Some(Token::Bool(b)) => Ok(Expr::Bool(b)),
            Some(Token::Ident(name)) => Ok(Expr::Field(name)),
            other => Err(RuleError::Expression(format!(
                "expected value, found {other:?}"
            ))),
        }
    }
}

/// Parse and evaluate an expression against a decoded frame's value map.
pub fn eval_bool(input: &str, values: &serde_json::Map<String, Value>) -> Result<bool> {
    let expr = Parser::parse(input)?;
    eval(&expr, values).map(|v| matches!(v, Scalar::Bool(true)))
}

#[derive(Debug, Clone, PartialEq)]
enum Scalar {
    Num(f64),
    Text(String),
    Bool(bool),
    /// Field not present in this frame (e.g. CLCW when OCF flag is absent).
    Missing,
}

fn field_value(name: &str, values: &serde_json::Map<String, Value>) -> Result<Scalar> {
    let Some(value) = values.get(name) else {
        return Ok(Scalar::Missing);
    };
    match value {
        Value::Number(n) => Ok(Scalar::Num(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => Ok(Scalar::Text(s.clone())),
        Value::Bool(b) => Ok(Scalar::Bool(*b)),
        Value::Null => Ok(Scalar::Missing),
        other => Err(RuleError::Expression(format!(
            "field `{name}` has unsupported type {other}"
        ))),
    }
}

fn eval(expr: &Expr, values: &serde_json::Map<String, Value>) -> Result<Scalar> {
    match expr {
        Expr::Field(name) => field_value(name, values),
        Expr::Num(n) => Ok(Scalar::Num(*n)),
        Expr::Text(s) => Ok(Scalar::Text(s.clone())),
        Expr::Bool(b) => Ok(Scalar::Bool(*b)),
        Expr::Not(inner) => match eval(inner, values)? {
            Scalar::Bool(b) => Ok(Scalar::Bool(!b)),
            Scalar::Missing => Ok(Scalar::Bool(false)),
            other => Err(RuleError::Expression(format!(
                "`not` requires boolean operand, got {other:?}"
            ))),
        },
        Expr::And(a, b) => {
            let left = eval(a, values)?;
            if matches!(left, Scalar::Bool(false)) {
                return Ok(Scalar::Bool(false));
            }
            Ok(Scalar::Bool(
                matches!(left, Scalar::Bool(true))
                    && matches!(eval(b, values)?, Scalar::Bool(true)),
            ))
        }
        Expr::Or(a, b) => {
            let left = eval(a, values)?;
            if matches!(left, Scalar::Bool(true)) {
                return Ok(Scalar::Bool(true));
            }
            Ok(Scalar::Bool(
                matches!(left, Scalar::Bool(true))
                    || matches!(eval(b, values)?, Scalar::Bool(true)),
            ))
        }
        Expr::Cmp { op, left, right } => {
            let l = eval(left, values)?;
            let r = eval(right, values)?;
            compare(op, l, r)
        }
    }
}

fn compare(op: &CmpOp, l: Scalar, r: Scalar) -> Result<Scalar> {
    // A missing optional field never satisfies a leaf condition.
    if matches!(l, Scalar::Missing) || matches!(r, Scalar::Missing) {
        return Ok(Scalar::Bool(false));
    }
    let result = match (&l, &r) {
        (Scalar::Text(a), Scalar::Text(b)) => match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            _ => {
                return Err(RuleError::Expression(
                    "only == and != apply to string values".into(),
                ))
            }
        },
        (Scalar::Num(a), Scalar::Num(b)) => match op {
            CmpOp::Eq => (a - b).abs() < f64::EPSILON,
            CmpOp::Ne => (a - b).abs() >= f64::EPSILON,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        },
        (Scalar::Bool(a), Scalar::Bool(b)) => match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            _ => return Err(RuleError::Expression("cannot order booleans".into())),
        },
        _ => {
            return Err(RuleError::Expression(format!(
                "cannot compare {l:?} with {r:?}"
            )))
        }
    };
    Ok(Scalar::Bool(result))
}
