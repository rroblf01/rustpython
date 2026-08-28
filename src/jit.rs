#![cfg(feature = "jit")]

pub mod globals;
pub mod runtime;
pub mod runtime_extra;
pub mod compiler;
pub mod compile;
pub mod emit;
pub mod emit2;

pub use globals::{JitGlobalsGuard, set_jit_globals};
pub use compiler::JitCompiler;
