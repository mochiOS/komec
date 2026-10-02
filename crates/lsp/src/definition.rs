use crate::position::{position_to_byte_offset, span_to_range};
use kome_ast::Span;
use kome_ast::declarations::{Declaration, Module, PathSegmentKind, UseImport, Visibility};
use kome_semantics::resolver::ScopeBuilder;
use kome_semantics::scope::Reference;
use komec::stdlib::{LoadedModule, StandardLibrary};
use std::path::Path;
use tower_lsp::lsp_types::{Location, Position, Url};

/// Resolves the definition location referenced at an LSP document position.
pub fn definition_at(
    document_uri: &Url,
    document_source: &str,
    position: Position,
) -> Option<Location> {
    let application = kome_parser::parse(document_source).ok()?;

    let byte_offset = position_to_byte_offset(document_source, position)?;

    /*
     * Local definitions do not require the standard library.
     */
    let application_resolution = ScopeBuilder::resolve(&application);

    if let Some(reference) = reference_at_offset(&application_resolution.references, byte_offset) {
        if let Some(symbol_id) = reference.resolved_to {
            let symbol = application_resolution.symbols.get(symbol_id)?;
            if let Some(definition_span) = symbol.definition_span() {
                return Some(Location {
                    uri: document_uri.clone(),
                    range: span_to_range(document_source, definition_span),
                });
            }
        }
    }

    let standard_library = StandardLibrary::discover().ok()?;

    let standard_modules = standard_library.modules_for(&application).ok()?;

    /*
     * Import paths are not ordinary name references.
     */
    if let Some(location) = import_definition_at(
        &application,
        byte_offset,
        &standard_library,
        &standard_modules,
    ) {
        return Some(location);
    }

    let target_reference = reference_at_offset(&application_resolution.references, byte_offset)?;

    /*
     * Imported symbols are looked up directly in the loaded module
     * ASTs. This avoids losing source-file identity while modules are
     * combined for semantic analysis.
     */
    let source_name = document_source
        .get(target_reference.span.start..target_reference.span.end)
        .filter(|name| name.contains("::"))
        .unwrap_or(&target_reference.name);
    standard_symbol_definition(source_name, &application, &standard_modules)
}

fn standard_symbol_definition(
    name: &str,
    application: &Module,
    modules: &[LoadedModule],
) -> Option<Location> {
    let (loaded, symbol) = imported_symbol(name, application, modules)?;
    let mut visited = Vec::new();
    let (loaded, definition_span) = exported_definition(loaded, &symbol, modules, &mut visited)?;
    loaded_location(loaded, definition_span)
}

fn exported_definition<'a>(
    loaded: &'a LoadedModule,
    symbol: &str,
    modules: &'a [LoadedModule],
    visited: &mut Vec<(Vec<String>, String)>,
) -> Option<(&'a LoadedModule, Span)> {
    let key = (loaded.name.clone(), symbol.to_owned());
    if visited.contains(&key) {
        return None;
    }
    visited.push(key);

    if let Some(span) = find_top_level_definition(&loaded.module, symbol) {
        return Some((loaded, span));
    }

    for declaration in &loaded.module.declarations {
        let Declaration::Use(use_declaration) = declaration else {
            continue;
        };
        if use_declaration.visibility != Visibility::Public {
            continue;
        }
        for import in &use_declaration.imports {
            match import {
                UseImport::Module(path) | UseImport::AliasedModule { path, .. } => {
                    let segments = identifier_segments(path);
                    let (target_symbol, module_name) = segments.split_last()?;
                    let local = match import {
                        UseImport::AliasedModule { alias, .. } => match &alias.kind {
                            PathSegmentKind::Ident(name) => name,
                            _ => continue,
                        },
                        _ => target_symbol,
                    };
                    if local != symbol {
                        continue;
                    }
                    let target = modules.iter().find(|module| module.name == module_name)?;
                    if let Some(definition) =
                        exported_definition(target, target_symbol, modules, visited)
                    {
                        return Some(definition);
                    }
                }
                UseImport::WildcardFrom { path, .. } => {
                    let segments = identifier_segments(path);
                    let Some(target) = modules.iter().find(|module| module.name == segments) else {
                        continue;
                    };
                    if let Some(definition) = exported_definition(target, symbol, modules, visited)
                    {
                        return Some(definition);
                    }
                }
                UseImport::Wildcard { .. } => {}
            }
        }
    }
    None
}

