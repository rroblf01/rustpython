// Split from src/object/subscript.rs — slice helpers.
use super::*;

/// `slice.indices(length)` for an arbitrary-length sequence — mirrors
/// CPython's `PySlice_GetIndicesEx` with arbitrary-precision arithmetic
/// (negative start/stop are offset by `length`; for a negative step both
/// are clamped to `[-1, length-1]`, otherwise to `[0, length]`). Shared by
/// the `slice.indices` attribute, `range.__getitem__`, and sequence
/// subscripting, so `range(10)[slice]` agrees exactly with
/// `range(*slice.indices(10))` (CPython's own test_slice.py pins this).
pub(crate) fn slice_indices_values(
    start: &PyObjectRef,
    stop: &PyObjectRef,
    step: &PyObjectRef,
    length: &BigInt,
) -> PyResult<(BigInt, BigInt, BigInt)> {
    let one = BigInt::from(1);
    let zero = BigInt::from(0);
    let comp = |v: &PyObjectRef| -> PyResult<BigInt> {
        crate::object::to_index(v).map_err(|e| {
            if e.type_name() == "TypeError" {
                PyError::type_error(
                    "slice indices must be integers or None or have an __index__ method",
                )
            } else {
                e
            }
        })
    };
    let is_none = |v: &PyObjectRef| matches!(&*v.borrow(), PyObject::None);
    let step = if is_none(step) {
        one.clone()
    } else {
        let s = comp(step)?;
        if s == zero {
            return Err(PyError::value_error("slice step cannot be zero"));
        }
        s
    };
    let neg = step.sign() == num_bigint::Sign::Minus;
    let start = if is_none(start) {
        if neg {
            length - &one
        } else {
            zero.clone()
        }
    } else {
        comp(start)?
    };
    let stop = if is_none(stop) {
        if neg {
            -(length + &one)
        } else {
            length.clone()
        }
    } else {
        comp(stop)?
    };
    let clamp = |v: BigInt, lo: &BigInt, hi: &BigInt| -> BigInt {
        let v = if v.sign() == num_bigint::Sign::Minus {
            length + &v
        } else {
            v
        };
        let v = if &v < lo { lo.clone() } else { v };
        if &v > hi {
            hi.clone()
        } else {
            v
        }
    };
    let (res_start, res_stop) = if neg {
        let lo = BigInt::from(-1);
        let hi = length - &one;
        (clamp(start, &lo, &hi), clamp(stop, &lo, &hi))
    } else {
        (clamp(start, &zero, length), clamp(stop, &zero, length))
    };
    Ok((res_start, res_stop, step))
}

/// Real Python slice-index normalization for a sequence of length `len` —
/// mirrors CPython's own `PySlice_GetIndicesEx`. Converts a possibly-
/// negative, possibly-omitted (`None`) start/stop pair into concrete,
/// in-bounds `isize` values a caller can safely loop
/// `while i (< or >) stop { ...; i += step }` over and cast to `usize`
/// without ever going negative.
///
/// Was NOT applied consistently anywhere in this file before this fix:
/// `List`/`Tuple` read-slicing did `start_val.max(0).min(len)` — clamping a
/// negative value straight to 0 instead of first adding `len` (so
/// `[1,2,3,4,5][-3:]`, meaning "last 3 elements", silently returned the
/// WHOLE list instead — a silent wrong-answer bug, not a crash). `Str`/
/// `Bytes`/`ByteArray` read-slicing did no clamping at all, so a negative
/// start/stop was cast straight from a negative `isize` to `usize`,
/// wrapping around to an astronomical value and panicking on the first
/// array access (confirmed via the simplest possible repro: `"hello"[-3:]`
/// crashed the whole process). Negative slice bounds are one of the most
/// common idioms in all of Python (`seq[:-1]`, `seq[-n:]`) — this was a
/// severe, high-blast-radius bug hiding in plain sight.
pub(crate) fn normalize_slice_bounds(
    start: Option<isize>,
    stop: Option<isize>,
    step: isize,
    len: usize,
) -> (isize, isize) {
    let len = len as isize;
    if step > 0 {
        let start = match start {
            None => 0,
            Some(v) if v < 0 => (len + v).max(0),
            Some(v) => v.min(len),
        };
        let stop = match stop {
            None => len,
            Some(v) if v < 0 => (len + v).max(0),
            Some(v) => v.min(len),
        };
        (start, stop)
    } else {
        let start = match start {
            None => len - 1,
            Some(v) if v < 0 => (len + v).max(-1),
            Some(v) => v.min(len - 1),
        };
        let stop = match stop {
            None => -1,
            Some(v) if v < 0 => (len + v).max(-1),
            Some(v) => v.min(len - 1),
        };
        (start, stop)
    }
}

/// Extracts `(start, stop, step)` as `Option<isize>`/`isize` from a
/// `PyObject::Slice`'s three borrowed fields, ready to hand to
/// `normalize_slice_bounds`.
///
/// Rejects a literal `step=0` with a real `ValueError` — this interpreter's
/// `slice()`/`BUILD_SLICE` construction does NOT reject it up front (unlike
/// what an earlier version of this comment assumed), so `some_list[::0]`
/// previously reached the iteration loops below with `step_val = 0` and
/// hung the whole process forever (`i += 0` never advances past `stop_n`,
/// an infinite loop — confirmed via the simplest repro, `[1,2,3][::0]`).
/// Real CPython raises `ValueError: slice step cannot be zero` at the point
/// a zero-step slice is actually USED for indexing, matched here.
pub(crate) fn extract_slice_fields(
    start: &PyObjectRef,
    stop: &PyObjectRef,
    step: &PyObjectRef,
) -> PyResult<(Option<isize>, Option<isize>, isize)> {
    // Slice components must be int (or int-like via __index__) or None; a
    // component whose __index__ misbehaves must propagate its own
    // exception (e.g. RuntimeError), not be masked as TypeError.
    let to_opt = |v: &PyObjectRef| -> PyResult<Option<isize>> {
        if matches!(&*v.borrow(), PyObject::None) {
            return Ok(None);
        }
        let i = crate::object::to_index(v).map_err(|e| {
            if e.type_name() == "TypeError" {
                PyError::type_error(
                    "slice indices must be integers or None or have an __index__ method",
                )
            } else {
                e
            }
        })?;
        Ok(i.to_isize())
    };
    let step_val = if matches!(&*step.borrow(), PyObject::None) {
        1
    } else {
        let i = crate::object::to_index(step).map_err(|e| {
            if e.type_name() == "TypeError" {
                PyError::type_error(
                    "slice indices must be integers or None or have an __index__ method",
                )
            } else {
                e
            }
        })?;
        i.to_isize().unwrap_or(1)
    };
    if step_val == 0 {
        return Err(PyError::value_error("slice step cannot be zero"));
    }
    let start_val = to_opt(start)?;
    let stop_val = to_opt(stop)?;
    Ok((start_val, stop_val, step_val))
}
