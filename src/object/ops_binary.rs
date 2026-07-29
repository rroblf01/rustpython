// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds binary numeric/
// container operations (`+ - * / // % ** << >> | ^ &`) and the panic-
// tolerant stable merge sort used by `list.sort()`/`sorted()`.
use super::*;

pub fn try_dunder_binop(a: &PyObjectRef, b: &PyObjectRef, method: &str) -> PyResult<Option<PyObjectRef>> {
    let f = {
        let a_borrowed = a.borrow();
        match &*a_borrowed {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, method),
            _ => {
                // Try dunder method directly on the object (for builtin types like str, list, etc.)
                a_borrowed.get_attribute(method).ok()
            }
        }
    };
    if let Some(f) = f {
        // `call_bound_method`'s `BuiltinMethod` arm always prepends *both*
        // the method's own (placeholder, for a native type's dunder — see
        // `get_attribute`) `self_obj` *and* the `a` we pass here as a
        // second, separate `self_obj` parameter — so every native-type
        // dunder reached this way is written expecting 3 args: `[None,
        // self, other]`, not `[self, other]` (confirmed against `__mod__`,
        // `__contains__`, etc., which all read `args[1]`/`args[2]`
        // accordingly). `dict.__or__` was the one written inconsistently
        // with a 2-arg `[self, other]` assumption instead — fixed there
        // (not here) to match the established convention, since this
        // calling shape is what every *other* native dunder already
        // depends on.
        let result = call_bound_method(f, a.clone(), vec![b.clone()])?;
        if !is_not_implemented(&result) {
            return Ok(Some(result));
        }
    }
    // Not overridden anywhere in the mro: for a class that transparently
    // subclasses list/str (`class Foo(list): ...`), +/* on it should behave
    // like the same operation on the real native backing (dict supports
    // neither, so it's simply not in this list). `__and__`/`__or__`/
    // `__xor__`/`__sub__` are here specifically for `set` subclasses
    // (`&`/`|`/`^`/`-` are set's real operators) — added alongside `set`'s
    // own native-base migration, since subclassing `set` at all was
    // unsupported before then (nothing exercised this gap until now).
    if let Some(native) = native_backing_of(a) {
        let result = match method {
            "__add__" => Some(py_add(&native, b)),
            "__mul__" => Some(py_mul(&native, b)),
            "__and__" => Some(py_bit_and(&native, b)),
            "__or__" => Some(py_bit_or(&native, b)),
            "__xor__" => Some(py_bit_xor(&native, b)),
            "__sub__" => Some(py_sub(&native, b)),
            _ => None,
        };
        if let Some(result) = result {
            return result.map(Some);
        }
    }
    Ok(None)
}

pub fn py_add(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        match ai.checked_add(bi) {
            Some(result) => return Ok(py_int(result)),
            None => { /* fall through to BigInt path */ }
        }
    }
    if a.is_float_typed() || b.is_float_typed() {
        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
            return Ok(py_float(af + bf));
        }
    }
    if let Some(r) = try_dunder_binop(a, b, "__add__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__radd__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() + b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a + b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() + b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a + b.to_f64().unwrap())),
        (PyObject::Str(a), PyObject::Str(b)) => Ok(py_str(&format!("{}{}", a, b))),
        (PyObject::List(a), PyObject::List(b)) => {
            let mut v = a.clone();
            v.extend(b.clone());
            Ok(py_list(v))
        }
        (PyObject::Tuple(a), PyObject::Tuple(b)) => {
            let mut v = a.clone();
            v.extend(b.clone());
            Ok(py_tuple(v))
        }
        (PyObject::Bytes(a), PyObject::Bytes(b)) => {
            let mut v = a.clone();
            v.extend(b);
            Ok(PyObjectRef::imm(PyObject::Bytes(v)))
        }
        (a, b) if matches!(a, PyObject::Complex(..)) || matches!(b, PyObject::Complex(..)) => {
            match (as_complex_parts(a), as_complex_parts(b)) {
                (Some((ar, ai)), Some((br, bi))) => Ok(PyObjectRef::imm(PyObject::Complex(ar + br, ai + bi))),
                _ => Err(PyError::type_error(format!("unsupported operand type(s) for +: '{}' and '{}'", a.type_name(), b.type_name()))),
            }
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for +: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name())))
    }
}

