//! Native code generation for the Kome language, built on Cranelift.
//!
//! The pipeline is statically typed: type annotations on parameters,
//! returns, and `let` bindings drive the choice of native representations
//! (`Number` → `F64`, `Boolean`/`Null` → `I8`, `String` → boxed pointer).
//!
//! Runtime backends such as `kome_jit` and `kome_aot` use this crate's common
//! AST analysis and Cranelift IR-generation pass.

pub mod compile;

mod error;
mod types;

pub use error::{CodegenError, CodegenResult};
pub use types::KomeType;
