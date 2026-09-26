use crate::object::*;
use num_traits::{Signed, ToPrimitive};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicI64;

pub mod builtins;
mod codecs;
mod extra;
mod importlib;
mod math;
mod operator;
pub mod os;
pub mod posixsubprocess;
mod sys;

pub use builtins::*;
pub use codecs::*;
pub use extra::*;
pub use importlib::*;
pub use math::*;
pub use operator::*;
pub use os::*;

/// `subprocess`'s `pass_fds` support: clear `FD_CLOEXEC` in the child.
pub use os::fd_ops::clear_cloexec as os_fd_clear_cloexec;
pub use posixsubprocess::*;
pub use sys::*;
