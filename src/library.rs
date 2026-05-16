use clang::{Clang, EntityKind, Index, TypeKind};
use inkwell::module::Module;
use std::path::Path;

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
        let parts: Vec<&str> = header_name.split('.').collect();

        let header_name = match parts.last() {
            Some(part) => part,
            None => return false,
        };

        let header_path = format!("/usr/include/{}.h", header_name);
        if !Path::new(&header_path).exists() {
            eprintln!("LibraryManager: Header not found: {}", header_path);
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
                            println!("LibraryManager: Loaded function '{}'", func_name);
                        }
                    }
                }
            }
        }
        true
    }
}