//! Package modules, imports, visibility checks, and canonical declaration names.

use crate::resolver::ScopeBuilder;
use crate::scope::Symbol;
use kome_ast::Span;
use kome_ast::declarations::{
    Binding, ComponentMember, Declaration, ExternItem, Module, Path, PathSegmentKind, TypeMember,
    UseImport, Visibility,
};
use kome_ast::expressions::{CallArg, Expression, ObjectProperty, PropertyKey, TemplatePart};
use kome_ast::patterns::{IsPattern, Pattern};
use kome_ast::statements::Statement;
use kome_ast::types::Type;
use std::collections::{HashMap, HashSet};
use std::fmt;

/// One parsed source module together with its canonical package path.
#[derive(Debug, Clone)]
pub struct SourceModule {
    /// Package identity used as the namespace root.
    pub package: String,
    /// Module path below the package root.
    pub path: Vec<String>,
    /// Parsed source syntax tree.
    pub module: Module,
    /// Whether declarations should retain application-root names.
    pub application: bool,
}

impl SourceModule {
    /// Creates a source module for namespace linking.
    pub fn new(
        package: impl Into<String>,
        path: Vec<String>,
        module: Module,
        application: bool,
    ) -> Self {
        Self {
            package: package.into(),
            path,
            module,
            application,
        }
    }

    fn canonical_path(&self) -> String {
        std::iter::once(self.package.as_str())
            .chain(self.path.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::")
    }
}

/// A namespace or visibility error produced before ordinary name resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleError {
    /// Human-readable explanation.
    pub message: String,
    /// Source range that caused the error.
    pub span: Span,
}

impl fmt::Display for ModuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at byte range {}..{}",
            self.message, self.span.start, self.span.end
        )
    }
}

impl std::error::Error for ModuleError {}

#[derive(Debug, Clone)]
struct Export {
    module: String,
    package: String,
    visibility: Visibility,
    target: String,
}

#[derive(Default)]
struct ModuleIndex {
    modules: HashSet<String>,
    declarations: HashMap<String, Export>,
    module_declarations: HashMap<String, Vec<(String, Export)>>,
}

/// Links source modules into the flat syntax tree consumed by later compiler passes.
///
/// Declaration and reference names are canonicalized before the modules are combined,
/// so identical local names from different packages cannot collide.
pub fn link_modules(mut sources: Vec<SourceModule>) -> Result<Module, Vec<ModuleError>> {
    let (index, mut errors) = build_index(&sources);
    validate_import_cycles(&sources, &index, &mut errors);
    let mut declarations = Vec::new();
    let span = sources
        .iter()
        .find(|source| source.application)
        .map_or(Span::new(0, 0), |source| source.module.span);

    for source in &mut sources {
        let module_name = source.canonical_path();
        let local_names = index
            .module_declarations
            .get(&module_name)
            .into_iter()
            .flatten()
            .filter(|(name, export)| export.target == format!("{module_name}::{name}"))
            .map(|(name, _)| {
                let canonical = if source.application {
                    name.clone()
                } else {
                    format!("{module_name}::{name}")
                };
                (name.clone(), canonical)
            })
            .collect::<HashMap<_, _>>();
        let resolution = ScopeBuilder::resolve(&source.module);
        let reference_kinds = resolution
            .references
            .iter()
            .filter_map(|reference| {
                let symbol = reference
                    .resolved_to
                    .and_then(|id| resolution.symbols.get(id))?;
                Some((
                    (reference.span.start, reference.span.end),
                    symbol_kind(symbol),
                ))
            })
            .collect();
        let imports = collect_imports(source, &index, &mut errors);
        let mut source_declarations = std::mem::take(&mut source.module.declarations);
        let mut rewriter = Rewriter {
            source,
            index: &index,
            local_names,
            reference_kinds,
            module_aliases: imports.0,
            item_aliases: imports.1,
            errors: &mut errors,
        };
        for declaration in &mut source_declarations {
            rewriter.rewrite_declaration(declaration, true);
        }
        drop(rewriter);
        for declaration in &source_declarations {
            validate_public_api(declaration, &index, &mut errors);
        }
        declarations.extend(
            source_declarations
                .into_iter()
                .filter(|declaration| !matches!(declaration, Declaration::Use(_))),
        );
    }

    if errors.is_empty() {
        Ok(Module::new(declarations, span))
    } else {
        Err(errors)
    }
}

