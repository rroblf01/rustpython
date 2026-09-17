#![cfg(feature = "jit")]

pub mod compile;
pub mod compiler;
pub mod emit;
pub mod emit2;
pub mod globals;
pub mod runtime;
pub mod runtime_extra;

pub use compiler::JitCompiler;
pub use globals::{set_jit_globals, JitGlobalsGuard};
