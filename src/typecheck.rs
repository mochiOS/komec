use crate::ast::{Expr, FnParam, Op, Stmt};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Void,
    Int,
    Bool,
    Ptr,
    Unknown,
}

fn parse_type(name: &str) -> Type {
    match name {
        "Void" => Type::Void,
        "Int" | "i32" => Type::Int,
        "Bool" => Type::Bool,
        "Ptr" | "Any" | "String" => Type::Ptr,
        _ => Type::Unknown,
    }
}

fn type_of_expr(expr: &Expr, env: &HashMap<String, Type>) -> Type {
    match expr {
        Expr::Integer(_) => Type::Int,
        Expr::String(_) => Type::Ptr,
        Expr::Ident(name) => *env.get(name).unwrap_or(&Type::Unknown),
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
                _ => Type::Unknown,
            }
        }
        Expr::Block(_) => Type::Void,
        Expr::CallChain { .. } => Type::Unknown,
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

            for r in returns.iter() {
                match (ret, r) {
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
        }
    }
    Ok(())
}