fn validate_import_cycles(
    sources: &[SourceModule],
    index: &ModuleIndex,
    errors: &mut Vec<ModuleError>,
) {
    let mut graph: HashMap<String, Vec<(String, Span)>> = HashMap::new();
    for source in sources {
        let module = source.canonical_path();
        let edges = graph.entry(module.clone()).or_default();
        for declaration in &source.module.declarations {
            let Declaration::Use(declaration) = declaration else {
                continue;
            };
            for import in &declaration.imports {
                let (path, span) = match import {
                    UseImport::Module(path) | UseImport::AliasedModule { path, .. } => {
                        (path, path.span)
                    }
                    UseImport::WildcardFrom { path, span } => (path, *span),
                    UseImport::Wildcard { .. } => continue,
                };
                let canonical = normalize_path(source, path);
                let target = if index.modules.contains(&canonical) {
                    Some(canonical)
                } else {
                    index
                        .declarations
                        .get(&canonical)
                        .map(|export| export.module.clone())
                };
                if let Some(target) = target
                    && target != module
                {
                    edges.push((target, span));
                }
            }
        }
    }

    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();
    let mut stack = Vec::new();
    for source in sources {
        let module = source.canonical_path();
        find_import_cycle(
            &module,
            &graph,
            &mut visited,
            &mut visiting,
            &mut stack,
            errors,
        );
    }
}

fn find_import_cycle(
    module: &str,
    graph: &HashMap<String, Vec<(String, Span)>>,
    visited: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
    stack: &mut Vec<String>,
    errors: &mut Vec<ModuleError>,
) {
    if visited.contains(module) || !visiting.insert(module.to_owned()) {
        return;
    }
    stack.push(module.to_owned());

    if let Some(edges) = graph.get(module) {
        for (target, span) in edges {
            if let Some(start) = stack.iter().position(|entry| entry == target) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(target.clone());
                errors.push(ModuleError {
                    message: format!("cyclic module import: {}", cycle.join(" -> ")),
                    span: *span,
                });
                continue;
            }
            find_import_cycle(target, graph, visited, visiting, stack, errors);
        }
    }

    stack.pop();
    visiting.remove(module);
    visited.insert(module.to_owned());
}

fn validate_public_api(
    declaration: &Declaration,
    index: &ModuleIndex,
    errors: &mut Vec<ModuleError>,
) {
    match declaration {
        Declaration::Function(value) => {
            validate_function_api(value, value.visibility, index, errors)
        }
        Declaration::Struct(value) => {
            if let Some(fields) = &value.fields {
                for field in fields {
                    if visibility_rank(field.visibility) > visibility_rank(value.visibility) {
                        errors.push(ModuleError {
                            message: format!(
                                "field `{}` is more visible than struct `{}`",
                                field.name, value.name
                            ),
                            span: field.span,
                        });
                    }
                    validate_exposed_type(&field.type_, field.visibility, index, errors);
                }
            }
        }
        Declaration::Trait(value) => {
            for function in &value.functions {
                validate_function_api(function, value.visibility, index, errors);
            }
        }
        Declaration::For(value) => {
            for member in &value.members {
                match member {
                    TypeMember::Function(function) => {
                        validate_function_api(function, function.visibility, index, errors)
                    }
                    TypeMember::Constant(binding) => {
                        if let Some(type_) = &binding.type_annotation {
                            validate_exposed_type(type_, binding.visibility, index, errors);
                        }
                    }
                }
            }
        }
        Declaration::Constant(value) => {
            if let Some(type_) = &value.type_annotation {
                validate_exposed_type(type_, value.visibility, index, errors);
            }
        }
        Declaration::Component(value) => {
            for parameter in &value.params {
                validate_exposed_type(&parameter.type_, value.visibility, index, errors);
            }
        }
        Declaration::Extern(value) => {
            for item in &value.items {
                match item {
                    ExternItem::Struct(value) => {
                        if let Some(fields) = &value.fields {
                            for field in fields {
                                validate_exposed_type(
                                    &field.type_,
                                    field.visibility,
                                    index,
                                    errors,
                                );
                            }
                        }
                    }
                    ExternItem::Function(value) => {
                        validate_function_api(value, value.visibility, index, errors)
                    }
                }
            }
        }
        Declaration::Enum(_) | Declaration::Let(_) | Declaration::Use(_) => {}
    }
}

