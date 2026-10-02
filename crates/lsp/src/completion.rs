//! Completion support for Kome module and import names.

use crate::position::position_to_byte_offset;
use kome_ast::declarations::{Declaration, Module, PathSegmentKind, UseImport, Visibility};
use komec::stdlib::{LoadedModule, StandardLibrary};
use std::collections::BTreeSet;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

/// Returns visible module and declaration completions at a document position.
pub fn completion_at(source: &str, position: Position) -> Vec<CompletionItem> {
    let Some(offset) = position_to_byte_offset(source, position) else {
        return Vec::new();
    };
    let (qualifier, prefix) = completion_context(source, offset);
    let parsed_source = if qualifier.is_some() && prefix.is_empty() {
        let mut completed = source.to_owned();
        completed.insert_str(offset, "__completion");
        completed
    } else {
        source.to_owned()
    };
    let Ok(application) = kome_parser::parse(&parsed_source) else {
        return Vec::new();
    };
    let Ok(standard_library) = StandardLibrary::discover() else {
        return local_completions(&application, &prefix);
    };
    let Ok(modules) = standard_library.modules_for(&application) else {
        return local_completions(&application, &prefix);
    };

    if let Some(qualifier) = qualifier {
        let Some(module) = imported_module(&application, &modules, &qualifier) else {
            return Vec::new();
        };
        return exported_names(module, &modules)
            .into_iter()
            .filter(|name| name.starts_with(&prefix))
            .map(|label| CompletionItem {
                label,
                kind: Some(CompletionItemKind::REFERENCE),
                ..CompletionItem::default()
            })
            .collect();
    }

    let mut completions = local_completions(&application, &prefix);
    for declaration in &application.declarations {
        let Declaration::Use(declaration) = declaration else {
            continue;
        };
        for import in &declaration.imports {
            let label = match import {
                UseImport::Module(path) => identifier_segments(path).last().cloned(),
                UseImport::AliasedModule { alias, .. } => match &alias.kind {
                    PathSegmentKind::Ident(name) => Some(name.clone()),
                    _ => None,
                },
                UseImport::WildcardFrom { path, .. } => {
                    let segments = identifier_segments(path);
                    let module = modules.iter().find(|module| module.name == segments);
                    if let Some(module) = module {
                        completions.extend(
                            exported_names(module, &modules)
                                .into_iter()
                                .filter(|name| name.starts_with(&prefix))
                                .map(|label| CompletionItem {
                                    label,
                                    kind: Some(CompletionItemKind::REFERENCE),
                                    ..CompletionItem::default()
                                }),
                        );
                    }
                    None
                }
                UseImport::Wildcard { .. } => None,
            };
            if let Some(label) = label.filter(|label| label.starts_with(&prefix)) {
                completions.push(CompletionItem {
                    label,
                    kind: Some(CompletionItemKind::MODULE),
                    ..CompletionItem::default()
                });
            }
        }
    }
    completions.sort_by(|left, right| left.label.cmp(&right.label));
    completions.dedup_by(|left, right| left.label == right.label);
    completions
}

fn completion_context(source: &str, offset: usize) -> (Option<String>, String) {
    let before = &source[..offset];
    let prefix_start = before
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_alphanumeric() && *character != '_')
        .map_or(0, |(index, character)| index + character.len_utf8());
    let prefix = before[prefix_start..].to_owned();
    let path = &before[..prefix_start];
    let Some(path) = path.strip_suffix("::") else {
        return (None, prefix);
    };
    let qualifier_start = path
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !character.is_alphanumeric() && *character != '_' && *character != ':'
        })
        .map_or(0, |(index, character)| index + character.len_utf8());
    (Some(path[qualifier_start..].to_owned()), prefix)
}

fn local_completions(module: &Module, prefix: &str) -> Vec<CompletionItem> {
    local_declaration_names(module)
        .into_iter()
        .filter(|name| name.starts_with(prefix))
        .map(|label| CompletionItem {
            label,
            kind: Some(CompletionItemKind::REFERENCE),
            ..CompletionItem::default()
        })
        .collect()
}

