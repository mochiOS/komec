//! Package-level Kome reference documentation generation.

use crate::Project;
use kome_ast::declarations::{Declaration, TypeMember, UseImport, Visibility};
use kome_ast::patterns::Pattern;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// Generates Markdown reference documentation for a package library target.
pub fn generate_reference(project: &Project, output: &Path) -> Result<(), String> {
    let package = &project.manifest().package;
    let mut document = format!("# {}\n", package.name);
    if let Some(version) = &package.version {
        let _ = write!(document, "\nバージョン: `{version}`\n");
    }

    for path in project.library_sources()? {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
        let module = kome_parser::parse(&source)
            .map_err(|error| format!("failed to parse `{}`: {error}", path.display()))?;
        let module_name = path.strip_prefix(project.root()).unwrap_or(&path).display();
        let _ = write!(document, "\n## {module_name}\n");

        for declaration in &module.declarations {
            match declaration {
                Declaration::Struct(value) if value.visibility == Visibility::Public => {
                    write_api(
                        &mut document,
                        3,
                        "構造体",
                        &value.name,
                        &source,
                        value.span.start,
                    );
                    if let Some(fields) = &value.fields {
                        for field in fields
                            .iter()
                            .filter(|field| field.visibility == Visibility::Public)
                        {
                            write_api(
                                &mut document,
                                4,
                                "フィールド",
                                &field.name,
                                &source,
                                field.span.start,
                            );
                        }
                    }
                }
                Declaration::Trait(value) if value.visibility == Visibility::Public => {
                    write_api(
                        &mut document,
                        3,
                        "trait",
                        &value.name,
                        &source,
                        value.span.start,
                    );
                    for function in &value.functions {
                        write_api(
                            &mut document,
                            4,
                            "関数",
                            &function.name,
                            &source,
                            function.span.start,
                        );
                    }
                }
                Declaration::Function(value) if value.visibility == Visibility::Public => {
                    write_api(
                        &mut document,
                        3,
                        "関数",
                        &value.name,
                        &source,
                        value.span.start,
                    );
                }
                Declaration::Component(value) if value.visibility == Visibility::Public => {
                    write_api(
                        &mut document,
                        3,
                        "コンポーネント",
                        &value.name,
                        &source,
                        value.span.start,
                    );
                }
                Declaration::Enum(value) if value.visibility == Visibility::Public => {
                    write_api(
                        &mut document,
                        3,
                        "列挙型",
                        &value.name,
                        &source,
                        value.span.start,
                    );
                }
                Declaration::Constant(value) if value.visibility == Visibility::Public => {
                    if let Pattern::Ident(pattern) = &value.pattern {
                        write_api(
                            &mut document,
                            3,
                            "定数",
                            &pattern.name,
                            &source,
                            value.span.start,
                        );
                    }
                }
                Declaration::For(value) => {
                    let owner = type_name(&value.target);
                    for member in &value.members {
                        match member {
                            TypeMember::Function(function)
                                if function.visibility == Visibility::Public =>
                            {
                                write_api(
                                    &mut document,
                                    3,
                                    "関数",
                                    &format!("{owner}.{}", function.name),
                                    &source,
                                    function.span.start,
                                );
                            }
                            TypeMember::Constant(binding)
                                if binding.visibility == Visibility::Public =>
                            {
                                if let Pattern::Ident(pattern) = &binding.pattern {
                                    write_api(
                                        &mut document,
                                        3,
                                        "関連定数",
                                        &format!("{owner}.{}", pattern.name),
                                        &source,
                                        binding.span.start,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Declaration::Use(value) if value.visibility == Visibility::Public => {
                    for import in &value.imports {
                        let label = match import {
                            UseImport::Module(path) => path_name(path),
                            UseImport::AliasedModule { path, alias } => {
                                let alias = match &alias.kind {
                                    kome_ast::declarations::PathSegmentKind::Ident(name) => name,
                                    _ => continue,
                                };
                                format!("{} as {alias}", path_name(path))
                            }
                            UseImport::WildcardFrom { path, .. } => {
                                format!("{}::*", path_name(path))
                            }
                            UseImport::Wildcard { .. } => "*".to_owned(),
                        };
                        let _ = write!(document, "\n### 再公開 `{label}`\n");
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create `{}`: {error}", parent.display()))?;
    }
    fs::write(output, document)
        .map_err(|error| format!("failed to write `{}`: {error}", output.display()))
}

fn write_api(
    output: &mut String,
    level: usize,
    kind: &str,
    name: &str,
    source: &str,
    start: usize,
) {
    let _ = write!(output, "\n{} {kind} `{name}`\n", "#".repeat(level));
    let documentation = documentation_before(source, start);
    if !documentation.is_empty() {
        let _ = write!(output, "\n{documentation}\n");
    }
}

fn documentation_before(source: &str, start: usize) -> String {
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let mut lines = source[..line_start].lines().collect::<Vec<_>>();
    let mut documentation = Vec::new();
    while let Some(line) = lines.pop() {
        let trimmed = line.trim_start();
        if let Some(content) = trimmed.strip_prefix("///") {
            documentation.push(content.strip_prefix(' ').unwrap_or(content).to_owned());
        } else if trimmed.is_empty() && documentation.is_empty() {
            continue;
        } else {
            break;
        }
    }
    documentation.reverse();
    documentation.join("\n")
}

fn path_name(path: &kome_ast::declarations::Path) -> String {
    path.segments
        .iter()
        .map(|segment| match &segment.kind {
            kome_ast::declarations::PathSegmentKind::Ident(name) => name.as_str(),
            kome_ast::declarations::PathSegmentKind::Self_ => "self",
            kome_ast::declarations::PathSegmentKind::Super => "super",
        })
        .collect::<Vec<_>>()
        .join("::")
}

fn type_name(type_: &kome_ast::types::Type) -> String {
    match type_ {
        kome_ast::types::Type::Named(value) => value.name.clone(),
        _ => "型".to_owned(),
    }
}
