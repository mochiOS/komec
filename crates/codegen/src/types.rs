//! Kome's statically typed value kinds and their native representations.

use crate::error::{CodegenError, CodegenResult};
use cranelift::prelude::types;
use kome_abi as abi;
use kome_ast::AstNode;
use kome_ast::types::{PrimitiveTypeKind, Type};

/// The subset of Kome types that compiles to native code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KomeType {
    Number,
    String,
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
    /// A user-defined struct. The id indexes `ModuleInfo::structs`.
    Struct(usize),
    /// A runtime task. The id indexes `ModuleInfo::task_types`.
    Task(usize),
    /// A homogeneous runtime list. The id indexes `ModuleInfo::list_types`.
    List(usize),
    Void,
}

impl KomeType {
    /// Maps a type annotation to a compilable type.
    pub fn from_annotation(annotation: &Type) -> CodegenResult<Self> {
        match annotation {
            Type::Primitive(primitive) => Self::from_primitive_kind(&primitive.kind),
            _ => Err(CodegenError::at(
                "only primitive or runtime-backed types are supported now",
                annotation.span(),
            )),
        }
    }

    /// Maps a runtime representation name to its native codegen type.
    pub fn from_runtime_name(name: &str, span: kome_ast::Span) -> CodegenResult<Self> {
        match name {
            "string" => Ok(Self::String),
            "number" => Ok(Self::Number),
            _ => Err(CodegenError::at(
                format!("unsupported runtime type `{name}`"),
                span,
            )),
        }
    }

    fn from_primitive_kind(kind: &PrimitiveTypeKind) -> CodegenResult<Self> {
        match kind {
            PrimitiveTypeKind::Number => Ok(Self::Number),
            PrimitiveTypeKind::String => Ok(Self::String),
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
            PrimitiveTypeKind::Null => Ok(Self::Null),
        }
    }

    /// The Cranelift representation; `None` for `Void`.
    pub fn cranelift(self) -> Option<cranelift::prelude::Type> {
        match self {
            Self::Number | Self::String | Self::Struct(_) | Self::Task(_) | Self::List(_) => {
                Some(types::I64)
            }
            Self::F64 => Some(types::F64),
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
            Self::String => Ok(abi::TAG_STRING),
            Self::Boolean => Ok(abi::TAG_BOOLEAN),
            Self::Null => Ok(abi::TAG_NULL),
            Self::Void => Ok(abi::TAG_VOID),
            Self::Struct(_) => Err(CodegenError::new(
                "user-defined structs cannot cross the native ABI",
                None,
            )),
            Self::Task(_) => Err(CodegenError::new("tasks cannot cross the native ABI", None)),
            Self::List(_) => Err(CodegenError::new("lists cannot cross the native ABI", None)),
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

    /// Returns a diagnostic name for this code-generation type.
    pub fn name(self) -> String {
        match self {
            Self::Number => "Number".into(),
            Self::String => "String".into(),
            Self::Boolean => "bool".into(),
            Self::I8 => "i8".into(),
            Self::I16 => "i16".into(),
            Self::I32 => "i32".into(),
            Self::I64 => "i64".into(),
            Self::U8 => "u8".into(),
            Self::U16 => "u16".into(),
            Self::U32 => "u32".into(),
            Self::U64 => "u64".into(),
            Self::F32 => "f32".into(),
            Self::F64 => "f64".into(),
            Self::Null => "Null".into(),
            Self::Struct(id) => format!("struct#{id}"),
            Self::Task(id) => format!("Task#{id}"),
            Self::List(id) => format!("List#{id}"),
            Self::Void => "Void".into(),
        }
    }

    /// Returns whether values of this type use retain/release ownership.
    pub fn is_managed(self) -> bool {
        matches!(
            self,
            Self::Number | Self::String | Self::Struct(_) | Self::Task(_) | Self::List(_)
        )
    }
}
