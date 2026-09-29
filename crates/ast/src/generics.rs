//! Reusable generic type substitution over AST type syntax.

use crate::types::{FunctionType, ListType, NamedType, ObjectType, OptionalType, Parameter, Type};
use std::collections::HashMap;

/// A mapping from declaration type-parameter names to concrete type arguments.
#[derive(Debug, Clone, Default)]
pub struct TypeSubstitution {
    arguments: HashMap<String, Type>,
}

impl TypeSubstitution {
    /// Creates a substitution by pairing parameters and arguments in order.
    pub fn new<'a>(parameters: impl IntoIterator<Item = &'a str>, arguments: &[Type]) -> Self {
        Self {
            arguments: parameters
                .into_iter()
                .zip(arguments.iter().cloned())
                .map(|(name, type_)| (name.to_owned(), type_))
                .collect(),
        }
    }

    /// Returns the concrete argument bound to `name`, if one exists.
    pub fn get(&self, name: &str) -> Option<&Type> {
        self.arguments.get(name)
    }

    /// Recursively substitutes type parameters in a type expression.
    pub fn apply(&self, type_: &Type) -> Type {
        match type_ {
            Type::Named(named) if named.type_arguments.is_empty() => self
                .get(&named.name)
                .cloned()
                .unwrap_or_else(|| type_.clone()),
            Type::Named(named) => Type::Named(NamedType {
                span: named.span,
                name: named.name.clone(),
                type_arguments: named
                    .type_arguments
                    .iter()
                    .map(|argument| self.apply(argument))
                    .collect(),
            }),
            Type::Optional(optional) => Type::Optional(OptionalType {
                span: optional.span,
                inner: Box::new(self.apply(&optional.inner)),
            }),
            Type::List(list) => Type::List(ListType {
                span: list.span,
                element: Box::new(self.apply(&list.element)),
            }),
            Type::Function(function) => Type::Function(FunctionType {
                span: function.span,
                params: function
                    .params
                    .iter()
                    .map(|parameter| Parameter {
                        span: parameter.span,
                        name: parameter.name.clone(),
                        type_: self.apply(&parameter.type_),
                        default: parameter.default.clone(),
                    })
                    .collect(),
                return_type: Box::new(self.apply(&function.return_type)),
            }),
            Type::Object(object) => Type::Object(ObjectType {
                span: object.span,
                members: object
                    .members
                    .iter()
                    .map(|member| crate::types::ObjectTypeMember {
                        span: member.span,
                        key: member.key.clone(),
                        type_: self.apply(&member.type_),
                        optional: member.optional,
                    })
                    .collect(),
            }),
            Type::Primitive(_) => type_.clone(),
        }
    }
}

//    ^==^
//  = . _ . =
//    U---U
