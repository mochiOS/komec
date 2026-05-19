use std::path::Path;
use clang::{Clang, EntityKind, Index, TypeKind};
use inkwell::module::Module;
use log::*;

pub struct LibraryManager {
    clang: Clang,
}

impl LibraryManager {
    pub fn new() -> Self {
        LibraryManager {
            clang: Clang::new().expect("Failed to initialize clang"),
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

        // Clangでヘッダファイルをパース
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

                        let llvm_ret = match result_type.get_kind() {
                            TypeKind::Int => context.i32_type(),
                            _ => context.i32_type(),
                        };

                        // 可変長引数（printfなど）かどうかの判定
                        let is_variadic = func_type.is_variadic();

                        // LLVMモジュールに関数を登録
                        let fn_type = llvm_ret.fn_type(&llvm_args, is_variadic);
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