fn validate_function_api(
    function: &kome_ast::declarations::FunctionDeclaration,
    visibility: Visibility,
    index: &ModuleIndex,
    errors: &mut Vec<ModuleError>,
) {
    if visibility == Visibility::Private {
        return;
    }
    for parameter in &function.params {
        if let Pattern::Ident(parameter) = parameter
            && let Some(type_) = &parameter.type_annotation
        {
            validate_exposed_type(type_, visibility, index, errors);
        }
    }
    if let Some(type_) = &function.return_type {
        validate_exposed_type(type_, visibility, index, errors);
    }
}

fn validate_exposed_type(
    type_: &Type,
    visibility: Visibility,
    index: &ModuleIndex,
    errors: &mut Vec<ModuleError>,
) {
    if visibility == Visibility::Private {
        return;
    }
    match type_ {
        Type::Named(value) => {
            if let Some(export) = index.declarations.get(&value.name)
                && visibility_rank(export.visibility) < visibility_rank(visibility)
            {
                errors.push(ModuleError {
                    message: format!(
                        "{} API exposes less-visible type `{}`",
                        visibility_name(visibility),
                        value.name
                    ),
                    span: value.span,
                });
            }
            for argument in &value.type_arguments {
                validate_exposed_type(argument, visibility, index, errors);
            }
        }
        Type::Function(value) => {
            for parameter in &value.params {
                validate_exposed_type(&parameter.type_, visibility, index, errors);
            }
            validate_exposed_type(&value.return_type, visibility, index, errors);
        }
        Type::List(value) => validate_exposed_type(&value.element, visibility, index, errors),
        Type::Object(value) => {
            for member in &value.members {
                validate_exposed_type(&member.type_, visibility, index, errors);
            }
        }
        Type::Optional(value) => validate_exposed_type(&value.inner, visibility, index, errors),
        Type::Pointer(value) => validate_exposed_type(&value.pointee, visibility, index, errors),
        Type::Primitive(_) => {}
    }
}