pub fn py_sub(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        match ai.checked_sub(bi) {
            Some(result) => return Ok(py_int(result)),
            None => { /* fall through to BigInt path */ }
        }
    }
    if a.is_float_typed() || b.is_float_typed() {
        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
            return Ok(py_float(af - bf));
        }
    }
    if let Some(r) = try_dunder_binop(a, b, "__sub__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rsub__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() - b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a - b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() - b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a - b.to_f64().unwrap())),
        (PyObject::Set(a), PyObject::Set(b)) => set_difference(a, b, false),
        (PyObject::Set(a), PyObject::FrozenSet(b)) => set_difference(a, b, false),
        (PyObject::FrozenSet(a), PyObject::Set(b)) => set_difference(a, b, true),
        (PyObject::FrozenSet(a), PyObject::FrozenSet(b)) => set_difference(a, b, true),
        (a, b) if matches!(a, PyObject::Complex(..)) || matches!(b, PyObject::Complex(..)) => {
            match (as_complex_parts(a), as_complex_parts(b)) {
                (Some((ar, ai)), Some((br, bi))) => Ok(PyObjectRef::imm(PyObject::Complex(ar - br, ai - bi))),
                _ => Err(PyError::type_error(format!("unsupported operand type(s) for -: '{}' and '{}'", a.type_name(), b.type_name()))),
            }
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for -: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_mul(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        match ai.checked_mul(bi) {
            Some(result) => return Ok(py_int(result)),
            None => { /* fall through to BigInt path */ }
        }
    }
    if a.is_float_typed() || b.is_float_typed() {
        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
            return Ok(py_float(af * bf));
        }
    }
    if let Some(r) = try_dunder_binop(a, b, "__mul__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rmul__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() * b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a * b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() * b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a * b.to_f64().unwrap())),
        (PyObject::Str(s), PyObject::Int(n)) => {
            if let Some(n) = n.to_usize() {
                Ok(py_str(&s.repeat(n)))
            } else {
                Err(PyError::value_error("cannot multiply string by negative number"))
            }
        }
        (PyObject::Int(n), PyObject::Str(s)) => {
            if let Some(n) = n.to_usize() {
                Ok(py_str(&s.repeat(n)))
            } else {
                Err(PyError::value_error("cannot multiply string by negative number"))
            }
        }
        (PyObject::List(v), PyObject::Int(n)) => {
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
            } else {
                Err(PyError::value_error("cannot multiply list by negative number"))
            }
        }
        (PyObject::Tuple(v), PyObject::Int(n)) => {
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
            } else {
                Err(PyError::value_error("cannot multiply tuple by negative number"))
            }
        }
        // `bytes`/`bytearray` repetition (`b'\0' * n`) — real, common idiom
        // for zero-padding/pre-sizing a buffer (real trigger: CPython's own
        // `dbm/dumb.py`, `f.write(b'\0'*(npos-pos))`) — was missing
        // entirely despite `str`/`list`/`tuple` all already supporting `*`.
        (PyObject::Bytes(v), PyObject::Int(n)) | (PyObject::Int(n), PyObject::Bytes(v)) => {
            if let Some(n) = n.to_usize() {
                Ok(PyObjectRef::imm(PyObject::Bytes(v.repeat(n))))
            } else {
                Err(PyError::value_error("cannot multiply bytes by negative number"))
            }
        }
        (PyObject::ByteArray(v), PyObject::Int(n)) | (PyObject::Int(n), PyObject::ByteArray(v)) => {
            if let Some(n) = n.to_usize() {
                Ok(PyObjectRef::new(PyObject::ByteArray(v.repeat(n))))
            } else {
                Err(PyError::value_error("cannot multiply bytearray by negative number"))
            }
        }
        (a, b) if matches!(a, PyObject::Complex(..)) || matches!(b, PyObject::Complex(..)) => {
            match (as_complex_parts(a), as_complex_parts(b)) {
                (Some((ar, ai)), Some((br, bi))) => Ok(PyObjectRef::imm(PyObject::Complex(ar * br - ai * bi, ar * bi + ai * br))),
                _ => Err(PyError::type_error(format!("unsupported operand type(s) for *: '{}' and '{}'", a.type_name(), b.type_name()))),
            }
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for *: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_div(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 { return Err(PyError::zero_division()); }
        return Ok(py_float(ai as f64 / bi as f64));
    }
    if let Some(r) = try_dunder_binop(a, b, "__truediv__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rtruediv__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            Ok(py_float(a.to_f64().unwrap() / b.to_f64().unwrap()))
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float(a / b))
        }
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float(a.to_f64().unwrap() / b))
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            Ok(py_float(a / b.to_f64().unwrap()))
        }
        (a, b) if matches!(a, PyObject::Complex(..)) || matches!(b, PyObject::Complex(..)) => {
            match (as_complex_parts(a), as_complex_parts(b)) {
                (Some((ar, ai)), Some((br, bi))) => {
                    let denom = br * br + bi * bi;
                    if denom == 0.0 { return Err(PyError::zero_division()); }
                    Ok(PyObjectRef::imm(PyObject::Complex((ar * br + ai * bi) / denom, (ai * br - ar * bi) / denom)))
                }
                _ => Err(PyError::type_error(format!("unsupported operand type(s) for /: '{}' and '{}'", a.type_name(), b.type_name()))),
            }
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for /: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_floor_div(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 { return Err(PyError::zero_division()); }
        // `i64::MIN / -1` (and `% -1`) overflows outright — same classic
        // edge case as `py_mod`'s fix just above; fall back to BigInt
        // rather than let a plain `/`/`%` panic.
        if let (Some(q), Some(r)) = (ai.checked_div(bi), ai.checked_rem(bi)) {
            return if r != 0 && ((ai < 0) != (bi < 0)) {
                match q.checked_sub(1) {
                    Some(result) => Ok(py_int(result)),
                    None => Ok(py_int(BigInt::from(q) - 1)),
                }
            } else {
                Ok(py_int(q))
            };
        }
        let big_a = BigInt::from(ai);
        let big_b = BigInt::from(bi);
        return if big_a.sign() == Sign::Minus && &(&big_a % &big_b) != &BigInt::zero() {
            Ok(py_int((&big_a / &big_b) - 1))
        } else {
            Ok(py_int(&big_a / &big_b))
        };
    }
    if let Some(r) = try_dunder_binop(a, b, "__floordiv__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rfloordiv__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            if a.sign() == Sign::Minus && &(a % b) != &BigInt::zero() {
                Ok(py_int((a / b) - 1))
            } else {
                Ok(py_int(a / b))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float((a / b).floor()))
        }
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float((a.to_f64().unwrap() / b).floor()))
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            Ok(py_float((a / b.to_f64().unwrap()).floor()))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for //: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_mod(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 { return Err(PyError::zero_division()); }
        // `ai % bi` itself can panic outright (`i64::MIN % -1` overflows,
        // same classic edge case as division), and even when it doesn't,
        // `rem + bi` (both operands can be large-magnitude negatives) can
        // still overflow i64 — real, confirmed trigger: CPython's own
        // `test_range.py` exercises ranges near the i64 boundary. Fall back
        // to BigInt on either.
        //
        // The adjustment condition itself must compare the REMAINDER's
        // sign against the DIVISOR's sign (Python's `%` always returns a
        // result with the same sign as the divisor) — checking merely
        // `rem < 0` (the previous condition here, and in the BigInt arm
        // below) is wrong for a positive-dividend/negative-divisor pair:
        // `7 % -3` needs `1 + (-3) = -2`, but `1 < 0` is false, so it
        // wrongly returned the unadjusted `1`. Confirmed against real
        // Python: `7 % -3 == -2`, `-7 % -3 == -1` (not `-4`).
        if let Some(rem) = ai.checked_rem(bi) {
            if rem != 0 && (rem < 0) != (bi < 0) {
                if let Some(result) = rem.checked_add(bi) {
                    return Ok(py_int(result));
                }
            } else {
                return Ok(py_int(rem));
            }
        }
        let big_a = BigInt::from(ai);
        let big_b = BigInt::from(bi);
        let rem = &big_a % &big_b;
        return if !rem.is_zero() && (rem.sign() == Sign::Minus) != (big_b.sign() == Sign::Minus) {
            Ok(py_int(rem + big_b))
        } else {
            Ok(py_int(rem))
        };
    }
    if let Some(r) = try_dunder_binop(a, b, "__mod__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rmod__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            let rem = a % b;
            if !rem.is_zero() && (rem.sign() == Sign::Minus) != (b.sign() == Sign::Minus) {
                Ok(py_int(rem + b))
            } else {
                Ok(py_int(rem))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            py_float_mod(*a, *b)
        }
        // Mixed int/float `%` (`5 % 2.0`, `5.0 % 2`) was missing entirely —
        // fell to the `_` catch-all TypeError below instead of promoting
        // to float like every other mixed-numeric-tower operator here does.
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            py_float_mod(a.to_f64().unwrap(), *b)
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            py_float_mod(*a, b.to_f64().unwrap())
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for %: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

/// Python's `%` for floats follows the DIVISOR's sign (floor-mod), unlike
/// Rust's `%` (which follows the dividend, like C's `fmod`) — e.g. real
/// Python's `7.5 % -3.0 == -1.5`, not `1.5`.
fn py_float_mod(a: f64, b: f64) -> PyResult<PyObjectRef> {
    let rem = a % b;
    if rem != 0.0 && (rem < 0.0) != (b < 0.0) {
        Ok(py_float(rem + b))
    } else {
        Ok(py_float(rem))
    }
}

pub fn py_pow(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return Ok(py_float((ai as f64).powi(bi as i32))); }
        if bi == 0 { return Ok(py_int(1)); }
        if bi == 1 { return Ok(py_int(ai)); }
        // Real CPython promotes to an arbitrary-precision int the instant
        // a computation would overflow, regardless of how "small" the
        // exponent looks. The previous "use BigInt only when bi > 63"
        // heuristic was unsound two ways: (1) the boundary itself was off
        // by one — `2**63` (exponent exactly 63) fell into the FAST i64
        // path below and silently wrapped via `wrapping_mul` to
        // `i64::MIN` instead of the correct `9223372036854775808`; (2) an
        // exponent under 63 can still overflow i64 if the BASE is large
        // enough (`3**40` already exceeds i64::MAX). Confirmed via
        // CPython's own `test_math.testIsqrt`, which fed `2**e` for `e`
        // up to 200 into `isqrt` and got a spurious `ValueError:
        // isqrt() argument must be nonnegative` from the wrapped-negative
        // `2**63`. Using checked arithmetic and falling back to BigInt on
        // ANY overflow (not just large exponents) fixes both.
        if bi <= u32::MAX as i64 {
            if let Some(result) = ai.checked_pow(bi as u32) {
                return Ok(py_int(result));
            }
        }
        let big_a = BigInt::from(ai);
        let result = big_a.pow(bi as u32);
        return Ok(py_int(result));
    }
    if let Some(r) = try_dunder_binop(a, b, "__pow__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rpow__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if let Some(exp) = b.to_usize() {
                let result = a.pow(exp as u32);
                Ok(py_int(result))
            } else if b.is_zero() {
                Ok(py_int(BigInt::one()))
            } else if b.sign() == Sign::Minus {
                // For now, return float
                let f = a.to_f64().unwrap().powf(b.to_f64().unwrap());
                Ok(py_float(f))
            } else {
                Err(PyError::value_error("int too large to convert to int"))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a.powf(*b))),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap().powf(*b))),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a.powf(b.to_f64().unwrap()))),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for **: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_lshift(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return Err(PyError::value_error("negative shift count")); }
        return Ok(py_int(ai.wrapping_shl(bi as u32)));
    }
    if let Some(r) = try_dunder_binop(a, b, "__lshift__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rlshift__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            let shift = b.to_usize().ok_or_else(|| PyError::value_error("negative shift count"))?;
            Ok(py_int(a << shift))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for <<: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_rshift(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return Err(PyError::value_error("negative shift count")); }
        if bi >= 64 { return Ok(py_int(if ai < 0 { -1i64 } else { 0i64 })); }
        return Ok(py_int(ai.wrapping_shr(bi as u32)));
    }
    if let Some(r) = try_dunder_binop(a, b, "__rshift__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rrshift__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            let shift = b.to_usize().ok_or_else(|| PyError::value_error("negative shift count"))?;
            Ok(py_int(a >> shift))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for >>: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

/// Wraps a computed `PySet` result as either `PyObject::Set` or
/// `PyObject::FrozenSet`, matching the LEFT operand's own container type —
/// same rule real CPython uses (`frozenset() & set()` returns a
/// `frozenset`; `set() & frozenset()` returns a `set`).
fn wrap_set_result(result: PySet, as_frozen: bool) -> PyObjectRef {
    if as_frozen {
        PyObjectRef::imm(PyObject::FrozenSet(result))
    } else {
        PyObjectRef::new(PyObject::Set(result))
    }
}
fn set_union(a: &PySet, b: &PySet, as_frozen: bool) -> PyResult<PyObjectRef> {
    let mut result = a.clone();
    for item in b.to_vec() { result.add(item)?; }
    Ok(wrap_set_result(result, as_frozen))
}
fn set_intersection(a: &PySet, b: &PySet, as_frozen: bool) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() { if b.contains(&item)? { result.add(item)?; } }
    Ok(wrap_set_result(result, as_frozen))
}
fn set_difference(a: &PySet, b: &PySet, as_frozen: bool) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() { if !b.contains(&item)? { result.add(item)?; } }
    Ok(wrap_set_result(result, as_frozen))
}
fn set_symmetric_diff(a: &PySet, b: &PySet, as_frozen: bool) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() { if !b.contains(&item)? { result.add(item)?; } }
    for item in b.to_vec() { if !a.contains(&item)? { result.add(item)?; } }
    Ok(wrap_set_result(result, as_frozen))
}