fn local_declaration_names(module: &Module) -> BTreeSet<String> {
    module
        .declarations
        .iter()
        .filter_map(|declaration| match declaration {
            Declaration::Function(value) => Some(value.name.clone()),
            Declaration::Component(value) => Some(value.name.clone()),
            Declaration::Struct(value) => Some(value.name.clone()),
            Declaration::Trait(value) => Some(value.name.clone()),
            Declaration::Enum(value) => Some(value.name.clone()),
            Declaration::Constant(value) => match &value.pattern {
                kome_ast::patterns::Pattern::Ident(value) => Some(value.name.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn imported_module<'a>(
    application: &Module,
    modules: &'a [LoadedModule],
    qualifier: &str,
) -> Option<&'a LoadedModule> {
    if qualifier.contains("::") {
        let segments = qualifier.split("::").map(str::to_owned).collect::<Vec<_>>();
        if let Some(module) = modules.iter().find(|module| module.name == segments) {
            return Some(module);
        }
    }
    for declaration in &application.declarations {
        let Declaration::Use(declaration) = declaration else {
            continue;
        };
        for import in &declaration.imports {
            let (path, local) = match import {
                UseImport::Module(path) => {
                    let segments = identifier_segments(path);
                    let local = segments.last()?.clone();
                    (path, local)
                }
                UseImport::AliasedModule { path, alias } => {
                    let PathSegmentKind::Ident(local) = &alias.kind else {
                        continue;
                    };
                    (path, local.clone())
                }
                _ => continue,
            };
            if local == qualifier {
                let segments = identifier_segments(path);
                if let Some(module) = modules.iter().find(|module| module.name == segments) {
                    return Some(module);
                }
            }
        }
    }
    None
}

fn exported_names(module: &LoadedModule, modules: &[LoadedModule]) -> BTreeSet<String> {
    let mut names = declaration_names(&module.module);
    for declaration in &module.module.declarations {
        let Declaration::Use(declaration) = declaration else {
            continue;
        };
        if declaration.visibility != Visibility::Public {
            continue;
        }
        for import in &declaration.imports {
            match import {
                UseImport::Module(path) => {
                    if let Some(name) = identifier_segments(path).last() {
                        names.insert(name.clone());
                    }
                }
                UseImport::AliasedModule { alias, .. } => {
                    if let PathSegmentKind::Ident(name) = &alias.kind {
                        names.insert(name.clone());
                    }
                }
                UseImport::WildcardFrom { path, .. } => {
                    let segments = identifier_segments(path);
                    if let Some(target) = modules.iter().find(|module| module.name == segments) {
                        names.extend(declaration_names(&target.module));
                    }
                }
                UseImport::Wildcard { .. } => {}
            }
        }
    }
    names
}

fn declaration_names(module: &Module) -> BTreeSet<String> {
    module
        .declarations
        .iter()
        .filter_map(|declaration| match declaration {
            Declaration::Function(value) if value.visibility == Visibility::Public => {
                Some(value.name.clone())
            }
            Declaration::Component(value) if value.visibility == Visibility::Public => {
                Some(value.name.clone())
            }
            Declaration::Struct(value) if value.visibility == Visibility::Public => {
                Some(value.name.clone())
            }
            Declaration::Trait(value) if value.visibility == Visibility::Public => {
                Some(value.name.clone())
            }
            Declaration::Enum(value) if value.visibility == Visibility::Public => {
                Some(value.name.clone())
            }
            Declaration::Constant(value) if value.visibility == Visibility::Public => {
                match &value.pattern {
                    kome_ast::patterns::Pattern::Ident(value) => Some(value.name.clone()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn identifier_segments(path: &kome_ast::declarations::Path) -> Vec<String> {
    path.segments
        .iter()
        .filter_map(|segment| match &segment.kind {
            PathSegmentKind::Ident(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}
