// Split from src/object/ops_binary.rs — multiplication (`*`) with sequence repetition.
use super::*;
use crate::object::*;
use num_bigint::Sign;
use num_traits::One;
use std::collections::VecDeque;

pub fn py_mul(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        match ai.checked_mul(bi) {
            Some(result) => return Ok(py_int(result)),
            None => { /* fall through to BigInt path */ }
        }
    }
    if a.is_float_typed() || b.is_float_typed() {
        // A huge int being coerced to float overflows (CPython raises
        // OverflowError: `10**1000 * 1.0`).
        for v in [a, b] {
            if matches!(&*v.borrow(), PyObject::Int(_)) {
                let overflow = match v.as_f64() {
                    Some(f) => f.is_infinite(),
                    None => true,
                };
                if overflow {
                    return Err(PyError::overflow_error("int too large to convert to float"));
                }
            }
        }
        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
            return Ok(py_float(af * bf));
        }
    }
    if let Some(r) = try_dunder_binop(a, b, "__mul__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rmul__")? {
        return Ok(r);
    }
    // Sequence repetition (`seq * n`) accepts ANY object implementing
    // `__index__`, not just a plain `int` — real CPython's C implementation
    // converts the count via `PyNumber_AsSsize_t`, which itself falls back
    // to `__index__` for a non-int. This was missing entirely (only a bare
    // `PyObject::Int` count worked), confirmed via CPython's own
    // `test_index.py::test_repeat` (`self.seq * self.o` where `self.o` is a
    // plain class implementing only `__index__`). Convert BEFORE the main
    // match below by substituting a real `py_int` for whichever side is a
    // sequence-like paired with a non-int/non-float `Instance` — recursing
    // once with the substituted value reaches the same match arms a literal
    // int count would.
    let is_seq_like = |v: &PyObjectRef| {
        matches!(
            &*v.borrow(),
            PyObject::Str(_)
                | PyObject::List(_)
                | PyObject::Tuple(_)
                | PyObject::Bytes(_)
                | PyObject::ByteArray(_)
        )
    };
    let is_plain_instance = |v: &PyObjectRef| matches!(&*v.borrow(), PyObject::Instance { .. });
    if is_seq_like(a) && is_plain_instance(b) {
        if let Ok(n) = to_index(b) {
            return py_mul(a, &py_int(n));
        }
    } else if is_seq_like(b) && is_plain_instance(a) {
        if let Ok(n) = to_index(a) {
            return py_mul(&py_int(n), b);
        }
    }
    // deque subclass with hijacked __new__ returning non-deque should make `d * n` raise (test_bug_31608)
    {
        let is_deque_like = |o: &PyObjectRef| {
            if matches!(&*o.borrow(), PyObject::Deque { .. }) {
                true
            } else if let Some(n) = crate::object::native_backing_of(o) {
                matches!(&*n.borrow(), PyObject::Deque { .. })
            } else {
                false
            }
        };
        let has_custom_new = |o: &PyObjectRef| {
            if let PyObject::Instance { typ, .. } = &*o.borrow() {
                if let PyObject::Type { dict, .. } = &*typ.borrow() {
                    return dict.get_str("__new__").is_some();
                }
            }
            false
        };
        let is_int_like = |o: &PyObjectRef| o.as_i64().is_some() || crate::object::to_index(o).is_ok();
        if (has_custom_new(a) && is_deque_like(a) && is_int_like(b))
            || (has_custom_new(b) && is_deque_like(b) && is_int_like(a))
        {
            return Err(PyError::type_error("cannot create 'deque' instances"));
        }
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() * b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a * b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() * b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a * b.to_f64().unwrap())),
        // A negative (or zero) repetition count yields an EMPTY result for
        // every sequence type, matching real Python (`"abc" * -1 == ""`,
        // `[1] * -1 == []`, `(1,) * -1 == ()`, same for bytes/bytearray) —
        // this used to raise `ValueError` for any negative count on all six
        // sequence*int sites below instead, confirmed against a real
        // CPython interpreter first. `to_usize()` returns `None` for BOTH a
        // negative value AND one too large to fit `usize` — only the
        // latter is a genuine error (`OverflowError`/`MemoryError`), so the
        // sign must be checked explicitly to tell them apart.
        (PyObject::Str(s), PyObject::Int(n)) => {
            if let Some(n) = n.to_usize() {
                Ok(py_str(&s.repeat(n)))
            } else if n.sign() == Sign::Minus {
                // `'a' * -1` -> "" but `'a' * -2**100` -> OverflowError
                // (the magnitude overflows C ssize_t).
                if n.magnitude().bits() > 63 {
                    Err(PyError::overflow_error("repeated string is too long"))
                } else {
                    Ok(py_str(""))
                }
            } else {
                Err(PyError::overflow_error("repeated string is too long"))
            }
        }
        (PyObject::Int(n), PyObject::Str(s)) => {
            if let Some(n) = n.to_usize() {
                Ok(py_str(&s.repeat(n)))
            } else if n.sign() == Sign::Minus {
                if n.magnitude().bits() > 63 {
                    Err(PyError::overflow_error("repeated string is too long"))
                } else {
                    Ok(py_str(""))
                }
            } else {
                Err(PyError::overflow_error("repeated string is too long"))
            }
        }
        // Reflected forms (`2 * [1]`, `2 * (1,)`) were missing entirely —
        // only the `sequence * int` order was handled, not `int *
        // sequence` — confirmed against a real CPython interpreter
        // (`2 * (1,2) == (1, 2, 1, 2)`) alongside the negative-repeat-count
        // fix above.
        (PyObject::List(v), PyObject::Int(n)) | (PyObject::Int(n), PyObject::List(v)) => {
            if let Some(n) = n.to_usize() {
                // Pre-check + pre-reserve the total size (real CPython's
                // `list_resize` does the equivalent `new_allocated *
                // sizeof(PyObject*)` overflow check) — without this,
                // `lst * huge_n` grows the result one `extend()` at a time,
                // succeeding right up until it has consumed all of physical
                // RAM instead of failing fast. See `test_list.py`'s
                // `test_overflow`/`test_list_resize_overflow`, which expect
                // exactly `(MemoryError, OverflowError)` here.
                let mut result = Vec::new();
                match v.len().checked_mul(n) {
                    Some(total) if result.try_reserve_exact(total).is_ok() => {
                        for _ in 0..n {
                            result.extend(v.clone());
                        }
                        Ok(py_list(result))
                    }
                    _ => Err(PyError::memory_error("could not allocate list")),
                }
            } else if n.sign() == Sign::Minus {
                Ok(py_list(Vec::new()))
            } else {
                Err(PyError::memory_error("could not allocate list"))
            }
        }
        (PyObject::Tuple(v), PyObject::Int(n)) | (PyObject::Int(n), PyObject::Tuple(v)) => {
            // Real CPython returns the SAME tuple object for `tuple * 1`
            // (immutable optimization) — `id(s) == id(s*1)` holds, exercised
            // by `seq_tests.CommonTest.test_repeat`. Need the original
            // `PyObjectRef` (the `a`/`b` operands), not just `v`.
            if n.is_one() {
                return Ok(if matches!(&*a.borrow(), PyObject::Tuple(_)) {
                    a.clone()
                } else {
                    b.clone()
                });
            }
            if let Some(n) = n.to_usize() {
                let mut result = Vec::new();
                match v.len().checked_mul(n) {
                    Some(total) if result.try_reserve_exact(total).is_ok() => {
                        for _ in 0..n {
                            result.extend(v.clone());
                        }
                        Ok(py_tuple(result))
                    }
                    _ => Err(PyError::memory_error("could not allocate tuple")),
                }
            } else if n.sign() == Sign::Minus {
                Ok(py_tuple(Vec::new()))
            } else {
                Err(PyError::memory_error("could not allocate tuple"))
            }
        }
        (PyObject::Deque { data: v, maxlen }, PyObject::Int(n))
        | (PyObject::Int(n), PyObject::Deque { data: v, maxlen }) => {
            // `deque.__mul__`/`__rmul__` preserves the deque's maxlen and
            // truncates the repetition to its LAST `maxlen` items
            // (`deque('abc', maxlen=5) * 2` == `deque('bcabc')`, matching
            // CPython — `test_mul` in `test_deque.py`); n <= 0 yields an
            // empty deque (maxlen still preserved).
            if let Some(n) = n.to_usize() {
                let mut result = VecDeque::new();
                let ok = match maxlen {
                    Some(_) => true, // bounded by maxlen, cannot overflow
                    None => v
                        .len()
                        .checked_mul(n)
                        .map(|total| result.try_reserve_exact(total).is_ok())
                        .unwrap_or(false),
                };
                if ok {
                    for _ in 0..n {
                        for item in v.iter() {
                            result.push_back(item.clone());
                            if let Some(maxlen) = maxlen {
                                while result.len() > *maxlen {
                                    result.pop_front();
                                }
                            }
                        }
                    }
                    Ok(py_deque(result, *maxlen))
                } else {
                    Err(PyError::memory_error("could not allocate deque"))
                }
            } else if n.sign() == Sign::Minus {
                Ok(py_deque(VecDeque::new(), *maxlen))
            } else {
                Err(PyError::memory_error("could not allocate deque"))
            }
        }
        // `bytes`/`bytearray` repetition (`b'\0' * n`) — real, common idiom
        // for zero-padding/pre-sizing a buffer (real trigger: CPython's own
        // `dbm/dumb.py`, `f.write(b'\0'*(npos-pos))`) — was missing
        // entirely despite `str`/`list`/`tuple` all already supporting `*`.
        (PyObject::Bytes(v), PyObject::Int(n)) | (PyObject::Int(n), PyObject::Bytes(v)) => {
            if let Some(n) = n.to_usize() {
                Ok(PyObjectRef::imm(PyObject::Bytes(v.repeat(n))))
            } else if n.sign() == Sign::Minus {
                Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())))
            } else {
                Err(PyError::overflow_error("repeated bytes are too long"))
            }
        }
        (PyObject::ByteArray(v), PyObject::Int(n)) | (PyObject::Int(n), PyObject::ByteArray(v)) => {
            if let Some(n) = n.to_usize() {
                Ok(PyObjectRef::new(PyObject::ByteArray(v.repeat(n))))
            } else if n.sign() == Sign::Minus {
                Ok(PyObjectRef::new(PyObject::ByteArray(Vec::new())))
            } else {
                Err(PyError::overflow_error("repeated bytearray are too long"))
            }
        }
        (a, b) if matches!(a, PyObject::Complex(..)) || matches!(b, PyObject::Complex(..)) => {
            match (as_complex_parts(a), as_complex_parts(b)) {
                (Some((ar, ai)), Some((br, bi))) => Ok(PyObjectRef::imm(PyObject::Complex(
                    ar * br - ai * bi,
                    ar * bi + ai * br,
                ))),
                _ => Err(PyError::type_error(format!(
                    "unsupported operand type(s) for *: '{}' and '{}'",
                    a.type_name(),
                    b.type_name()
                ))),
            }
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for *: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
    }
}
