use std::path::Path;
use clang::{Clang, EntityKind, Index, TypeKind};
use inkwell::module::Module;
use log::*;

pub struct LibraryManager {
    clang: &'static Clang,
}

impl LibraryManager {
    pub fn new() -> Self {
        // Create Clang and leak it to avoid destructor ordering issues between libclang and LLVM
        // (some environments crash in clang/LLVM destructor chains at program exit).
        let clang = Clang::new().expect("Failed to initialize clang");
        let boxed = Box::new(clang);
        let leaked: &'static Clang = Box::leak(boxed);
        LibraryManager {
            clang: leaked,
        }
    }

    /// libcのヘッダーを読み込む
    pub fn load_c_header<'a>(&self, header_name: &str, context: &'a inkwell::context::Context, module: &Module<'a>) -> bool {
        let header_path = if header_name.ends_with(".h") {
            header_name.to_string()
        } else {
            let parts: Vec<&str> = header_name.split('.').collect();
            let actual_name = if parts.len() == 2 && parts[0] == "libc" {
                parts[1]
            } else {
                eprintln!("LibraryManager: Invalid header name format: {}", header_name);
                return false;
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
                    let std_root = std::env::var("KOME_STD_PATH").unwrap_or_else(|_| "./".to_owned());
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
                            eprintln!("LibraryManager: Header not found in system, local or std (recursively): {}.h", actual_name);
                            return false;
                        }
                    }
                }
            }
        };

        if !Path::new(&header_path).exists() {
            eprintln!("LibraryManager: Header file not found on disk: {}", header_path);
            return false;
        }

        // 一部のよく使うヘッダについては libclang に頼らずプロトタイプを手で登録して
        // libclang の TranslationUnit 破棄時の不安定性を回避する
        if header_path.ends_with("stdio.h") || header_name.starts_with("libc.") {
            // printf, puts, fflush, putchar, getchar, vprintf などを簡易登録
            let i32_t = context.i32_type();
            let _void_t = context.void_type();
            let ptr_t = context.ptr_type(inkwell::AddressSpace::from(0));

            // int printf(const char *fmt, ...);
            if module.get_function("printf").is_none() {
                let fn_ty = i32_t.fn_type(&[ptr_t.into()], true);
                module.add_function("printf", fn_ty, None);
                debug!("LibraryManager: Added builtin prototype 'printf'");
            }
            if module.get_function("puts").is_none() {
                let fn_ty = i32_t.fn_type(&[ptr_t.into()], false);
                module.add_function("puts", fn_ty, None);
                debug!("LibraryManager: Added builtin prototype 'puts'");
            }
            if module.get_function("fflush").is_none() {
                let fn_ty = i32_t.fn_type(&[ptr_t.into()], false);
                module.add_function("fflush", fn_ty, None);
                debug!("LibraryManager: Added builtin prototype 'fflush'");
            }
            if module.get_function("putchar").is_none() {
                let fn_ty = i32_t.fn_type(&[i32_t.into()], false);
                module.add_function("putchar", fn_ty, None);
                debug!("LibraryManager: Added builtin prototype 'putchar'");
            }
            if module.get_function("getchar").is_none() {
                let fn_ty = i32_t.fn_type(&[], false);
                module.add_function("getchar", fn_ty, None);
                debug!("LibraryManager: Added builtin prototype 'getchar'");
            }
            if module.get_function("vprintf").is_none() {
                let fn_ty = i32_t.fn_type(&[ptr_t.into()], true);
                module.add_function("vprintf", fn_ty, None);
                debug!("LibraryManager: Added builtin prototype 'vprintf'");
            }

            // 内部的な補助関数名（過去のログで出てきたもの）も追加しておく
            if module.get_function("__uflow").is_none() {
                let fn_ty = i32_t.fn_type(&[], false);
                module.add_function("__uflow", fn_ty, None);
            }
            if module.get_function("__overflow").is_none() {
                let fn_ty = i32_t.fn_type(&[], false);
                module.add_function("__overflow", fn_ty, None);
            }

            return true;
        }

        // bundle や runtime 用のヘッダが要求された場合も、プロトタイプだけ登録しておく
        if header_path.ends_with("bundle.h") || header_name.contains("std.bundle") || header_name.contains("bundle") {
            let void_t = context.void_type();
            let ptr_t = context.ptr_type(inkwell::AddressSpace::from(0));

            // void __kome_runtime_start_loop(void)
            if module.get_function("__kome_runtime_start_loop").is_none() {
                let fn_ty = void_t.fn_type(&[], false);
                module.add_function("__kome_runtime_start_loop", fn_ty, None);
            }
            // void __kome_runtime_process_events(void)
            if module.get_function("__kome_runtime_process_events").is_none() {
                let fn_ty = void_t.fn_type(&[], false);
                module.add_function("__kome_runtime_process_events", fn_ty, None);
            }
            // void __kome_runtime_subscribe(const char*, void*) -- 保守的にポインタで扱う
            if module.get_function("__kome_runtime_subscribe").is_none() {
                let fn_ty = void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
                module.add_function("__kome_runtime_subscribe", fn_ty, None);
            }
            return true;
        }
        

        // デフォルトは既存の clang ベースのパーサを使うが、libclang が環境で不安定な場合は
        // ここで失敗することがあるため安全に扱う必要がある。まずは試行して失敗したら false を返す。
        let index = Index::new(&self.clang, false, false);
        let tu = match index.parser(&header_path).parse() {
            Ok(tu) => tu,
            Err(_) => return false,
        };

        // ASTのトップレベルのノードを取得
        let entity = tu.get_entity();

        // ヘッダ内の全ての定義を走査
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
                        if module.get_function(&func_name).is_none() {
                            module.add_function(&func_name, fn_type, None);
                            debug!("LibraryManager: Loaded function '{}'", func_name);
                        }
                    }
                }
            }
        }
        true
    }
}