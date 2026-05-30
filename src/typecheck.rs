use crate::ast::{Expr, FnParam, MatchPat, Op, Stmt};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Void,
    Int,
    Bool,
    Ptr,
    Optional(Box<Type>),
    NoneLit,
    Unknown,
}

fn parse_type(name: &str) -> Type {
    let s = name.trim();
    if let Some(inner) = s.strip_suffix('?') {
        return Type::Optional(Box::new(parse_type(inner)));
    }
    match s {
        "Void" | "none" | "None" => Type::Void,
        "Int" | "i32" | "int" => Type::Int,
        "Bool" | "bool" => Type::Bool,
        "Ptr" | "Any" | "String" | "string" => Type::Ptr,
        _ => Type::Unknown,
    }
}

fn type_of_expr(expr: &Expr, env: &HashMap<String, Type>) -> Type {
    match expr {
        Expr::Integer(_) => Type::Int,
        Expr::String(_) => Type::Ptr,
        Expr::Bool(_) => Type::Bool,
        Expr::None => Type::NoneLit,
        Expr::Ident(name) => {
            // "_" は値を捨てるためのプレースホルダ
            if name == "_" {
                Type::Ptr
            } else {
                env.get(name).cloned().unwrap_or(Type::Unknown)
            }
        }
        Expr::BinaryOp { left, op, right } => {
            let lt = type_of_expr(left, env);
            let rt = type_of_expr(right, env);
            match op {
                Op::Add | Op::Sub | Op::Mul | Op::Div => {
                    if lt == Type::Int && rt == Type::Int {
                        Type::Int
                    } else {
                        Type::Unknown
                    }
                }
                Op::Eq | Op::Neq | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    if lt == rt && lt != Type::Unknown {
                        Type::Bool
                    } else {
                        Type::Unknown
                    }
                }
                Op::And | Op::Or => {
                    if lt == Type::Bool && rt == Type::Bool {
                        Type::Bool
                    } else {
                        Type::Unknown
                    }
                }
                Op::With => lt,
                Op::Question => {
                    // ?? は `T? ?? T -> T`（none のときは右）
                    match (&lt, &rt) {
                        (Type::Optional(inner), t) if inner.as_ref() == t => (*t).clone(),
                        (Type::Optional(inner), Type::NoneLit) => (*inner.as_ref()).clone(),
                        (Type::NoneLit, t) => t.clone(),
                        _ => Type::Unknown,
                    }
                }
                _ => Type::Unknown,
            }
        }
        Expr::IfExpr {
            condition,
            then_body,
            else_body,
        } => {
            let ct = type_of_expr(condition, env);
            if ct != Type::Unknown && ct != Type::Bool {
                return Type::Unknown;
            }
            let tt = type_of_block(then_body, env);
            let et = type_of_block(else_body, env);
            if tt == et {
                tt
            } else {
                Type::Unknown
            }
        }
        Expr::IsExpr { value, pat, then_expr } => {
            // `is` 式は「一致したら値を返し、そうでなければ none(ptr)」の想定
            // ここは厳密化する余地があるが、当面は ptr 扱いにする。
            let _ = (value, pat);
            let tt = type_of_expr(then_expr, env);
            if tt == Type::Ptr || tt == Type::Unknown {
                Type::Ptr
            } else {
                Type::Unknown
            }
        }
        Expr::Block(_) => Type::Void,
        Expr::CallChain { .. } => Type::Unknown,
    }
}

fn type_of_block(stmts: &[Stmt], env: &HashMap<String, Type>) -> Type {
    // 仕様: ブロック式の値は「最後の式」
    match stmts.last() {
        Some(Stmt::ExprStmt(e)) => type_of_expr(e, env),
        _ => Type::Void,
    }
}