fn imported_symbol<'a>(
    name: &str,
    application: &Module,
    modules: &'a [LoadedModule],
) -> Option<(&'a LoadedModule, String)> {
    for declaration in &application.declarations {
        let Declaration::Use(use_declaration) = declaration else {
            continue;
        };
        for import in &use_declaration.imports {
            let (path, alias, wildcard) = match import {
                UseImport::Module(path) => (path, None, false),
                UseImport::AliasedModule { path, alias } => {
                    let kome_ast::declarations::PathSegmentKind::Ident(alias) = &alias.kind else {
                        continue;
                    };
                    (path, Some(alias.as_str()), false)
                }
                UseImport::WildcardFrom { path, .. } => (path, None, true),
                UseImport::Wildcard { .. } => continue,
            };
            let segments = identifier_segments(path);
            let module = modules.iter().find(|loaded| loaded.name == segments);
            if let Some(module) = module {
                if wildcard && find_top_level_definition(&module.module, name).is_some() {
                    return Some((module, name.to_owned()));
                }
                let local = alias
                    .unwrap_or_else(|| segments.last().map(String::as_str).unwrap_or_default());
                if let Some(symbol) = name.strip_prefix(&format!("{local}::")) {
                    return Some((module, symbol.to_owned()));
                }
                continue;
            }

            let (symbol, module_segments) = segments.split_last()?;
            let module = modules
                .iter()
                .find(|loaded| loaded.name == module_segments)?;
            let local = alias.unwrap_or(symbol);
            if name == local {
                return Some((module, symbol.clone()));
            }
        }
    }

    let parts = name.split("::").collect::<Vec<_>>();
    for loaded in modules {
        let module_len = loaded.name.len();
        if parts.len() == module_len + 1
            && loaded
                .name
                .iter()
                .map(String::as_str)
                .eq(parts[..module_len].iter().copied())
        {
            return Some((loaded, parts[module_len].to_owned()));
        }
    }
    None
}

fn identifier_segments(path: &kome_ast::declarations::Path) -> Vec<String> {
    path.segments
        .iter()
        .filter_map(|segment| match &segment.kind {
            kome_ast::declarations::PathSegmentKind::Ident(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn find_top_level_definition(module: &Module, name: &str) -> Option<Span> {
    module
        .declarations
        .iter()
        .find_map(|declaration| match declaration {
            Declaration::Function(function) if function.name == name => Some(function.span),

            Declaration::Component(component) if component.name == name => Some(component.span),

            Declaration::Enum(enum_declaration) if enum_declaration.name == name => {
                Some(enum_declaration.span)
            }

            Declaration::Struct(declaration) if declaration.name == name => Some(declaration.span),

            Declaration::Trait(declaration) if declaration.name == name => Some(declaration.span),

            Declaration::Constant(binding)
                if matches!(&binding.pattern, kome_ast::patterns::Pattern::Ident(value) if value.name == name) =>
            {
                Some(binding.span)
            }

            _ => None,
        })
}

fn import_definition_at(
    application: &Module,
    byte_offset: usize,
    standard_library: &StandardLibrary,
    modules: &[LoadedModule],
) -> Option<Location> {
    for declaration in &application.declarations {
        let Declaration::Use(use_declaration) = declaration else {
            continue;
        };

        for import in &use_declaration.imports {
            let path = match import {
                UseImport::Module(path)
                | UseImport::AliasedModule { path, .. }
                | UseImport::WildcardFrom { path, .. } => path,
                UseImport::Wildcard { .. } => continue,
            };

            if !span_contains_cursor(path.span, byte_offset) {
                continue;
            }

            let segments = path
                .segments
                .iter()
                .filter_map(|segment| match &segment.kind {
                    kome_ast::declarations::PathSegmentKind::Ident(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();

            if !segments
                .first()
                .copied()
                .is_some_and(komec::stdlib::is_known_package)
            {
                continue;
            }

            if let Some(loaded) = find_imported_module(standard_library.root(), modules, &segments)
            {
                return loaded_location(loaded, Span::new(0, 0));
            }
            let (symbol, module_segments) = segments.split_last()?;
            let loaded = find_imported_module(standard_library.root(), modules, module_segments)?;
            let mut visited = Vec::new();
            let (definition_module, span) =
                exported_definition(loaded, symbol, modules, &mut visited)?;
            return loaded_location(definition_module, span);
        }
    }

    None
}

fn find_imported_module<'a>(
    standard_library_root: &Path,
    modules: &'a [LoadedModule],
    segments: &[&str],
) -> Option<&'a LoadedModule> {
    let module_segments = segments.get(1..)?;

    if module_segments.is_empty() {
        return None;
    }

    let mut module_base = standard_library_root.to_path_buf();

    for segment in module_segments {
        module_base.push(segment);
    }

    let file_candidate = module_base.with_extension("kome");
    let directory_candidate = module_base.join("mod.kome");

    modules.iter().find(|loaded| {
        paths_equal(&loaded.path, &file_candidate)
            || paths_equal(&loaded.path, &directory_candidate)
    })
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,

        _ => left == right,
    }
}

fn loaded_location(loaded: &LoadedModule, span: Span) -> Option<Location> {
    let path = loaded.path.canonicalize().ok()?;

    let uri = Url::from_file_path(path).ok()?;

    Some(Location {
        uri,
        range: span_to_range(&loaded.source, span),
    })
}

fn reference_at_offset(references: &[Reference], byte_offset: usize) -> Option<&Reference> {
    references
        .iter()
        .filter(|reference| span_contains_cursor(reference.span, byte_offset))
        .min_by_key(|reference| reference.span.end.saturating_sub(reference.span.start))
}

fn span_contains_cursor(span: Span, byte_offset: usize) -> bool {
    if span.start <= byte_offset && byte_offset < span.end {
        return true;
    }

    byte_offset
        .checked_sub(1)
        .is_some_and(|previous| span.start <= previous && previous < span.end)
}