fn visibility_rank(visibility: Visibility) -> u8 {
    match visibility {
        Visibility::Private => 0,
        Visibility::Package => 1,
        Visibility::Public => 2,
    }
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private",
        Visibility::Package => "package-visible",
        Visibility::Public => "public",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferenceKind {
    TopLevel,
    Local,
}

fn symbol_kind(symbol: &Symbol) -> ReferenceKind {
    match symbol {
        Symbol::Component { .. }
        | Symbol::Function { .. }
        | Symbol::EnumType { .. }
        | Symbol::StructType { .. }
        | Symbol::TraitType { .. }
        | Symbol::EnumCase { .. } => ReferenceKind::TopLevel,
        _ => ReferenceKind::Local,
    }
}

fn build_index(sources: &[SourceModule]) -> (ModuleIndex, Vec<ModuleError>) {
    let mut index = ModuleIndex::default();
    let mut errors = Vec::new();
    for source in sources {
        let module = source.canonical_path();
        index.modules.insert(module.clone());
        for (name, visibility) in declaration_names(&source.module) {
            let canonical = format!("{module}::{name}");
            let export = Export {
                module: module.clone(),
                package: source.package.clone(),
                visibility,
                target: canonical.clone(),
            };
            index.declarations.insert(canonical, export.clone());
            index
                .module_declarations
                .entry(module.clone())
                .or_default()
                .push((name, export));
        }
    }

    for _ in 0..sources.len().max(1) {
        let mut changed = false;
        for source in sources {
            changed |= collect_reexports(source, &mut index, &mut errors);
        }
        if !changed {
            break;
        }
    }

    (index, errors)
}

fn collect_reexports(
    source: &SourceModule,
    index: &mut ModuleIndex,
    errors: &mut Vec<ModuleError>,
) -> bool {
    let mut changed = false;
    for declaration in &source.module.declarations {
        let Declaration::Use(declaration) = declaration else {
            continue;
        };
        if declaration.visibility == Visibility::Private {
            continue;
        }
        for import in &declaration.imports {
            match import {
                UseImport::Module(path) | UseImport::AliasedModule { path, .. } => {
                    let canonical = normalize_path(source, path);
                    let Some(target) = index.declarations.get(&canonical).cloned() else {
                        continue;
                    };
                    if !accessible(source, &target) {
                        continue;
                    }
                    if visibility_rank(target.visibility) < visibility_rank(declaration.visibility)
                    {
                        push_unique_error(
                            errors,
                            ModuleError {
                                message: format!(
                                    "cannot re-export `{canonical}` with a wider visibility"
                                ),
                                span: path.span,
                            },
                        );
                        continue;
                    }
                    let local = match import {
                        UseImport::AliasedModule { alias, .. } => match &alias.kind {
                            PathSegmentKind::Ident(name) => name.clone(),
                            _ => continue,
                        },
                        _ => canonical
                            .rsplit("::")
                            .next()
                            .unwrap_or(&canonical)
                            .to_owned(),
                    };
                    changed |= insert_reexport(
                        source,
                        &local,
                        target.target,
                        declaration.visibility,
                        path.span,
                        index,
                        errors,
                    );
                }
                UseImport::WildcardFrom { path, span } => {
                    let canonical = normalize_path(source, path);
                    let Some(exports) = index.module_declarations.get(&canonical).cloned() else {
                        continue;
                    };
                    for (name, target) in exports {
                        if !accessible(source, &target) {
                            continue;
                        }
                        if visibility_rank(target.visibility)
                            < visibility_rank(declaration.visibility)
                        {
                            continue;
                        }
                        changed |= insert_reexport(
                            source,
                            &name,
                            target.target,
                            declaration.visibility,
                            *span,
                            index,
                            errors,
                        );
                    }
                }
                UseImport::Wildcard { .. } => {}
            }
        }
    }
    changed
}

fn insert_reexport(
    source: &SourceModule,
    local: &str,
    target: String,
    visibility: Visibility,
    span: Span,
    index: &mut ModuleIndex,
    errors: &mut Vec<ModuleError>,
) -> bool {
    let module = source.canonical_path();
    let canonical = format!("{module}::{local}");
    if let Some(previous) = index.declarations.get(&canonical) {
        if previous.target != target {
            push_unique_error(
                errors,
                ModuleError {
                    message: format!("re-export `{canonical}` conflicts with another declaration"),
                    span,
                },
            );
        }
        return false;
    }
    let export = Export {
        module: module.clone(),
        package: source.package.clone(),
        visibility,
        target,
    };
    index.declarations.insert(canonical, export.clone());
    index
        .module_declarations
        .entry(module)
        .or_default()
        .push((local.to_owned(), export));
    true
}

fn push_unique_error(errors: &mut Vec<ModuleError>, error: ModuleError) {
    if !errors.contains(&error) {
        errors.push(error);
    }
}

fn declaration_names(module: &Module) -> Vec<(String, Visibility)> {
    let mut names = Vec::new();
    for declaration in &module.declarations {
        match declaration {
            Declaration::Component(value) => names.push((value.name.clone(), value.visibility)),
            Declaration::Function(value) => names.push((value.name.clone(), value.visibility)),
            Declaration::Struct(value) => names.push((value.name.clone(), value.visibility)),
            Declaration::Trait(value) => names.push((value.name.clone(), value.visibility)),
            Declaration::Constant(value) => {
                if let Some(name) = pattern_name(&value.pattern) {
                    names.push((name.to_owned(), value.visibility));
                }
            }
            Declaration::Enum(value) => names.push((value.name.clone(), value.visibility)),
            Declaration::Extern(value) => {
                for item in &value.items {
                    match item {
                        ExternItem::Struct(value) => {
                            names.push((value.name.clone(), value.visibility));
                        }
                        ExternItem::Function(value) => {
                            names.push((value.name.clone(), value.visibility));
                        }
                    }
                }
            }
            Declaration::For(_) | Declaration::Let(_) | Declaration::Use(_) => {}
        }
    }
    names
}

fn pattern_name(pattern: &Pattern) -> Option<&str> {
    match pattern {
        Pattern::Ident(value) => Some(&value.name),
        Pattern::Literal(_) => None,
    }
}

fn collect_imports(
    source: &SourceModule,
    index: &ModuleIndex,
    errors: &mut Vec<ModuleError>,
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut modules = HashMap::new();
    let mut items = HashMap::new();
    for declaration in &source.module.declarations {
        let Declaration::Use(declaration) = declaration else {
            continue;
        };
        for import in &declaration.imports {
            match import {
                UseImport::Module(path) => {
                    import_path(source, index, path, None, &mut modules, &mut items, errors);
                }
                UseImport::AliasedModule { path, alias } => {
                    let PathSegmentKind::Ident(alias) = &alias.kind else {
                        continue;
                    };
                    import_path(
                        source,
                        index,
                        path,
                        Some(alias),
                        &mut modules,
                        &mut items,
                        errors,
                    );
                }
                UseImport::WildcardFrom { path, span } => {
                    let canonical = normalize_path(source, path);
                    let Some(declarations) = index.module_declarations.get(&canonical) else {
                        errors.push(ModuleError {
                            message: format!("unknown module `{canonical}`"),
                            span: *span,
                        });
                        continue;
                    };
                    for (name, export) in declarations {
                        if accessible(source, export) {
                            insert_import(&mut items, name, export.target.clone(), *span, errors);
                        }
                    }
                }
                UseImport::Wildcard { span } => errors.push(ModuleError {
                    message: "`use *` requires an explicit module path".into(),
                    span: *span,
                }),
            }
        }
    }
    (modules, items)
}

fn import_path(
    source: &SourceModule,
    index: &ModuleIndex,
    path: &Path,
    alias: Option<&String>,
    modules: &mut HashMap<String, String>,
    items: &mut HashMap<String, String>,
    errors: &mut Vec<ModuleError>,
) {
    let canonical = normalize_path(source, path);
    if index.modules.contains(&canonical) {
        let local = alias.cloned().unwrap_or_else(|| {
            canonical
                .rsplit("::")
                .next()
                .unwrap_or(&canonical)
                .to_owned()
        });
        insert_import(modules, &local, canonical, path.span, errors);
        return;
    }
    let Some(export) = index.declarations.get(&canonical) else {
        errors.push(ModuleError {
            message: format!("unknown module or declaration `{canonical}`"),
            span: path.span,
        });
        return;
    };
    if !accessible(source, export) {
        errors.push(ModuleError {
            message: format!("declaration `{canonical}` is not visible here"),
            span: path.span,
        });
        return;
    }
    let local = alias.cloned().unwrap_or_else(|| {
        canonical
            .rsplit("::")
            .next()
            .unwrap_or(&canonical)
            .to_owned()
    });
    insert_import(items, &local, export.target.clone(), path.span, errors);
}

fn insert_import(
    imports: &mut HashMap<String, String>,
    local: &str,
    canonical: String,
    span: Span,
    errors: &mut Vec<ModuleError>,
) {
    if let Some(previous) = imports.insert(local.to_owned(), canonical.clone())
        && previous != canonical
    {
        errors.push(ModuleError {
            message: format!("import name `{local}` is ambiguous"),
            span,
        });
    }
}

fn normalize_path(source: &SourceModule, path: &Path) -> String {
    let mut segments = Vec::new();
    for segment in &path.segments {
        match &segment.kind {
            PathSegmentKind::Ident(name) => segments.push(name.clone()),
            PathSegmentKind::Self_ => {
                segments.push(source.package.clone());
                segments.extend(source.path.clone());
            }
            PathSegmentKind::Super => {
                if segments.is_empty() {
                    segments.push(source.package.clone());
                    segments.extend(
                        source
                            .path
                            .iter()
                            .take(source.path.len().saturating_sub(1))
                            .cloned(),
                    );
                } else {
                    segments.pop();
                }
            }
        }
    }
    segments.join("::")
}

fn accessible(source: &SourceModule, export: &Export) -> bool {
    export.module == source.canonical_path()
        || export.visibility == Visibility::Public
        || (export.visibility == Visibility::Package && export.package == source.package)
}

struct Rewriter<'a> {
    source: &'a mut SourceModule,
    index: &'a ModuleIndex,
    local_names: HashMap<String, String>,
    reference_kinds: HashMap<(usize, usize), ReferenceKind>,
    module_aliases: HashMap<String, String>,
    item_aliases: HashMap<String, String>,
    errors: &'a mut Vec<ModuleError>,
}

