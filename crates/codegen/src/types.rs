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
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Null,
    Void,
}

impl KomeType {
    /// Maps a type annotation to a compilable type.
    pub fn from_annotation(annotation: &Type) -> CodegenResult<Self> {
        let Type::Primitive(primitive) = annotation else {
            return Err(CodegenError::at(
                "only Number, bool, Null are supported now",
                annotation.span(),
            ));
        };

        Self::from_primitive_kind(&primitive.kind)
    }

    fn from_primitive_kind(kind: &PrimitiveTypeKind) -> CodegenResult<Self> {
        match kind {
            PrimitiveTypeKind::Number => Ok(Self::Number),
            PrimitiveTypeKind::Bool => Ok(Self::Boolean),
            PrimitiveTypeKind::I8 => Ok(Self::I8),
            PrimitiveTypeKind::I16 => Ok(Self::I16),
            PrimitiveTypeKind::I32 => Ok(Self::I32),
            PrimitiveTypeKind::I64 => Ok(Self::I64),
            PrimitiveTypeKind::U8 => Ok(Self::U8),
            PrimitiveTypeKind::U16 => Ok(Self::U16),
            PrimitiveTypeKind::U32 => Ok(Self::U32),
            PrimitiveTypeKind::U64 => Ok(Self::U64),
            PrimitiveTypeKind::F32 => Ok(Self::F32),
            PrimitiveTypeKind::F64 => Ok(Self::F64),
            PrimitiveTypeKind::String => unreachable!("String is rejected before code generation"),
            PrimitiveTypeKind::Null => Ok(Self::Null),
        }
    }

    /// The Cranelift representation; `None` for `Void`.
    pub fn cranelift(self) -> Option<cranelift::prelude::Type> {
        match self {
            Self::Number | Self::F64 => Some(types::F64),
            Self::F32 => Some(types::F32),
            Self::Boolean | Self::I8 | Self::U8 | Self::Null => Some(types::I8),
            Self::I16 | Self::U16 => Some(types::I16),
            Self::I32 | Self::U32 => Some(types::I32),
            Self::I64 | Self::U64 => Some(types::I64),
            Self::Void => None,
        }
    }

    /// The ABI tag used when marshalling values of this type.
    pub fn tag(self) -> CodegenResult<i64> {
        match self {
            Self::Number => Ok(abi::TAG_NUMBER),
            Self::Boolean => Ok(abi::TAG_BOOLEAN),
            Self::Null => Ok(abi::TAG_NULL),
            Self::Void => Ok(abi::TAG_VOID),
            Self::I8
            | Self::I16
            | Self::I32
            | Self::I64
            | Self::U8
            | Self::U16
            | Self::U32
            | Self::U64
            | Self::F32
            | Self::F64 => Err(CodegenError::new(
                "fixed-width numeric types are not supported by the native ABI yet",
                None,
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Number => "Number",
            Self::Boolean => "bool",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Null => "Null",
            Self::Void => "Void",
        }
    }
}