fn i64_binop(a: &PyObjectRef, b: &PyObjectRef, f: impl Fn(i64, i64) -> i64) -> Option<PyResult<PyObjectRef>> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Some(Ok(py_int(f(ai, bi))));
    }
    None
}

pub fn py_bit_or(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    // Bool __or__ returns bool, not int (CPython: True | True -> True, but True | 1 -> int)
    if let (Some(av), Some(bv)) = (a.as_i64(), b.as_i64()) {
        if matches!(&*a.borrow(), PyObject::Bool(_)) && matches!(&*b.borrow(), PyObject::Bool(_)) {
            return Ok(py_bool((av | bv) != 0));
        }
    }
    if let Some(r) = i64_binop(a, b, |x, y| x | y) { return r; }
    if let Some(r) = try_dunder_binop(a, b, "__or__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__ror__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() | b)),
        (PyObject::Set(a), PyObject::Set(b)) => set_union(a, b, false),
        (PyObject::Set(a), PyObject::FrozenSet(b)) => set_union(a, b, false),
        (PyObject::FrozenSet(a), PyObject::Set(b)) => set_union(a, b, true),
        (PyObject::FrozenSet(a), PyObject::FrozenSet(b)) => set_union(a, b, true),
        (PyObject::Dict(a), PyObject::Dict(b)) => {
            let mut merged = PyDict::new();
            for k in a.keys() {
                if let Ok(Some(v)) = a.get(&k) { merged.set(k, v)?; }
            }
            for k in b.keys() {
                if let Ok(Some(v)) = b.get(&k) { merged.set(k, v)?; }
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(merged))))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for |: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_bit_xor(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    // Bool __xor__ returns bool, not int (CPython: True ^ True -> False, but True ^ 1 -> int)
    if let (Some(av), Some(bv)) = (a.as_i64(), b.as_i64()) {
        if matches!(&*a.borrow(), PyObject::Bool(_)) && matches!(&*b.borrow(), PyObject::Bool(_)) {
            return Ok(py_bool((av ^ bv) != 0));
        }
    }
    if let Some(r) = i64_binop(a, b, |x, y| x ^ y) { return r; }
    if let Some(r) = try_dunder_binop(a, b, "__xor__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rxor__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() ^ b)),
        (PyObject::Set(a), PyObject::Set(b)) => set_symmetric_diff(a, b, false),
        (PyObject::Set(a), PyObject::FrozenSet(b)) => set_symmetric_diff(a, b, false),
        (PyObject::FrozenSet(a), PyObject::Set(b)) => set_symmetric_diff(a, b, true),
        (PyObject::FrozenSet(a), PyObject::FrozenSet(b)) => set_symmetric_diff(a, b, true),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for ^: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_bit_and(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    // Bool __and__ returns bool, not int (CPython: True & True -> True, but True & 1 -> int)
    if let (Some(av), Some(bv)) = (a.as_i64(), b.as_i64()) {
        if matches!(&*a.borrow(), PyObject::Bool(_)) && matches!(&*b.borrow(), PyObject::Bool(_)) {
            return Ok(py_bool((av & bv) != 0));
        }
    }
    if let Some(r) = i64_binop(a, b, |x, y| x & y) { return r; }
    if let Some(r) = try_dunder_binop(a, b, "__and__")? { return Ok(r); }
    if let Some(r) = try_dunder_binop(b, a, "__rand__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() & b)),
        (PyObject::Set(a), PyObject::Set(b)) => set_intersection(a, b, false),
        (PyObject::Set(a), PyObject::FrozenSet(b)) => set_intersection(a, b, false),
        (PyObject::FrozenSet(a), PyObject::Set(b)) => set_intersection(a, b, true),
        (PyObject::FrozenSet(a), PyObject::FrozenSet(b)) => set_intersection(a, b, true),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for &: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

/// Stable merge sort driven by an arbitrary Python-visible `is_less(a, b)`
/// predicate — used instead of `[T]::sort_by` (`Vec::sort_by`) specifically
/// because Rust's sort implementation panics outright ("user-provided
/// comparison function does not correctly implement a total order") the
/// moment it detects an inconsistent comparator. Real CPython's own
/// `list.sort()`/`sorted()` tolerate this — deliberately exercised by
/// CPython's own test suite (e.g. `test_sort.py`'s `test_bug453523`-style
/// tests with intentionally broken `__lt__`) — producing a
/// not-necessarily-fully-sorted result rather than crashing. A
/// straightforward merge sort only ever does pairwise comparisons and
/// never relies on any total-order invariant to stay memory-safe, so it
/// can't panic regardless of what the predicate returns.
pub(crate) fn py_stable_sort_by<F>(mut items: Vec<PyObjectRef>, is_less: &F) -> Vec<PyObjectRef>
where
    F: Fn(&PyObjectRef, &PyObjectRef) -> bool,
{
    let n = items.len();
    if n <= 1 { return items; }
    let right = items.split_off(n / 2);
    let left = py_stable_sort_by(items, is_less);
    let right = py_stable_sort_by(right, is_less);
    let mut merged = Vec::with_capacity(left.len() + right.len());
    let mut li = 0;
    let mut ri = 0;
    while li < left.len() && ri < right.len() {
        if is_less(&right[ri], &left[li]) {
            merged.push(right[ri].clone());
            ri += 1;
        } else {
            merged.push(left[li].clone());
            li += 1;
        }
    }
    merged.extend_from_slice(&left[li..]);
    merged.extend_from_slice(&right[ri..]);
    merged
}
