//! Kome's statically typed value kinds and their native representations.

use crate::error::{CodegenError, CodegenResult};
use cranelift::prelude::types;
use kome_abi as abi;
use kome_ast::AstNode;
use kome_ast::types::{PrimitiveTypeKind, Type};

/// The subset of Kome types that compiles to native code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KomeType {
    Number,
    Boolean,
    Null,
    Void,
}

impl KomeType {
    /// Maps a type annotation to a compilable type.
    pub fn from_annotation(annotation: &Type) -> CodegenResult<Self> {
        let Type::Primitive(primitive) = annotation else {
            return Err(CodegenError::at(
                "only Number, Boolean, Null are supported now",
                annotation.span(),
            ));
        };

        Self::from_primitive_kind(&primitive.kind)
    }

    fn from_primitive_kind(kind: &PrimitiveTypeKind) -> CodegenResult<Self> {
        match kind {
            PrimitiveTypeKind::Number => Ok(Self::Number),
            PrimitiveTypeKind::Boolean => Ok(Self::Boolean),
            PrimitiveTypeKind::String => unreachable!("String is rejected before code generation"),
            PrimitiveTypeKind::Null => Ok(Self::Null),
        }
    }

    /// The Cranelift representation; `None` for `Void`.
    pub fn cranelift(self) -> Option<cranelift::prelude::Type> {
        match self {
            Self::Number => Some(types::F64),
            Self::Boolean | Self::Null => Some(types::I8),
            Self::Void => None,
        }
    }

    /// The ABI tag used when marshalling values of this type.
    pub fn tag(self) -> i64 {
        match self {
            Self::Number => abi::TAG_NUMBER,
            Self::Boolean => abi::TAG_BOOLEAN,
            Self::Null => abi::TAG_NULL,
            Self::Void => abi::TAG_VOID,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Number => "Number",
            Self::Boolean => "Boolean",
            Self::Null => "Null",
            Self::Void => "Void",
        }
    }
}