impl Rewriter<'_> {
    fn rewrite_declaration(&mut self, declaration: &mut Declaration, top_level: bool) {
        match declaration {
            Declaration::Component(value) => {
                self.rewrite_parameters(&mut value.params);
                if let Some(body) = &mut value.body {
                    for member in body {
                        match member {
                            ComponentMember::State(value) | ComponentMember::Let(value) => {
                                self.rewrite_binding(value)
                            }
                            ComponentMember::Recipe(value) => self.rewrite_block(&mut value.body),
                            ComponentMember::Function(value) => self.rewrite_function(value, false),
                        }
                    }
                }
                if top_level {
                    self.rename_declaration(&mut value.name);
                }
            }
            Declaration::Function(value) => self.rewrite_function(value, top_level),
            Declaration::Struct(value) => {
                if let Some(fields) = &mut value.fields {
                    for field in fields {
                        self.rewrite_type(&mut field.type_);
                    }
                }
                if top_level {
                    self.rename_declaration(&mut value.name);
                }
            }
            Declaration::Trait(value) => {
                for function in &mut value.functions {
                    self.rewrite_function(function, false);
                }
                if top_level {
                    self.rename_declaration(&mut value.name);
                }
            }
            Declaration::For(value) => {
                self.rewrite_type(&mut value.target);
                if let Some(trait_) = &mut value.trait_ {
                    self.rewrite_type(trait_);
                }
                for member in &mut value.members {
                    match member {
                        TypeMember::Constant(value) => self.rewrite_binding(value),
                        TypeMember::Function(value) => self.rewrite_function(value, false),
                    }
                }
            }
            Declaration::Let(value) => self.rewrite_binding(value),
            Declaration::Constant(value) => {
                self.rewrite_binding(value);
                if top_level {
                    self.rename_binding(value);
                }
            }
            Declaration::Use(_) => {}
            Declaration::Enum(value) => {
                for case in &mut value.cases {
                    if let Some(expression) = &mut case.value {
                        self.rewrite_expression(expression);
                    }
                }
                if top_level {
                    self.rename_declaration(&mut value.name);
                }
            }
            Declaration::Extern(value) => {
                for item in &mut value.items {
                    match item {
                        ExternItem::Struct(value) => {
                            if let Some(fields) = &mut value.fields {
                                for field in fields {
                                    self.rewrite_type(&mut field.type_);
                                }
                            }
                            self.rename_declaration(&mut value.name);
                        }
                        ExternItem::Function(value) => self.rewrite_function(value, true),
                    }
                }
            }
        }
    }

    fn rewrite_function(
        &mut self,
        value: &mut kome_ast::declarations::FunctionDeclaration,
        rename: bool,
    ) {
        for parameter in &mut value.params {
            self.rewrite_pattern(parameter);
        }
        if let Some(type_) = &mut value.return_type {
            self.rewrite_type(type_);
        }
        if let Some(body) = &mut value.body {
            self.rewrite_block(body);
        }
        if rename {
            self.rename_declaration(&mut value.name);
        }
    }

    fn rewrite_binding(&mut self, value: &mut Binding) {
        self.rewrite_pattern(&mut value.pattern);
        if let Some(type_) = &mut value.type_annotation {
            self.rewrite_type(type_);
        }
        if let Some(expression) = &mut value.init {
            self.rewrite_expression(expression);
        }
    }

    fn rewrite_parameters(&mut self, parameters: &mut [kome_ast::types::Parameter]) {
        for parameter in parameters {
            self.rewrite_type(&mut parameter.type_);
            if let Some(default) = &mut parameter.default {
                self.rewrite_expression(default);
            }
        }
    }

    fn rewrite_pattern(&mut self, pattern: &mut Pattern) {
        if let Pattern::Ident(value) = pattern {
            if let Some(type_) = &mut value.type_annotation {
                self.rewrite_type(type_);
            }
            if let Some(default) = &mut value.default {
                self.rewrite_expression(default);
            }
        }
    }

    fn rewrite_block(&mut self, block: &mut kome_ast::statements::BlockStatement) {
        for statement in &mut block.statements {
            self.rewrite_statement(statement);
        }
    }

    fn rewrite_statement(&mut self, statement: &mut Statement) {
        match statement {
            Statement::Block(value) => self.rewrite_block(value),
            Statement::Expression(value) => self.rewrite_expression(&mut value.expression),
            Statement::Let(value) => self.rewrite_binding(value),
            Statement::If(value) => {
                self.rewrite_expression(&mut value.test);
                self.rewrite_statement(&mut value.consequent);
                if let Some(alternative) = &mut value.alternative {
                    self.rewrite_statement(alternative);
                }
            }
            Statement::While(value) => {
                self.rewrite_expression(&mut value.test);
                self.rewrite_statement(&mut value.body);
            }
            Statement::ForIn(value) => {
                self.rewrite_pattern(&mut value.pattern);
                self.rewrite_expression(&mut value.right);
                self.rewrite_statement(&mut value.body);
            }
            Statement::Return(value) => {
                if let Some(argument) = &mut value.argument {
                    self.rewrite_expression(argument);
                }
            }
            Statement::Is(value) => {
                if let Some(expression) = &mut value.value {
                    self.rewrite_expression(expression);
                }
                self.rewrite_is_pattern(&mut value.pattern);
                self.rewrite_statement(&mut value.body);
            }
            Statement::Declaration(value) => self.rewrite_declaration(value, false),
            Statement::Break(_) | Statement::Continue(_) | Statement::Empty(_) => {}
        }
    }

    fn rewrite_expression(&mut self, expression: &mut Expression) {
        match expression {
            Expression::Ident(value) => {
                value.name = self.resolve_name(&value.name, value.span);
            }
            Expression::Unary(value) => self.rewrite_expression(&mut value.argument),
            Expression::Unwrap(value) => self.rewrite_expression(&mut value.argument),
            Expression::Task(value) => self.rewrite_expression(&mut value.argument),
            Expression::Wait(value) => self.rewrite_expression(&mut value.argument),
            Expression::Cancel(value) => self.rewrite_expression(&mut value.argument),
            Expression::Binary(value) => {
                self.rewrite_expression(&mut value.left);
                self.rewrite_expression(&mut value.right);
            }
            Expression::Call(value) => {
                self.rewrite_expression(&mut value.callee);
                for type_ in &mut value.type_arguments {
                    self.rewrite_type(type_);
                }
                self.rewrite_call_args(&mut value.args);
            }
            Expression::Member(value) => self.rewrite_expression(&mut value.object),
            Expression::Index(value) => {
                self.rewrite_expression(&mut value.object);
                self.rewrite_expression(&mut value.index);
            }
            Expression::Assign(value) => {
                self.rewrite_expression(&mut value.target);
                self.rewrite_expression(&mut value.value);
            }
            Expression::Group(value) => self.rewrite_expression(&mut value.expression),
            Expression::Block(value) => {
                for statement in &mut value.statements {
                    self.rewrite_statement(statement);
                }
                if let Some(tail) = &mut value.tail {
                    self.rewrite_expression(tail);
                }
            }
            Expression::List(value) => {
                for expression in value.elems.iter_mut().flatten() {
                    self.rewrite_expression(expression);
                }
            }
            Expression::Object(value) => {
                for property in &mut value.props {
                    let ObjectProperty::KeyValue(property) = property;
                    if let PropertyKey::Computed { expression, .. } = &mut property.key {
                        self.rewrite_expression(expression);
                    }
                    self.rewrite_expression(&mut property.value);
                }
            }
            Expression::Struct(value) => {
                value.name = self.resolve_name(&value.name, value.span);
                for type_ in &mut value.type_arguments {
                    self.rewrite_type(type_);
                }
                for field in &mut value.fields {
                    self.rewrite_expression(&mut field.value);
                }
            }
            Expression::Template(value) => {
                for part in &mut value.parts {
                    if let TemplatePart::Expression { expression, .. } = part {
                        self.rewrite_expression(expression);
                    }
                }
            }
            Expression::Closure(value) => {
                for parameter in &mut value.params {
                    self.rewrite_pattern(parameter);
                }
                self.rewrite_expression(&mut value.body);
            }
            Expression::Is(value) => {
                self.rewrite_expression(&mut value.value);
                self.rewrite_is_pattern(&mut value.pattern);
                self.rewrite_expression(&mut value.body);
            }
            Expression::Component(value) => {
                value.name = self.resolve_name(&value.name, value.span);
                self.rewrite_call_args(&mut value.args);
                for child in &mut value.children {
                    self.rewrite_expression(child);
                }
            }
            Expression::Literal(_) | Expression::DotIdent(_) => {}
        }
    }

    fn rewrite_call_args(&mut self, arguments: &mut [CallArg]) {
        for argument in arguments {
            match argument {
                CallArg::Positional(value) => self.rewrite_expression(value),
                CallArg::Named { value, .. } => self.rewrite_expression(value),
            }
        }
    }

    fn rewrite_is_pattern(&mut self, pattern: &mut IsPattern) {
        if let IsPattern::Ident(value) = pattern {
            if let Some(type_) = &mut value.type_annotation {
                self.rewrite_type(type_);
            }
            if let Some(default) = &mut value.default {
                self.rewrite_expression(default);
            }
        }
    }

    fn rewrite_type(&mut self, type_: &mut Type) {
        match type_ {
            Type::Function(value) => {
                self.rewrite_parameters(&mut value.params);
                self.rewrite_type(&mut value.return_type);
            }
            Type::List(value) => self.rewrite_type(&mut value.element),
            Type::Object(value) => {
                for member in &mut value.members {
                    self.rewrite_type(&mut member.type_);
                }
            }
            Type::Named(value) => {
                value.name = self.resolve_name(&value.name, value.span);
                for argument in &mut value.type_arguments {
                    self.rewrite_type(argument);
                }
            }
            Type::Optional(value) => self.rewrite_type(&mut value.inner),
            Type::Pointer(value) => self.rewrite_type(&mut value.pointee),
            Type::Primitive(_) => {}
        }
    }

    fn resolve_name(&mut self, name: &str, span: Span) -> String {
        let reference_kind = self.reference_kinds.get(&(span.start, span.end));
        if reference_kind == Some(&ReferenceKind::Local) {
            return name.to_owned();
        }
        if !name.contains("::") {
            if let Some(canonical) = self.local_names.get(name) {
                return canonical.clone();
            }
            if let Some(canonical) = self.item_aliases.get(name) {
                return canonical.clone();
            }
            return name.to_owned();
        }
        let mut segments = name.split("::");
        let first = segments.next().unwrap_or(name);
        let remainder = segments.collect::<Vec<_>>().join("::");
        let canonical = if let Some(module) = self.module_aliases.get(first) {
            if remainder.is_empty() {
                module.clone()
            } else {
                format!("{module}::{remainder}")
            }
        } else {
            name.to_owned()
        };
        if let Some(export) = self.index.declarations.get(&canonical)
            && !accessible(self.source, export)
        {
            self.errors.push(ModuleError {
                message: format!("declaration `{canonical}` is not visible here"),
                span,
            });
        }
        self.index
            .declarations
            .get(&canonical)
            .map_or(canonical, |export| export.target.clone())
    }

    fn rename_declaration(&self, name: &mut String) {
        if let Some(canonical) = self.local_names.get(name) {
            *name = canonical.clone();
        }
    }

    fn rename_binding(&self, binding: &mut Binding) {
        if let Pattern::Ident(pattern) = &mut binding.pattern
            && let Some(canonical) = self.local_names.get(&pattern.name)
        {
            pattern.name = canonical.clone();
        }
    }
}
