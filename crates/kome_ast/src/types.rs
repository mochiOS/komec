//! Types: primitives, functions, Lists, objects, named, unions, etc.

use crate::{AstNode, Span};

/// A type annotation.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Primitive(PrimitiveType),
    Function(FunctionType),
    List(ListType),
    Object(ObjectType),
    Named(NamedType),
    Optional(OptionalType),
}

/// `String`, `Number`, `Boolean`, etc.
#[derive(Debug, Clone, PartialEq)]
pub struct PrimitiveType {
    pub span: Span,
    pub kind: PrimitiveTypeKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrimitiveTypeKind {
    String,
    Number,
    Boolean,
    Null,
}

/// `(parameter) => ReturnType`
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionType {
    pub span: Span,
    pub params: Vec<Parameter>,
    pub return_type: Box<Type>,
}

/// A single parameter in a function type or component declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub span: Span,
    pub name: String,
    pub type_: Type,
    pub default: Option<crate::expressions::Expression>,
}

/// `ElementType[]`
#[derive(Debug, Clone, PartialEq)]
pub struct ListType {
    pub span: Span,
    pub element: Box<Type>,
}

/// `{ key: Type }`
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectType {
    pub span: Span,
    pub members: Vec<ObjectTypeMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectTypeMember {
    pub span: Span,
    pub key: String,
    pub type_: Type,
    pub optional: bool,
}

/// `Name<TypeArgument>`
#[derive(Debug, Clone, PartialEq)]
pub struct NamedType {
    pub span: Span,
    pub name: String,
    pub type_arguments: Vec<Type>,
}

/// `Type?`
#[derive(Debug, Clone, PartialEq)]
pub struct OptionalType {
    pub span: Span,
    pub inner: Box<Type>,
}

impl AstNode for Type {
    fn span(&self) -> Span {
        match self {
            Type::Primitive(type_) => type_.span,
            Type::Function(type_) => type_.span,
            Type::List(type_) => type_.span,
            Type::Object(type_) => type_.span,
            Type::Named(type_) => type_.span,
            Type::Optional(type_) => type_.span,
        }
    }
}
