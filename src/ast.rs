use pest::iterators::Pair;
use std::fmt::Debug;
use crate::Rule;

/// ASTの定義
#[derive(Debug)]
#[allow(unused)]
pub enum Stmt {
    Import(Vec<String>),
    Decorator {
        name: String,
        target: String,
        pairs: Vec<(String, Expr)>,
    },
    Bundle {
        name: String,
        body: Vec<Stmt>,
    },
    Declaration {
        is_state: bool,
        is_mut: bool,
        name: String,
        value: Expr,
        range: Option<RangeLimit>,
    },
    ExprStmt(Expr),
}

#[derive(Debug)]
#[allow(unused)]
pub struct RangeLimit {
    pub start: Expr,
    pub end: Expr,
    pub cycle: bool,
}

#[derive(Debug)]
#[allow(unused)]
pub enum Expr {
    Ident(String),
    Integer(i32),
    String(String),
    BinaryOp {
        left: Box<Expr>,
        op: Op,
        right: Box<Expr>,
    },
    CallChain {
        head: String,
        tails: Vec<Accessor>,
    },
}

#[derive(Debug)]
#[allow(unused)]
pub enum Accessor {
    Property(String),
    Method(Vec<Expr>),
}

#[derive(Debug)]
#[allow(unused)]
pub enum Op {
    Add, Sub, In, Question, // ??
}

#[allow(unused)]
fn parse_stmt(pair: Pair<Rule>) -> Stmt {
    match pair.as_rule() {
        Rule::declaration => {
            let mut inner = pair.into_inner();
            let mut is_state = false;
            let mut is_mut = false;

            let mut next = inner.next().unwrap();

            if next.as_rule() == Rule::state_kw {
                is_state = true;
                next = inner.next().unwrap();
            }

            let mut current = next;
            if current.as_str() == "let" {
                current = inner.next().unwrap();
            }

            if current.as_str() == "mut" {
                is_mut = true;
                current = inner.next().unwrap();
            }

            let name = current.as_str().to_string();
            let value = parse_expr(inner.next().unwrap());
            let range = inner.next().map(|p| parse_range_limit(p));

            Stmt::Declaration {
                is_state,
                is_mut,
                name,
                value,
                range,
            }
        }
        Rule::import_stmt => {
            let path = pair.into_inner().next().unwrap()
                .into_inner().map(|p| p.as_str().to_string()).collect();
            Stmt::Import(path)
        }
        _ => unreachable!("Undefined: {:?}", pair.as_rule()),
    }
}

fn parse_expr(pair: Pair<Rule>) -> Expr {
    let mut inner = pair.into_inner();
    let head = inner.next().unwrap();

    match head.as_rule() {
        Rule::integer => Expr::Integer(head.as_str().parse().unwrap()),
        Rule::string => Expr::String(head.into_inner().next().unwrap().as_str().to_string()),
        Rule::ident => Expr::Ident(head.as_str().to_string()),
        _ => Expr::Ident("unknown".to_string()),
    }
}

fn parse_range_limit(pair: Pair<Rule>) -> RangeLimit {
    let mut inner = pair.into_inner();
    let start = parse_expr(inner.next().unwrap());
    let end = parse_expr(inner.next().unwrap());
    let cycle = inner.next().is_some();

    RangeLimit { start, end, cycle }
}