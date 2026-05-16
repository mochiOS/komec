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
    Add,
    Sub,
    Mul,
    Div,
    In,
    Question, // ??
}

/// `pest` のパース結果から単一の文を解析してASTの `Stmt` へ変換する
///
/// プログラムのトップレベルやブロック内で出現する各種宣言（変数宣言やインポート文など） CSTを受け取り、コンパイラが処理しやすい抽象構文木（AST）にマッピングする
///
/// # Args
///
/// * `pair`: `Rule::declaration` または `Rule::import_stmt` にマッチした `pest::iterators::Pair`
///
/// # Returns
///
/// 解析に成功した場合、対応する `Stmt` 列挙型のバリアントを返す。例えば、変数宣言であれば `Stmt::Declaration`、インポート文であれば `Stmt::Import` など。
///
/// # Panic
///
/// 文法定義（`.pest`）で許可されているにもかかわらず、この関数内で定義されていない
/// 未実装のルールが渡された場合、`unreachable!` マクロによってパニックする
#[allow(unused)]
pub(crate) fn parse_stmt(pair: Pair<Rule>) -> Stmt {
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

/// `pest` のパース結果から式を解析してASTの `Expr` へ変換する
///
/// 数値リテラル、文字列リテラル、および識別子（変数名など）を解析する
/// 演算子を用いた複合式やメソッドチェーンの解析もここで処理する予定
///
/// # Args
///
/// * `pair`: `Rule::expr` にマッチした `pest::iterators::Pair`。
/// # Returns
///
/// 解析された式を表す `Expr` 列挙型のバリアント
/// または未知のルールに遭遇した場合は、フォールバックとして `Expr::Ident("unknown")`
pub fn parse_expr(pair: Pair<Rule>) -> Expr {
    match pair.as_rule() {
        Rule::expr => {
            let mut inner = pair.into_inner();
            let first_term = parse_expr(inner.next().unwrap());

            // 演算子が続く場合の処理
            if let Some(op_pair) = inner.next() {
                let op = match op_pair.as_str() {
                    "+" => Op::Add,
                    "-" => Op::Sub,
                    "in" => Op::In,
                    "??" => Op::Question,
                    _ => todo!("Undefined op: {}", op_pair.as_str()),
                };
                let right_term = parse_expr(inner.next().unwrap());
                Expr::BinaryOp {
                    left: Box::new(first_term),
                    op,
                    right: Box::new(right_term),
                }
            } else {
                first_term
            }
        }
        Rule::term => {
            // termの中身（call_chainかconstant）を取り出す
            let inner_pair = pair.into_inner().next().unwrap();
            parse_expr(inner_pair)
        }
        Rule::constant => {
            let inner_pair = pair.into_inner().next().unwrap();
            parse_expr(inner_pair)
        }
        Rule::call_chain => {
            let mut inner = pair.into_inner();
            let head_pair = inner.next().unwrap();
            let head = head_pair.as_str().to_string();

            // アクセサ（.propertyや()メソッド呼び出し）の解析
            let mut tails = Vec::new();
            for accessor_pair in inner {
                match accessor_pair.as_rule() {
                    Rule::property_access => {
                        let prop_name = accessor_pair.into_inner().next().unwrap().as_str().to_string();
                        tails.push(Accessor::Property(prop_name));
                    }
                    Rule::method_call => {
                        let mut args = Vec::new();
                        for arg_pair in accessor_pair.into_inner() {
                            args.push(parse_expr(arg_pair));
                        }
                        tails.push(Accessor::Method(args));
                    }
                    _ => {
                        // TODO: child_accessとかも実装
                        println!("Skip: {:?}", accessor_pair);
                    }
                }
            }

            // アクセサが何もなければ、それはただの単一の識別子なので、Expr::Identとして返す
            if tails.is_empty() && head_pair.as_rule() == Rule::ident {
                Expr::Ident(head)
            } else {
                Expr::CallChain { head, tails }
            }
        }
        Rule::integer => Expr::Integer(pair.as_str().parse().unwrap()),
        Rule::string => Expr::String(pair.into_inner().next().unwrap().as_str().to_string()),
        Rule::ident => Expr::Ident(pair.as_str().to_string()),
        _ => {
            panic!("parse_expr: Undefined rule: {:?}", pair.as_rule());
        }
    }
}

/// 変数の有効範囲を制限する構文を解析して `RangeLimit` 構造体へ変換する
///
/// `state` 修飾子が付与された変数に対して、値の最小値・最大値、および上限/下限に達した際の挙動（ループするかどうか）を定義するために使用する
///
/// # Args
///
/// * `pair`: `Rule::range_limit` にマッチした `pest::iterators::Pair`
///
/// # Returns
///
/// 開始式、終了式、およびサイクル設定を格納した `RangeLimit` 構造体
fn parse_range_limit(pair: Pair<Rule>) -> RangeLimit {
    let mut inner = pair.into_inner();
    let start = parse_expr(inner.next().unwrap());
    let end = parse_expr(inner.next().unwrap());
    let cycle = inner.next().is_some();

    RangeLimit { start, end, cycle }
}