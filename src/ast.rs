use crate::Rule;
use pest::iterators::Pair;
use std::fmt::Debug;

/// ASTの定義
#[derive(Debug, Clone)]
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
        is_public: bool,
        is_state: bool,
        is_mut: bool,
        name: String,
        value: Expr,
        range: Option<RangeLimit>,
    },
    ExprStmt(Expr),
    FnDecl {
        is_public: bool,
        name: String,
        params: Vec<FnParam>,
        body: Vec<Stmt>,
    },
    If {
        condition: Expr,
        then_body: Box<Stmt>,
        else_body: Option<Box<Stmt>>,
    },
    While {
        condition: Expr,
        body: Box<Stmt>,
    },
    For {
        init: Expr,
        condition: Expr,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    Recipe {
        is_public: bool,
        name: String,
        state_deps: Vec<String>,
        body: Expr,
    },
    Assignment {
        is_default: bool,
        name: String,
        value: Expr,
    },
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct FnParam {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct RangeLimit {
    pub start: Expr,
    pub end: Expr,
    pub cycle: bool,
}

#[derive(Debug, Clone)]
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
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub enum Accessor {
    Property(String),
    Method(Vec<Expr>, Option<Vec<Stmt>>),   // Option<Vec<Stmt>>はトレイリングクロージャ
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    In,
    Question,
    Or,
    And,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
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
        Rule::stmt => {
            let inner_pair = pair.into_inner().next().unwrap();
            parse_stmt(inner_pair)
        }
        Rule::declaration => {
            let mut inner = pair.into_inner();
            let mut is_state = false;
            let mut is_mut = false;

            let mut next = inner.next().unwrap();

            let mut public = false;
            if next.as_rule() == Rule::visibility {
                public = true;
                next = inner.next().unwrap();
            }

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
                is_public: public,
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
        Rule::expr_stmt => {
            let in_expr = pair.into_inner().next().unwrap();        // expr_stmtの中にある実際の式
            let expr = parse_expr(in_expr);                             // parse_exprを呼び出してExpr型に変換
            Stmt::ExprStmt(expr)
        }
        Rule::fn_decl => {
            let mut inner = pair.into_inner();
            let mut first = inner.next().unwrap();

            let mut is_public = false;
            if first.as_str() == "public" {
                is_public = true;
                first = inner.next().unwrap();
            }

            let name = first.as_str().to_string();
            let mut params: Vec<FnParam> = Vec::new();
            let mut body = Vec::new();

            // Parse all remaining items which could be params, return type, or statements
            for sub_pair in inner {
                match sub_pair.as_rule() {
                    Rule::stmt => {
                        body.push(parse_stmt(sub_pair));
                    }
                    Rule::block => {
                        for stmt_pair in sub_pair.into_inner() {
                            if stmt_pair.as_rule() == Rule::stmt {
                                body.push(parse_stmt(stmt_pair));
                            }
                        }
                    }
                    Rule::param => {
                        let mut p = sub_pair.into_inner();
                        let pname = p.next().map(|x| x.as_str().to_string()).unwrap_or_default();
                        let pty = p.next().map(|x| x.as_str().to_string()).unwrap_or_else(|| "Int".to_string());
                        if !pname.is_empty() {
                            params.push(FnParam { name: pname, ty: pty });
                        }
                    }
                    // Skip parameters and type specifications
                    Rule::type_spec | Rule::path | Rule::ident => {}
                    _ => {}
                }
            }

            Stmt::FnDecl { is_public, name, params, body }
        }

        Rule::if_stmt => {
            let mut inner = pair.into_inner();
            let condition = parse_expr(inner.next().unwrap());

            // then_block
            let then_pair = inner.next().unwrap();
            let mut then_body = Vec::new();
            for stmt_pair in then_pair.into_inner() {
                let inner_stmt = stmt_pair.into_inner().next().unwrap();
                then_body.push(parse_stmt(inner_stmt));
            }

            // else_block
            let mut else_body = None;
            if let Some(else_pair) = inner.next() {
                let mut else_block_stmts = Vec::new();
                for stmt_pair in else_pair.into_inner() {
                    let inner_stmt = stmt_pair.into_inner().next().unwrap();
                    else_block_stmts.push(parse_stmt(inner_stmt));
                }
                else_body = Some(else_block_stmts);
            }

            Stmt::If {
                condition,
                then_body: Box::new(Stmt::Bundle { name: "then".to_string(), body: then_body }),
                else_body: else_body.map(|b| Box::new(Stmt::Bundle { name: "else".to_string(), body: b })),
            }
        }
        Rule::while_stmt => {
            let mut inner = pair.into_inner();
            let condition = parse_expr(inner.next().unwrap());

            // 2つ目の要素（blockルール）を取り出す
            let block_pair = inner.next().unwrap();
            let mut body_stmts = Vec::new();

            // blockの中身({" ~ stmt* ~ "})をめくる
            for stmt_pair in block_pair.into_inner() {
                body_stmts.push(parse_stmt(stmt_pair));
            }

            Stmt::While {
                condition,
                body: Box::new(Stmt::Bundle {
                    name: "while_body".to_string(),
                    body: body_stmts,
                }),
            }
        }
        Rule::assignment => {
            let mut inner = pair.into_inner();
            let mut is_default = false;

            let mut next = inner.next().unwrap();

            // "!default" キーワードがあるかチェック
            if next.as_str() == "!default" {
                is_default = true;
                next = inner.next().unwrap();
            }

            // 変数名 (ident)
            let name = next.as_str().to_string();

            // 演算子 (-= や += などの処理)
            let op_pair = inner.next().unwrap();
            let raw_value_pair = inner.next().unwrap();
            let value = parse_expr(raw_value_pair);

            let final_value = match op_pair.as_rule() {
                Rule::sub_assign | Rule::sub => {
                    Expr::BinaryOp {
                        op: Op::Sub,
                        left: Box::new(Expr::Ident(name.clone())),
                        right: Box::new(value),
                    }
                }
                Rule::add_assign | Rule::add => {
                    Expr::BinaryOp {
                        op: Op::Add,
                        left: Box::new(Expr::Ident(name.clone())),
                        right: Box::new(value),
                    }
                }
                _ => value,
            };

            Stmt::Assignment {
                is_default,
                name,
                value: final_value,
            }
        }

        Rule::for_stmt => {
            let mut inner = pair.into_inner();

            // ループ変数名
            let loop_var = inner.next().unwrap().as_str().to_string();

            // 開始の値
            let start_expr = parse_expr(inner.next().unwrap());

            // 終了の値
            let end_expr = parse_expr(inner.next().unwrap());

            // ループの中身
            let block_pair = inner.next().unwrap();
            let mut body_stmts = Vec::new();
            for stmt_pair in block_pair.into_inner() {
                body_stmts.push(parse_stmt(stmt_pair));
            }

            Stmt::For {
                // 初期化式
                init: start_expr,

                condition: Expr::BinaryOp {
                    op: Op::Lt,
                    left: Box::new(Expr::Ident(loop_var.clone())),
                    right: Box::new(end_expr),
                },

                update: Some(Expr::BinaryOp {
                    op: Op::Add,
                    left: Box::new(Expr::Ident(loop_var.clone())),
                    right: Box::new(Expr::Integer(1)),
                }),

                body: Box::new(Stmt::Bundle {
                    name: "for_body".to_string(),
                    body: body_stmts,
                }),
            }
        }
        Rule::while_stmt => {
            let mut inner = pair.into_inner();
            let condition = parse_expr(inner.next().unwrap());

            let block_pair = inner.next().unwrap();
            let mut body_stmts = Vec::new();
            for stmt_pair in block_pair.into_inner() {
                body_stmts.push(parse_stmt(stmt_pair));
            }

            Stmt::While {
                condition,
                body: Box::new(Stmt::Bundle {
                    name: "while_body".to_string(),
                    body: body_stmts,
                }),
            }
        }
        Rule::bundle_stmt => {
            let mut inner_pairs = pair.into_inner();
            let name = inner_pairs.next().unwrap().as_str().to_string();
            let mut body = Vec::new();
            for stmt_pair in inner_pairs {
                body.push(parse_stmt(stmt_pair));
            }
            Stmt::Bundle { name, body }
        }
        Rule::recipe_stmt => {
            let mut inner_pairs = pair.into_inner();
            let mut is_public = false;

            let mut next_pair = inner_pairs.next().unwrap();
            let is_public = if next_pair.as_rule() == Rule::visibility {
                is_public = true;
                next_pair = inner_pairs.next().unwrap();
                true
            } else {
                false
            };

            // レシピ名
            let name = next_pair.as_str().to_string();

            // 依存するstate変数のリスト（Noneならなし）
            let mut state_deps = Vec::new();

            let mut body_pair = None;
            for p in inner_pairs {
                if p.as_rule() == Rule::ident {
                    state_deps.push(p.as_str().to_string());
                } else {
                    body_pair = Some(p);
                    break;
                }
            }

            let body = parse_expr(body_pair.unwrap());

            Stmt::Recipe {
                is_public,
                name,
                state_deps,
                body,
            }
        }
        Rule::block => {
            let mut body = Vec::new();
            for stmt_pair in pair.into_inner() {
                body.push(parse_stmt(stmt_pair));
            }

            Stmt::ExprStmt(Expr::Block(body))
        }
        _ => {
            println!("Rule: {:?}, Text: '{}'", pair.as_rule(), pair.as_str());
            unreachable!("Undefined: {:?}", pair.as_rule())
        }
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
                    "*" => Op::Mul,
                    "/" => Op::Div,
                    "in" => Op::In,
                    "??" => Op::Question,
                    "||" | "or" => Op::Or,
                    "&&" | "and" => Op::And,
                    "==" => Op::Eq,
                    "!=" => Op::Neq,
                    "<" => Op::Lt,
                    ">" => Op::Gt,
                    "<=" => Op::Le,
                    ">=" => Op::Ge,
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

            let mut tails = Vec::new();
            for accessor_pair in inner {
                let target_pair = if accessor_pair.as_rule() == Rule::child_access {
                    accessor_pair.into_inner().next().unwrap()
                } else {
                    accessor_pair
                };

                match target_pair.as_rule() {
                    Rule::property_access => {
                        let prop_name = target_pair.into_inner().next().unwrap().as_str().to_string();
                        tails.push(Accessor::Property(prop_name));
                    }
                    Rule::method_call => {
                        let inner_method = target_pair.into_inner();
                        let mut args = Vec::new();
                        let mut trailing_closure = None;

                        for sub_item in inner_method {
                            match sub_item.as_rule() {
                                Rule::expr => {
                                    args.push(parse_expr(sub_item));
                                }
                                Rule::block => {
                                    // 後ろにくっついているブロック `{ ... }` を解析
                                    let mut block_stmts = Vec::new();
                                    for stmt_pair in sub_item.into_inner() {
                                        if stmt_pair.as_rule() == Rule::stmt {
                                            block_stmts.push(parse_stmt(stmt_pair));
                                        }
                                    }
                                    trailing_closure = Some(block_stmts);
                                }
                                _ => {}
                            }
                        }

                        tails.push(Accessor::Method(args, trailing_closure));
                    }
                    _ => {}
                }
            }

            if tails.is_empty() && head_pair.as_rule() == Rule::ident {
                Expr::Ident(head)
            } else {
                Expr::CallChain { head, tails }
            }
        }
        Rule::block => {
            let mut body = Vec::new();
            for stmt_pair in pair.into_inner() {
                body.push(parse_stmt(stmt_pair));
            }
            Expr::Block(body)
        },
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
