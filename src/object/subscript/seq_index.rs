// Split from src/object/subscript.rs — sequence index helpers (smallest group).
use super::*;

/// Plain-value equivalent of `to_index` for the many sequence-indexing sites
/// below (`list`/`tuple`/`str`/`bytes`/`bytearray`/`array`/`range`) that
/// already have `index` borrowed as a `PyObject` and just need "is this an
/// int (or bool, a genuine int subtype) at all" without the `__index__`-via-
/// mro dispatch `to_index` also does (those sites fall back to a `Slice`
/// arm too, which `to_index` doesn't know about). Found via `list[True]`
/// (and the tuple/str/bytes/bytearray/array/range equivalents) all raising
/// `TypeError: ... indices must be integers or slices` despite `bool` being
/// a valid index in real Python — same root gap as `range()`'s own
/// `__index__`/bool fix just above, just for indexing instead of construction.
pub(crate) fn sequence_index_int(idx: &PyObject) -> Option<BigInt> {
    match idx {
        PyObject::Int(i) => Some(i.clone()),
        PyObject::Bool(b) => Some(BigInt::from(*b as i64)),
        _ => None,
    }
}
