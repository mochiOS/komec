use crate::position::{position_to_byte_offset, span_to_range};
use kome_ast::Span;
use kome_ast::declarations::{Declaration, Module, UseImport};
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

            let definition_span = symbol.definition_span()?;

            return Some(Location {
                uri: document_uri.clone(),
                range: span_to_range(document_source, definition_span),
            });
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
    standard_symbol_definition(&target_reference.name, &standard_modules)
}

fn standard_symbol_definition(name: &str, modules: &[LoadedModule]) -> Option<Location> {
    for loaded in modules {
        let Some(definition_span) = find_top_level_definition(&loaded.module, name) else {
            continue;
        };

        return loaded_location(loaded, definition_span);
    }

    None
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
            let UseImport::Module(path) = import else {
                continue;
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

            let loaded = find_imported_module(standard_library.root(), modules, &segments)?;

            return loaded_location(loaded, Span::new(0, 0));
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
        return modules
            .iter()
            .find(|loaded| loaded.path == standard_library_root.join("prelude.kome"));
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
