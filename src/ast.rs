use crate::Rule;
use pest::iterators::Pair;
use std::fmt::Debug;

/// ASTの定義
#[derive(Debug, Clone)]
#[allow(unused)]
pub enum Stmt {
    Import(Vec<String>),
    CInclude(String),
    EnumDecl {
        name: String,
        variants: Vec<EnumVariant>,
    },
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
        ty: Option<String>,
        value: Expr,
        range: Option<RangeLimit>,
    },
    ExprStmt(Expr),
    FnDecl {
        is_public: bool,
        name: String,
        params: Vec<FnParam>,
        return_ty: Option<String>,
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
    Return(Option<Expr>),
    Block(Vec<Stmt>),
    Match {
        value: Expr,
        arms: Vec<(MatchPat, Box<Stmt>)>,
    },
    Is {
        value: Expr,
        pat: MatchPat,
        body: Box<Stmt>,
    },
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub enum MatchPat {
    Wildcard,
    Variant(String),
    Integer(i32),
    String(String),
    Bool(bool),
    None,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct EnumVariant {
    pub name: String,
    pub payload_tys: Vec<String>,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct FnParam {
    pub name: String,
    pub ty: String,
    pub is_variadic: bool,
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
    Bool(bool),
    None,
    BinaryOp {
        left: Box<Expr>,
        op: Op,
        right: Box<Expr>,
    },
    IfExpr {
        condition: Box<Expr>,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    CallChain {
        head: String,
        tails: Vec<Accessor>,
    },
    Record(Vec<(String, Expr)>),
    Block(Vec<Stmt>),
    IsExpr {
        value: Box<Expr>,
        pat: MatchPat,
        then_expr: Box<Expr>,
    },
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub enum Accessor {
    Property(String),
    Index(Expr),
    Method(Vec<CallArg>, Option<ClosureBlock>), // Option<ClosureBlock> はトレイリングクロージャ
    With(String, Vec<(String, Expr)>),
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub enum CallArg {
    Positional(Expr),
    Named(String, Expr),
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct ClosureBlock {
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
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
    With,
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
            let mut ty = None;

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
            let next_after_name = inner.next().unwrap();
            let value = if next_after_name.as_rule() == Rule::type_spec {
                ty = Some(next_after_name.as_str().to_string());
                parse_expr(inner.next().unwrap())
            } else {
                parse_expr(next_after_name)
            };
            let range = inner.next().map(|p| parse_range_limit(p));

            Stmt::Declaration {
                is_public: public,
                is_state,
                is_mut,
                name,
                ty,
                value,
                range,
            }
        }
        Rule::import_stmt => {
            let path = pair
                .into_inner()
                .next()
                .unwrap()
                .into_inner()
                .map(|p| p.as_str().to_string())
                .collect();
            Stmt::Import(path)
        }
        Rule::cinclude_stmt => {
            let s = pair.into_inner().next().unwrap();
            // `string` rule yields inner_str
            let inner = s.into_inner().next().unwrap().as_str().to_string();
            Stmt::CInclude(inner)
        }
        Rule::decorator => {
            let mut inner = pair.into_inner();
            let name = inner.next().unwrap().as_str().to_string();
            let target = inner.next().unwrap().as_str().to_string();
            let mut pairs = Vec::new();
            for p in inner {
                if p.as_rule() != Rule::kv_pair {
                    continue;
                }
                let mut kv = p.into_inner();
                let key = kv.next().unwrap().as_str().to_string();
                let value = parse_expr(kv.next().unwrap());
                pairs.push((key, value));
            }
            Stmt::Decorator { name, target, pairs }
        }
        Rule::enum_stmt => {
            let mut inner = pair.into_inner();
            let name = inner.next().unwrap().as_str().to_string();
            let mut variants: Vec<EnumVariant> = Vec::new();
            for p in inner {
                if p.as_rule() != Rule::enum_variant {
                    continue;
                }
                let mut vi = p.into_inner();
                let vname = vi.next().unwrap().as_str().to_string();
                let mut payload_tys: Vec<String> = Vec::new();
                for ty in vi {
                    match ty.as_rule() {
                        Rule::type_spec | Rule::path | Rule::ident => {
                            payload_tys.push(ty.as_str().to_string());
                        }
                        _ => {}
                    }
                }
                variants.push(EnumVariant {
                    name: vname,
                    payload_tys,
                });
            }
            Stmt::EnumDecl { name, variants }
        }
        Rule::expr_stmt => {
            let in_expr = pair.into_inner().next().unwrap(); // expr_stmtの中にある実際の式
            let expr = parse_expr(in_expr); // parse_exprを呼び出してExpr型に変換
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
            let mut return_ty: Option<String> = None;

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
                        let pname = p
                            .next()
                            .map(|x| x.as_str().to_string())
                            .unwrap_or_default();
                        let pty = p
                            .next()
                            .map(|x| x.as_str().to_string())
                            .unwrap_or_else(|| "Int".to_string());
                        let mut is_variadic = false;
                        for rest in p {
                            if rest.as_rule() == Rule::ellipsis {
                                is_variadic = true;
                            }
                        }
                        if !pname.is_empty() {
                            params.push(FnParam {
                                name: pname,
                                ty: pty,
                                is_variadic,
                            });
                        }
                    }
                    // Skip parameters and type specifications
                    Rule::type_spec => {
                        return_ty = Some(sub_pair.as_str().to_string());
                    }
                    Rule::path | Rule::ident => {}
                    _ => {}
                }
            }

            Stmt::FnDecl {
                is_public,
                name,
                params,
                return_ty,
                body,
            }
        }
        Rule::extend_decl => {
            // `extend bundle.run() { ... }` のような拡張メソッド定義。
            // AST 上は通常の `FnDecl` と同じ形にして、名前だけ `bundle.run` のような path 文字列にする。
            let mut inner = pair.into_inner();
            let name_pair = inner.next().unwrap(); // path
            let name = name_pair.as_str().to_string();

            let mut params: Vec<FnParam> = Vec::new();
            let mut body = Vec::new();
            let mut return_ty: Option<String> = None;

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
                        let pname = p
                            .next()
                            .map(|x| x.as_str().to_string())
                            .unwrap_or_default();
                        let pty = p
                            .next()
                            .map(|x| x.as_str().to_string())
                            .unwrap_or_else(|| "Int".to_string());
                        let mut is_variadic = false;
                        for rest in p {
                            if rest.as_rule() == Rule::ellipsis {
                                is_variadic = true;
                            }
                        }
                        if !pname.is_empty() {
                            params.push(FnParam {
                                name: pname,
                                ty: pty,
                                is_variadic,
                            });
                        }
                    }
                    Rule::type_spec => {
                        return_ty = Some(sub_pair.as_str().to_string());
                    }
                    Rule::path | Rule::ident => {}
                    _ => {}
                }
            }

            Stmt::FnDecl {
                is_public: false,
                name,
                params,
                return_ty,
                body,
            }
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
                then_body: Box::new(Stmt::Bundle {
                    name: "then".to_string(),
                    body: then_body,
                }),
                else_body: else_body.map(|b| {
                    Box::new(Stmt::Bundle {
                        name: "else".to_string(),
                        body: b,
                    })
                }),
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
                Rule::sub_assign | Rule::sub => Expr::BinaryOp {
                    op: Op::Sub,
                    left: Box::new(Expr::Ident(name.clone())),
                    right: Box::new(value),
                },
                Rule::add_assign | Rule::add => Expr::BinaryOp {
                    op: Op::Add,
                    left: Box::new(Expr::Ident(name.clone())),
                    right: Box::new(value),
                },
                _ => value,
            };

            Stmt::Assignment {
                is_default,
                name,
                value: final_value,
            }
        }

        Rule::return_stmt => {
            let expr = pair.into_inner().next().map(parse_expr);
            Stmt::Return(expr)
        }
        Rule::match_stmt => {
            let mut inner = pair.into_inner();
            let value = parse_expr(inner.next().unwrap());
            let mut arms: Vec<(MatchPat, Box<Stmt>)> = Vec::new();
            for arm_pair in inner {
                if arm_pair.as_rule() != Rule::match_arm {
                    continue;
                }
                let mut arm_inner = arm_pair.into_inner();
                let pat_pair = arm_inner.next().unwrap();
                let body_pair = arm_inner.next().unwrap();
                let pat = parse_match_pat(pat_pair);
                let body = parse_match_arm_body(body_pair);
                arms.push((pat, Box::new(body)));
            }
            Stmt::Match { value, arms }
        }
        Rule::is_stmt => {
            let mut inner = pair.into_inner();
            let value = parse_expr(inner.next().unwrap());
            let pat_pair = inner.next().unwrap();
            let body_pair = inner.next().unwrap();
            let pat = parse_match_pat(pat_pair);
            let body = parse_match_arm_body(body_pair);
            Stmt::Is {
                value,
                pat,
                body: Box::new(body),
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

fn parse_match_pat(pair: Pair<Rule>) -> MatchPat {
    match pair.as_rule() {
        Rule::match_pat => {
            let s = pair.as_str().trim();
            if s == "_" {
                return MatchPat::Wildcard;
            }
            if s.starts_with('.') {
                return MatchPat::Variant(s.trim_start_matches('.').to_string());
            }
            if let Some(inner) = pair.into_inner().next() {
                return parse_match_pat(inner);
            }
            panic!("未知の match パターン: {s}");
        }
        Rule::integer => MatchPat::Integer(pair.as_str().parse().unwrap()),
        Rule::string => MatchPat::String(pair.into_inner().next().unwrap().as_str().to_string()),
        Rule::boolean => MatchPat::Bool(pair.as_str() == "true"),
        Rule::none => MatchPat::None,
        _ => {
            let s = pair.as_str().trim();
            if s == "_" {
                MatchPat::Wildcard
            } else if s.starts_with('.') {
                MatchPat::Variant(s.trim_start_matches('.').to_string())
            } else {
                panic!("未知の match パターン: {:?}", pair.as_rule());
            }
        }
    }
}

fn parse_match_arm_body(pair: Pair<Rule>) -> Stmt {
    match pair.as_rule() {
        Rule::match_body => {
            let inner = pair.into_inner().next().unwrap();
            parse_match_arm_body(inner)
        }
        Rule::block => {
            let mut body = Vec::new();
            for stmt_pair in pair.into_inner() {
                body.push(parse_stmt(stmt_pair));
            }
            Stmt::Block(body)
        }
        Rule::stmt => parse_stmt(pair),
        Rule::expr => Stmt::ExprStmt(parse_expr(pair)),
        _ => {
            // pest の展開によっては stmt/expr が直接来ることがある
            match pair.as_rule() {
                Rule::assignment
                | Rule::declaration
                | Rule::if_stmt
                | Rule::match_stmt
                | Rule::is_stmt
                | Rule::while_stmt
                | Rule::for_stmt
                | Rule::return_stmt
                | Rule::expr_stmt => parse_stmt(pair),
                _ => Stmt::ExprStmt(parse_expr(pair)),
            }
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
                let op_str = op_pair.as_str().trim();
                let op = match op_str {
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
                    "with" => Op::With,
                    _ => todo!("Undefined op: {}", op_str),
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
                if accessor_pair.as_rule() == Rule::child_access {
                    let mut w = accessor_pair.into_inner();
                    let name = w.next().unwrap().as_str().to_string();
                    let mut pairs = Vec::new();
                    for p in w {
                        if p.as_rule() != Rule::kv_pair {
                            continue;
                        }
                        let mut kv = p.into_inner();
                        let key = kv.next().unwrap().as_str().to_string();
                        let value = parse_expr(kv.next().unwrap());
                        pairs.push((key, value));
                    }
                    tails.push(Accessor::With(name, pairs));
                    continue;
                }

                let target_pair = accessor_pair;

                match target_pair.as_rule() {
                    Rule::property_access => {
                        let prop_name = target_pair
                            .into_inner()
                            .next()
                            .unwrap()
                            .as_str()
                            .to_string();
                        tails.push(Accessor::Property(prop_name));
                    }
                    Rule::index_access => {
                        let idx_expr = target_pair.into_inner().next().unwrap();
                        tails.push(Accessor::Index(parse_expr(idx_expr)));
                    }
                    Rule::method_call => {
                        let inner_method = target_pair.into_inner();
                        let mut args = Vec::new();
                        let mut trailing_closure = None;

                        for sub_item in inner_method {
                            match sub_item.as_rule() {
                                Rule::call_arg => {
                                    let inner = sub_item.into_inner().next().unwrap();
                                    match inner.as_rule() {
                                        Rule::named_arg => {
                                            let mut named = inner.into_inner();
                                            let key = named.next().unwrap().as_str().to_string();
                                            let value = parse_expr(named.next().unwrap());
                                            args.push(CallArg::Named(key, value));
                                        }
                                        Rule::expr => {
                                            args.push(CallArg::Positional(parse_expr(inner)));
                                        }
                                        Rule::is_stmt => {
                                            let is_stmt = parse_stmt(inner);
                                            let Stmt::Is { value, pat, body } = is_stmt else {
                                                unreachable!("is_stmt should parse into Stmt::Is");
                                            };
                                            // is 式として扱う（then は式のみ対応）
                                            let then_expr = match *body {
                                                Stmt::ExprStmt(e) => e,
                                                Stmt::Block(stmts) => Expr::Block(stmts),
                                                other => {
                                                    // 仕様を単純化: ここは式だけ許可
                                                    panic!("is 引数は式にしてください: {:?}", other);
                                                }
                                            };
                                            args.push(CallArg::Positional(Expr::IsExpr {
                                                value: Box::new(value),
                                                pat,
                                                then_expr: Box::new(then_expr),
                                            }));
                                        }
                                        _ => unreachable!("call_arg inner should be expr or is_stmt"),
                                    }
                                }
                                Rule::closure_block => {
                                    let mut closure_inner = sub_item.into_inner();
                                    let mut params = Vec::new();
                                    let first = closure_inner.next();
                                    let mut first_stmt_pair = None;
                                    if let Some(first_pair) = first {
                                        if first_pair.as_rule() == Rule::closure_params {
                                            for p in first_pair.into_inner() {
                                                if p.as_rule() == Rule::ident {
                                                    params.push(p.as_str().to_string());
                                                }
                                            }
                                            first_stmt_pair = closure_inner.next();
                                        } else if first_pair.as_rule() == Rule::stmt {
                                            first_stmt_pair = Some(first_pair);
                                        }
                                    }

                                    let mut block_stmts = Vec::new();
                                    if let Some(stmt_pair) = first_stmt_pair {
                                        block_stmts.push(parse_stmt(stmt_pair));
                                    }
                                    for stmt_pair in closure_inner {
                                        if stmt_pair.as_rule() == Rule::stmt {
                                            block_stmts.push(parse_stmt(stmt_pair));
                                        }
                                    }
                                    trailing_closure = Some(ClosureBlock {
                                        params,
                                        body: normalize_block_statements(block_stmts),
                                    });
                                }
                                _ => {}
                            }
                        }

                        tails.push(Accessor::Method(
                            normalize_call_args(args),
                            trailing_closure,
                        ));
                    }
                    Rule::property_closure_call => {
                        // `.name { ... }` を `Property(name)` + `Method([], block)` に展開する
                        let mut inner = target_pair.into_inner();
                        let name = inner
                            .next()
                            .expect("property_closure_call name")
                            .as_str()
                            .to_string();
                        let block = inner.next().expect("property_closure_call block");
                        let mut closure_inner = block.into_inner();
                        let mut params = Vec::new();
                        let mut first_stmt_pair = None;
                        if let Some(first_pair) = closure_inner.next() {
                            if first_pair.as_rule() == Rule::closure_params {
                                for p in first_pair.into_inner() {
                                    if p.as_rule() == Rule::ident {
                                        params.push(p.as_str().to_string());
                                    }
                                }
                                first_stmt_pair = closure_inner.next();
                            } else if first_pair.as_rule() == Rule::stmt {
                                first_stmt_pair = Some(first_pair);
                            }
                        }
                        let mut block_stmts = Vec::new();
                        if let Some(stmt_pair) = first_stmt_pair {
                            block_stmts.push(parse_stmt(stmt_pair));
                        }
                        for stmt_pair in closure_inner {
                            if stmt_pair.as_rule() == Rule::stmt {
                                block_stmts.push(parse_stmt(stmt_pair));
                            }
                        }
                        tails.push(Accessor::Property(name));
                        tails.push(Accessor::Method(
                            Vec::new(),
                            Some(ClosureBlock {
                                params,
                                body: normalize_block_statements(block_stmts),
                            }),
                        ));
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
        Rule::record => {
            let mut fields = Vec::new();
            for field in pair.into_inner() {
                if field.as_rule() != Rule::kv_pair {
                    continue;
                }
                let mut kv = field.into_inner();
                let key = kv.next().unwrap().as_str().to_string();
                let value = parse_expr(kv.next().unwrap());
                fields.push((key, value));
            }
            Expr::Record(fields)
        }
        Rule::block => {
            let mut body = Vec::new();
            for stmt_pair in pair.into_inner() {
                body.push(parse_stmt(stmt_pair));
            }
            Expr::Block(body)
        }
        Rule::if_expr => {
            let mut inner = pair.into_inner();
            let condition = parse_expr(inner.next().unwrap());
            let then_block = inner.next().unwrap();
            let else_block = inner.next().unwrap();

            let mut then_body = Vec::new();
            for stmt_pair in then_block.into_inner() {
                then_body.push(parse_stmt(stmt_pair));
            }

            let mut else_body = Vec::new();
            for stmt_pair in else_block.into_inner() {
                else_body.push(parse_stmt(stmt_pair));
            }

            Expr::IfExpr {
                condition: Box::new(condition),
                then_body,
                else_body,
            }
        }
        Rule::integer => Expr::Integer(pair.as_str().parse().unwrap()),
        Rule::string => Expr::String(pair.into_inner().next().unwrap().as_str().to_string()),
        Rule::boolean => Expr::Bool(pair.as_str() == "true"),
        Rule::none => Expr::None,
        Rule::ident => Expr::Ident(pair.as_str().to_string()),
        _ => {
            panic!("parse_expr: Undefined rule: {:?}", pair.as_rule());
        }
    }
}

fn normalize_block_statements(stmts: Vec<Stmt>) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < stmts.len() {
        if i + 2 < stmts.len() {
            if let Stmt::Declaration {
                is_public,
                is_state,
                is_mut,
                name,
                ty,
                value,
                range,
            } = &stmts[i]
            {
                let op_name = match &stmts[i + 1] {
                    Stmt::ExprStmt(Expr::Ident(name)) => name.as_str(),
                    _ => "",
                };
                let op = match op_name {
                    "and" | "&&" => Some(Op::And),
                    "or" | "||" => Some(Op::Or),
                    "??" => Some(Op::Question),
                    "in" => Some(Op::In),
                    "with" => Some(Op::With),
                    _ => None,
                };
                if let Some(op) = op {
                    if let Stmt::ExprStmt(rhs) = &stmts[i + 2] {
                        out.push(Stmt::Declaration {
                            is_public: *is_public,
                            is_state: *is_state,
                            is_mut: *is_mut,
                            name: name.clone(),
                            ty: ty.clone(),
                            value: Expr::BinaryOp {
                                left: Box::new(value.clone()),
                                op,
                                right: Box::new(rhs.clone()),
                            },
                            range: range.clone(),
                        });
                        i += 3;
                        continue;
                    }
                }
            }
        }

        if i + 2 < stmts.len() {
            let lhs = match &stmts[i] {
                Stmt::ExprStmt(expr) => expr.clone(),
                _ => {
                    out.push(stmts[i].clone());
                    i += 1;
                    continue;
                }
            };
            let op_name = match &stmts[i + 1] {
                Stmt::ExprStmt(Expr::Ident(name)) => name.as_str(),
                _ => {
                    out.push(stmts[i].clone());
                    i += 1;
                    continue;
                }
            };
            let op = match op_name {
                "and" | "&&" => Some(Op::And),
                "or" | "||" => Some(Op::Or),
                "??" => Some(Op::Question),
                "in" => Some(Op::In),
                "with" => Some(Op::With),
                _ => None,
            };
            if let Some(op) = op {
                if let Stmt::ExprStmt(rhs) = &stmts[i + 2] {
                    out.push(Stmt::ExprStmt(Expr::BinaryOp {
                        left: Box::new(lhs),
                        op,
                        right: Box::new(rhs.clone()),
                    }));
                    i += 3;
                    continue;
                }
            }
        }

        out.push(stmts[i].clone());
        i += 1;
    }
    out
}

fn normalize_call_args(args: Vec<CallArg>) -> Vec<CallArg> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if i + 2 < args.len() {
            let lhs = match &args[i] {
                CallArg::Positional(expr) => expr.clone(),
                _ => {
                    out.push(args[i].clone());
                    i += 1;
                    continue;
                }
            };
            let op_name = match &args[i + 1] {
                CallArg::Positional(Expr::Ident(name)) => name.as_str(),
                _ => {
                    out.push(args[i].clone());
                    i += 1;
                    continue;
                }
            };
            let op = match op_name {
                "and" | "&&" => Some(Op::And),
                "or" | "||" => Some(Op::Or),
                "??" => Some(Op::Question),
                "in" => Some(Op::In),
                "with" => Some(Op::With),
                _ => None,
            };
            if let Some(op) = op {
                if let CallArg::Positional(rhs) = &args[i + 2] {
                    out.push(CallArg::Positional(Expr::BinaryOp {
                        left: Box::new(lhs),
                        op,
                        right: Box::new(rhs.clone()),
                    }));
                    i += 3;
                    continue;
                }
            }
        }

        out.push(args[i].clone());
        i += 1;
    }
    out
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
