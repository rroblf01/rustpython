// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds unary numeric
// operations (`-`, `not`) plus the shared `hash_bigint`/`as_complex_parts`
// helpers used across the numeric-tower operations.
use super::*;

/// The int hash: take lower bits (kept as originally implemented — not
/// CPython's real mod-2**61-1 algorithm, but a stable, self-consistent hash
/// for `PyObject::Int`). Factored out so `PyObject::Float`'s whole-number
/// case (see `PyObject::hash`) can call the SAME function a bigint built
/// from that float, guaranteeing `hash(1) == hash(1.0)` without changing
/// Int's own existing hash values at all.
pub(crate) fn hash_bigint(i: &BigInt) -> usize {
    // Matches real CPython's `hash(n) == n` for any int that fits in a
    // machine word (the overwhelming common case for dict/set keys and for
    // the `hash()` builtin) — including negative values, which the
    // byte-XOR-scan fallback below gets wrong for: it reads
    // `to_signed_bytes_le()`'s two's-complement bytes WITHOUT sign-
    // extending them into the `usize` accumulator, so e.g. `hash(-5)`
    // produced `251` instead of `-5`'s own bit pattern. `-1` is remapped to
    // `-2`, matching CPython's own special case (C-level code reserves -1
    // as an internal "hash computation failed" sentinel, so a real hash
    // that happens to compute to -1 is bumped to -2 instead).
    if let Some(n) = i.to_i64() {
        return if n == -1 { (-2i64) as usize } else { n as usize };
    }
    // Fall back to a stable, self-consistent (not CPython-bit-exact) scan
    // for magnitudes beyond i64 — doesn't match CPython's real
    // mod-(2**61-1) big-int hash algorithm, but keeps the dict/set
    // invariant (equal values hash equal) for values sharing this
    // representation.
    let bytes = i.to_signed_bytes_le();
    let mut h: usize = 0;
    for (j, &b) in bytes.iter().enumerate() {
        h ^= (b as usize) << ((j % (std::mem::size_of::<usize>())) * 8);
    }
    h
}

/// Extracts (real, imaginary) from any of `complex`/`int`/`float`/`bool` —
/// used to let complex arithmetic transparently accept a real-number operand
/// on either side (`1 + 2j`, `2j * 3.0`, ...) without a combinatorial
/// explosion of match arms per numeric-type pairing.
pub(crate) fn as_complex_parts(obj: &PyObject) -> Option<(f64, f64)> {
    match obj {
        PyObject::Complex(re, im) => Some((*re, *im)),
        PyObject::Int(n) => n.to_f64().map(|f| (f, 0.0)),
        PyObject::Float(f) => Some((*f, 0.0)),
        PyObject::Bool(b) => Some((if *b { 1.0 } else { 0.0 }, 0.0)),
        _ => None,
    }
}

pub fn py_neg(val: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(i) = val.as_i64() {
        return Ok(py_int(-i));
    }
    if let Some(f) = val.as_f64() {
        return Ok(py_float(-f));
    }
    let obj = val.borrow();
    match &*obj {
        PyObject::Int(n) => Ok(py_int(-n.clone())),
        PyObject::Float(n) => Ok(py_float(-n)),
        PyObject::Complex(re, im) => Ok(PyObjectRef::imm(PyObject::Complex(-re, -im))),
        _ => Err(PyError::type_error(format!("bad operand type for unary -: '{}'", obj.type_name()))),
    }
}

pub fn py_not(val: &PyObjectRef) -> PyObjectRef {
    py_bool(!val.truthy())
}

pub fn py_pos(val: &PyObjectRef) -> PyResult<PyObjectRef> {
    // `+bool` yields the int 0/1 (a NEW int object, not the same bool) —
    // test_bool's test_math asserts `+False is not False`.
    if let PyObjectRef::SmallBool(b) = val {
        return Ok(py_int(if *b { 1 } else { 0 }));
    }
    if val.as_i64().is_some() || val.as_f64().is_some() {
        return Ok(val.clone());
    }
    let obj = val.borrow();
    match &*obj {
        PyObject::Int(_) | PyObject::Float(_) | PyObject::Complex(_, _) => {
            drop(obj);
            Ok(val.clone())
        }
        _ => Err(PyError::type_error(format!("bad operand type for unary +: '{}'", obj.type_name()))),
    }
}
