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
