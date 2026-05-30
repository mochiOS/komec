use clang::{Clang, EntityKind, Index, TypeKind};
use inkwell::module::Module;
use inkwell::types::BasicType;
use inkwell::types::BasicTypeEnum;
use log::*;
use std::path::Path;
use std::sync::OnceLock;

pub struct LibraryManager {}

impl LibraryManager {
    pub fn new() -> Self {
        LibraryManager {}
    }

    fn clang() -> &'static Clang {
        static CLANG: OnceLock<usize> = OnceLock::new();
        let addr = *CLANG.get_or_init(|| {
            let clang = Clang::new().expect("Failed to initialize clang");
            Box::into_raw(Box::new(clang)) as usize
        });

        unsafe { &*(addr as *const Clang) }
    }

    #[allow(unused)]
    fn try_load_simple_header<'a>(
        &self,
        header_path: &str,
        context: &'a inkwell::context::Context,
        module: &Module<'a>,
    ) -> bool {
        self.try_load_simple_header_collect(header_path, context, module)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    fn try_load_simple_header_collect<'a>(
        &self,
        header_path: &str,
        context: &'a inkwell::context::Context,
        module: &Module<'a>,
    ) -> Option<Vec<String>> {
        let source = match std::fs::read_to_string(header_path) {
            Ok(s) => s,
            Err(_) => return None,
        };

        let void_t = context.void_type();
        let i32_t = context.i32_type();
        let ptr_t = context.ptr_type(inkwell::AddressSpace::from(0));

        // allowlist 用に「ヘッダ内で見つけた関数名」を返す。
        // 既にLLVMモジュールへ登録済みでも、呼び出し許可としては必要。
        let mut seen: Vec<String> = Vec::new();

        for raw_line in source.lines() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if !line.ends_with(';') || !line.contains('(') || !line.contains(')') {
                continue;
            }
            if line.contains("typedef") || line.contains('{') || line.contains('}') {
                continue;
            }

            // Very small C prototype parser: "<ret> <name>(<args>);"
            let (before_paren, after_paren) = match line.split_once('(') {
                Some(v) => v,
                None => continue,
            };
            let args_str = match after_paren.rsplit_once(')') {
                Some((a, _)) => a.trim(),
                None => continue,
            };

            let before_paren = before_paren.trim_end_matches(';').trim();
            let mut head_parts = before_paren.split_whitespace().collect::<Vec<_>>();
            if head_parts.len() < 2 {
                continue;
            }

            let name = head_parts.pop().unwrap();
            let ret_raw = head_parts.join(" ");
            let ret_norm = ret_raw
                .split_whitespace()
                .filter(|t| {
                    !matches!(
                        *t,
                        "extern"
                            | "static"
                            | "inline"
                            | "__inline"
                            | "__extern"
                            | "__extension__"
                            | "const"
                            | "__const"
                            | "restrict"
                            | "__restrict"
                            | "volatile"
                            | "register"
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");

            let ret_kind = if ret_norm == "void" {
                None
            } else if ret_norm == "int" || ret_norm.ends_with(" int") || ret_norm.ends_with("int") {
                Some(i32_t.as_basic_type_enum())
            } else {
                Some(ptr_t.as_basic_type_enum())
            };

            let mut arg_types: Vec<BasicTypeEnum<'a>> = Vec::new();
            let mut is_variadic = false;
            let args_trimmed = args_str.trim();
            if !(args_trimmed.is_empty() || args_trimmed == "void") {
                for arg in args_trimmed.split(',') {
                    let arg = arg.trim();
                    if arg == "..." {
                        is_variadic = true;
                        continue;
                    }
                    // Parse type only; ignore parameter name.
                    let arg_type = if arg.contains('*') {
                        ptr_t.as_basic_type_enum()
                    } else if arg.starts_with("int") {
                        i32_t.as_basic_type_enum()
                    } else if arg.starts_with("void") {
                        ptr_t.as_basic_type_enum()
                    } else {
                        // Unknown: treat as pointer
                        ptr_t.as_basic_type_enum()
                    };
                    arg_types.push(arg_type);
                }
            }

            // 既に登録済みでも allowlist には入れたいので、ここで記録する。
            if !seen.iter().any(|v| v == name) {
                seen.push(name.to_string());
            }

            // LLVMモジュールに未登録ならプロトタイプを追加する。
            if module.get_function(name).is_none() {
                let fn_ty = match ret_kind {
                    None => void_t.fn_type(
                        &arg_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                        is_variadic,
                    ),
                    Some(rt) => rt.fn_type(
                        &arg_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                        is_variadic,
                    ),
                };
                module.add_function(name, fn_ty, None);
            }
        }

        Some(seen)
    }

    /// libcのヘッダーを読み込む
    pub fn load_c_header<'a>(
        &self,
        header_name: &str,
        context: &'a inkwell::context::Context,
        module: &Module<'a>,
    ) -> bool {
        self.load_c_header_collect(header_name, context, module)
            .is_some()
    }

    pub fn load_c_header_collect<'a>(
        &self,
        header_name: &str,
        context: &'a inkwell::context::Context,
        module: &Module<'a>,
    ) -> Option<Vec<String>> {
        let header_path = if header_name.ends_with(".h") {
            header_name.to_string()
        } else {
            let parts: Vec<&str> = header_name.split('.').collect();
            let actual_name = if parts.len() == 2 && parts[0] == "libc" {
                parts[1]
            } else {
                eprintln!(
                    "LibraryManager: Invalid header name format: {}",
                    header_name
                );
                return None;
            };

            // まずシステムヘッダを探す
            let system_path = format!("/usr/include/{}.h", actual_name);
            if Path::new(&system_path).exists() {
                system_path
            } else {
                let local_path = format!("./{}.h", actual_name);
                if Path::new(&local_path).exists() {
                    local_path
                } else {
                    let std_root =
                        std::env::var("KOME_STD_PATH").unwrap_or_else(|_| "./".to_owned());
                    let std_base = Path::new(&std_root).join("std");

                    // 再帰探索ヘルパー
                    fn find_in_dir(dir: &Path, target: &str) -> Option<std::path::PathBuf> {
                        let entries = match std::fs::read_dir(dir) {
                            Ok(e) => e,
                            Err(_) => return None,
                        };
                        for entry in entries.filter_map(Result::ok) {
                            let p = entry.path();
                            if p.is_file() {
                                if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
                                    if fname == target {
                                        return Some(p);
                                    }
                                }
                            } else if p.is_dir() {
                                if let Some(found) = find_in_dir(&p, target) {
                                    return Some(found);
                                }
                            }
                        }
                        None
                    }

                    let target_name = format!("{}.h", actual_name);
                    let found = if std_base.exists() {
                        find_in_dir(&std_base, &target_name)
                    } else {
                        None
                    };

                    if let Some(p) = found {
                        p.to_string_lossy().into_owned()
                    } else {
                        // std_root 直下も最後にチェック
                        let alt = Path::new(&std_root).join(&target_name);
                        if alt.exists() {
                            alt.to_string_lossy().into_owned()
                        } else {
                            eprintln!(
                                "LibraryManager: Header not found in system, local or std (recursively): {}.h",
                                actual_name
                            );
                            return None;
                        }
                    }
                }
            }
        };

        if !Path::new(&header_path).exists() {
            eprintln!(
                "LibraryManager: Header file not found on disk: {}",
                header_path
            );
            return None;
        }

        if let Some(names) = self.try_load_simple_header_collect(&header_path, context, module) {
            // 既に同名関数が登録済みで `added` が空になることがある。
            // その場合でも「解析自体は成功」なので clang フォールバックには進まない。
            return Some(names);
        }

        if header_path.starts_with("/usr/include/") {
            return None;
        }

        // それ以外（std/ やローカルヘッダ）は clang ベースのパーサを試す。
        let index = Index::new(Self::clang(), false, false);
        let tu = match index.parser(&header_path).parse() {
            Ok(tu) => tu,
            Err(_) => return None,
        };

        // ASTのトップレベルのノードを取得
        let entity = tu.get_entity();

        // ヘッダ内の全ての定義を走査
        // allowlist 用に「見つけた関数名」を返す（登録済みでも含める）。
        let mut seen: Vec<String> = Vec::new();
        for child in entity.get_children() {
            // 関数宣言だけをピックアップ
            if child.get_kind() == EntityKind::FunctionDecl {
                if let Some(func_name) = child.get_name() {
                    // 特定の関数だけをロード対象にする、あるいは全てロードする
                    if let Some(func_type) = child.get_type() {
                        let result_type = func_type.get_result_type().unwrap();

                        // 引数の型リストを取得
                        let argument_types = func_type.get_argument_types().unwrap();

                        // Clangの型からInkwellの型へ変換
                        let mut llvm_args = Vec::new();
                        for arg in argument_types {
                            match arg.get_kind() {
                                TypeKind::Int => llvm_args.push(context.i32_type().into()),
                                TypeKind::Pointer => {
                                    let ptr = context.ptr_type(inkwell::AddressSpace::from(0));
                                    llvm_args.push(ptr.into());
                                }
                                _ => continue, // TODO: 複雑な型も実装
                            }
                        }

                        // 可変長引数（printfなど）かどうかの判定
                        let is_variadic = func_type.is_variadic();

                        // LLVMモジュールに関数を登録
                        // Return typeをClangのTypeKindに基づいて直接決定する（Void/Int/Pointerを扱う）
                        let fn_type = match result_type.get_kind() {
                            TypeKind::Void => context.void_type().fn_type(&llvm_args, is_variadic),
                            TypeKind::Int => context.i32_type().fn_type(&llvm_args, is_variadic),
                            TypeKind::Pointer => {
                                let ptr = context.ptr_type(inkwell::AddressSpace::from(0));
                                ptr.fn_type(&llvm_args, is_variadic)
                            }
                            _ => {
                                // フォールバック: 未知の型は i32 として扱う
                                context.i32_type().fn_type(&llvm_args, is_variadic)
                            }
                        };
                        if !seen.iter().any(|v| v == &func_name) {
                            seen.push(func_name.clone());
                        }
                        if module.get_function(&func_name).is_none() {
                            module.add_function(&func_name, fn_type, None);
                            debug!("LibraryManager: Loaded function '{}'", func_name);
                        }
                    }
                }
            }
        }
        Some(seen)
    }
}

#[cfg(test)]
mod tests {
    use super::LibraryManager;
    use inkwell::context::Context;
    use std::path::Path;

    #[test]
    fn libc_stdio_loads_printf_without_clang_fallback() {
        if !Path::new("/usr/include/stdio.h").exists() {
            return;
        }

        let context = Context::create();
        let module = context.create_module("test");
        let mgr = LibraryManager::new();

        let names = mgr.load_c_header_collect("libc.stdio", &context, &module);
        assert!(names.is_some(), "failed to load libc.stdio header");
        assert!(
            module.get_function("printf").is_some(),
            "printf prototype was not registered"
        );
    }
}
