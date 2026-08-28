// Split from src/object/core.rs — foundational object representation.
// This module re-exports the split submodules to preserve `crate::object::*` paths.
use super::*;

pub mod hasher;
pub use hasher::{FxBuildHasher, FxHasher};

pub mod dict;
pub use dict::{DictMap, TypeDict};
pub(crate) use dict::{str_map_to_strid_map, str_map_to_typedict};

pub mod attr_map;
pub use attr_map::{AttrEntry, AttrMap};

pub mod small;
pub use small::{RefOrOwned, SmallStr, ALLOC_COUNT, IMM_COUNT, BuiltinFunc};

pub(crate) mod object_id;

pub mod pyref;
pub use pyref::PyObjectRef;

pub mod pyref_eq;
pub(crate) use pyref_eq::NativeDispatchRecursionGuard;