fn typecheck_stmt(stmt: &Stmt, env: &HashMap<String, Type>) -> Result<(), String> {
    match stmt {
        Stmt::Match { value, arms } => {
            let vt = type_of_expr(value, env);
            for (pat, _body) in arms {
                match pat {
                    MatchPat::Wildcard => {}
                    MatchPat::Integer(_) => {
                        if vt != Type::Int && vt != Type::Unknown {
                            return Err("match の値が int ではありません。".to_string());
                        }
                    }
                    MatchPat::String(_) => {
                        if vt != Type::Ptr && vt != Type::Unknown {
                            return Err("match の値が string/ptr ではありません。".to_string());
                        }
                    }
                    MatchPat::Bool(_) => {
                        if vt != Type::Bool && vt != Type::Unknown {
                            return Err("match の値が bool ではありません。".to_string());
                        }
                    }
                    MatchPat::None => {}
                    MatchPat::Variant(_) => {}
                }
            }
            Ok(())
        }
        Stmt::Is { value, pat, .. } => {
            let vt = type_of_expr(value, env);
            match pat {
                MatchPat::Integer(_) => {
                    if vt != Type::Int && vt != Type::Unknown {
                        return Err("is の値が int ではありません。".to_string());
                    }
                }
                MatchPat::String(_) => {
                    if vt != Type::Ptr && vt != Type::Unknown {
                        return Err("is の値が string/ptr ではありません。".to_string());
                    }
                }
                MatchPat::Bool(_) => {
                    if vt != Type::Bool && vt != Type::Unknown {
                        return Err("is の値が bool ではありません。".to_string());
                    }
                }
                _ => {}
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn collect_returns(stmts: &[Stmt], out: &mut Vec<Option<Expr>>) {
    for s in stmts {
        match s {
            Stmt::Return(e) => out.push(e.clone()),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                if let Stmt::Bundle { body, .. } = &**then_body {
                    collect_returns(body, out);
                }
                if let Some(else_b) = else_body {
                    if let Stmt::Bundle { body, .. } = &**else_b {
                        collect_returns(body, out);
                    }
                }
            }
            Stmt::While { body, .. } => {
                if let Stmt::Bundle { body, .. } = &**body {
                    collect_returns(body, out);
                }
            }
            Stmt::For { body, .. } => {
                if let Stmt::Bundle { body, .. } = &**body {
                    collect_returns(body, out);
                }
            }
            Stmt::Bundle { body, .. } | Stmt::Block(body) => collect_returns(body, out),
            Stmt::FnDecl { .. } => {}
            _ => {}
        }
    }
}

fn check_variadic_params(params: &[FnParam]) -> Result<(), String> {
    let count = params.iter().filter(|p| p.is_variadic).count();
    if count == 0 {
        return Ok(());
    }
    if count != 1 {
        return Err("可変長引数は1つだけ指定できます。".to_string());
    }
    if !params.last().is_some_and(|p| p.is_variadic) {
        return Err("可変長引数は最後の引数として指定してください。".to_string());
    }
    Ok(())
}

fn inferred_return_from_last_expr(body: &[Stmt], env: &HashMap<String, Type>) -> Type {
    match body.last() {
        Some(Stmt::ExprStmt(e)) => type_of_expr(e, env),
        Some(Stmt::If { then_body, else_body, .. }) => {
            let Some(else_body) = else_body else {
                return Type::Void;
            };
            let then_stmts = match &**then_body {
                Stmt::Bundle { body, .. } => body.as_slice(),
                Stmt::Block(body) => body.as_slice(),
                other => std::slice::from_ref(other),
            };
            let else_stmts = match &**else_body {
                Stmt::Bundle { body, .. } => body.as_slice(),
                Stmt::Block(body) => body.as_slice(),
                other => std::slice::from_ref(other),
            };
            let tt = type_of_block(then_stmts, env);
            let et = type_of_block(else_stmts, env);
            if tt == et { tt } else { Type::Unknown }
        }
        _ => Type::Void,
    }
}

fn stmt_can_fallthrough(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(_) => false,
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            // else が無い if は必ず fallthrough しうる
            let Some(else_body) = else_body else {
                return true;
            };

            let then_stmts = match &**then_body {
                Stmt::Bundle { body, .. } => body.as_slice(),
                Stmt::Block(body) => body.as_slice(),
                other => std::slice::from_ref(other),
            };
            let else_stmts = match &**else_body {
                Stmt::Bundle { body, .. } => body.as_slice(),
                Stmt::Block(body) => body.as_slice(),
                other => std::slice::from_ref(other),
            };

            let then_can = block_can_fallthrough(then_stmts);
            let else_can = block_can_fallthrough(else_stmts);
            then_can || else_can
        }
        Stmt::While { .. } | Stmt::For { .. } => {
            // ループは「実行されない」可能性があるので fallthrough するとみなす
            true
        }
        Stmt::Bundle { body, .. } | Stmt::Block(body) => block_can_fallthrough(body),
        Stmt::FnDecl { .. } => true,
        _ => true,
    }
}

fn block_can_fallthrough(stmts: &[Stmt]) -> bool {
    for s in stmts {
        if !stmt_can_fallthrough(s) {
            return false;
        }
    }
    true
}

pub fn typecheck_program(stmts: &[Stmt]) -> Result<(), String> {
    for s in stmts {
        if let Stmt::FnDecl {
            name,
            params,
            return_ty,
            body,
            ..
        } = s
        {
            check_variadic_params(params).map_err(|e| format!("fn {name}: {e}"))?;

            let ret = return_ty
                .as_deref()
                .map(parse_type)
                .unwrap_or(Type::Void);

            let mut env: HashMap<String, Type> = HashMap::new();
            for p in params {
                if p.is_variadic {
                    // variadic param は配列として扱う予定だが、ここでは Unknown にしておく
                    env.insert(p.name.clone(), Type::Unknown);
                } else {
                    env.insert(p.name.clone(), parse_type(&p.ty));
                }
            }

            let mut returns: Vec<Option<Expr>> = Vec::new();
            collect_returns(body, &mut returns);

            for st in body {
                typecheck_stmt(st, &env).map_err(|e| format!("fn {name}: {e}"))?;
            }

            for r in returns.iter() {
                match (ret.clone(), r) {
                    (Type::Void, None) => {}
                    (Type::Void, Some(_)) => {
                        return Err(format!("fn {name}: Void 関数で値を return しています。"));
                    }
                    (expected, None) => {
                        return Err(format!("fn {name}: `{expected:?}` 戻り値の関数で `return` の値がありません。"));
                    }
                    (expected, Some(expr)) => {
                        let got = type_of_expr(expr, &env);
                        if got != Type::Unknown && got != expected {
                            return Err(format!(
                                "fn {name}: return 型不一致: expected={expected:?}, got={got:?}"
                            ));
                        }
                    }
                }
            }

            // 仕様: `return` は任意で、最後の式が戻り値になる
            if ret != Type::Void {
                let has_implicit = matches!(body.last(), Some(Stmt::ExprStmt(_)));
                let has_implicit_if = matches!(
                    body.last(),
                    Some(Stmt::If {
                        else_body: Some(_),
                        ..
                    })
                );
                if has_implicit {
                    let inferred = inferred_return_from_last_expr(body, &env);
                    if inferred != Type::Unknown && inferred != ret {
                        return Err(format!(
                            "fn {name}: 最後の式の型が戻り値と一致しません: expected={ret:?}, got={inferred:?}"
                        ));
                    }
                }
                if !has_implicit && has_implicit_if {
                    let inferred = inferred_return_from_last_expr(body, &env);
                    if inferred != Type::Unknown && inferred != ret {
                        return Err(format!(
                            "fn {name}: 最後の if の型が戻り値と一致しません: expected={ret:?}, got={inferred:?}"
                        ));
                    }
                }
                if !has_implicit && !has_implicit_if {
                    // 末尾が式でない場合、明示 return が全パスで必要
                    if block_can_fallthrough(body) {
                        return Err(format!(
                            "fn {name}: `{ret:?}` 戻り値の関数は全てのコードパスで return が必要です。"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}
