// Split from src/object/subscript.rs — `__index__` fallback helper (smallest group).
use super::*;

/// Like `sequence_index_int` but via the full `__index__` protocol (a custom
/// object with an `__index__` method is a valid index — test_index.py).
pub(crate) fn try_to_index(index: &PyObjectRef) -> Option<BigInt> {
    crate::object::to_index(index).ok()
}
