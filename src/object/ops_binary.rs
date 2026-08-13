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

/// CPython's `unicode_concatenate`-style in-place string growth is NOT
/// reliably possible here: the VM clones references through the eval stack,
/// so a `s = s + "x"` left operand arrives with strong_count 3 (local +
/// popped value + frame), never 1. A refcount heuristic would corrupt
/// strings that genuinely have other live references. Repeated `+` stays
/// quadratic; `''.join(...)` is the fast path (already ~CPython speed).

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
        (PyObject::Deque { data: a, maxlen: am }, PyObject::Deque { data: b, .. }) => {
            // `deque.__add__` preserves the LEFT operand's maxlen and
            // truncates the concatenation to it (`deque('abcdef', 4) +
            // deque('gh')` == `deque(['e','f','g','h'], maxlen=4)`).
            let mut data = a.clone();
            for item in b.iter() {
                data.push_back(item.clone());
                if let Some(maxlen) = am {
                    while data.len() > *maxlen { data.pop_front(); }
                }
            }
            Ok(py_deque(data, *am))
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
        (PyObject::Bytes(a), PyObject::ByteArray(b)) => {
            let mut v = a.clone();
            v.extend(b);
            Ok(PyObjectRef::imm(PyObject::Bytes(v)))
        }
        (PyObject::ByteArray(a), PyObject::ByteArray(b)) => {
            let mut v = a.clone();
            v.extend(b);
            Ok(PyObjectRef::new(PyObject::ByteArray(v)))
        }
        (PyObject::ByteArray(a), PyObject::Bytes(b)) => {
            let mut v = a.clone();
            v.extend(b);
            Ok(PyObjectRef::new(PyObject::ByteArray(v)))
        }
        (a, b) if matches!(a, PyObject::Complex(..)) || matches!(b, PyObject::Complex(..)) => {
            match (as_complex_parts(a), as_complex_parts(b)) {
                (Some((ar, ai)), Some((br, bi))) => Ok(PyObjectRef::imm(PyObject::Complex(ar + br, ai + bi))),
                _ => Err(PyError::type_error(format!("unsupported operand type(s) for +: '{}' and '{}'", a.type_name(), b.type_name()))),
            }
        }
        // A class transparently subclassing a native container (`class
        // MyList(list): pass`) with no explicit `__add__` anywhere in its
        // mro falls through to here as a plain `PyObject::Instance`,
        // matching none of the arms above (which only match the RAW
        // native variants) — `MyList([1,2]) + MyList([3,4])` raised a
        // spurious `TypeError` instead of concatenating, exactly the same
        // "native dunders aren't real Python-callable methods, so mro
        // lookup finds nothing" gap already fixed for `==`/`!=` just above
        // in `ops_compare.rs`. Delegate to the native backing (or the
        // operand itself, if it's already a raw native value on one side,
        // e.g. `MyList([1,2]) + [3,4]`) by recursing with the unwrapped
        // values.
        _ => {
            let a_native = native_backing_of(a);
            let b_native = native_backing_of(b);
            if a_native.is_some() || b_native.is_some() {
                let a_use = a_native.unwrap_or_else(|| a.clone());
                let b_use = b_native.unwrap_or_else(|| b.clone());
                drop(a_obj);
                drop(b_obj);
                return py_add(&a_use, &b_use);
            }
            Err(PyError::type_error(format!("unsupported operand type(s) for +: '{}' and '{}'",
                a_obj.type_name(), b_obj.type_name())))
        }
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
    if let (Some((sa, frozen)), Some((sb, _))) = (extract_pyset(a), extract_pyset(b)) {
        return set_difference(&sa, &sb, frozen);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() - b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a - b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() - b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a - b.to_f64().unwrap())),
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
    let is_seq_like = |v: &PyObjectRef| matches!(&*v.borrow(), PyObject::Str(_) | PyObject::List(_) | PyObject::Tuple(_) | PyObject::Bytes(_) | PyObject::ByteArray(_));
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
                Ok(py_str(""))
            } else {
                Err(PyError::overflow_error("repeated string is too long"))
            }
        }
        (PyObject::Int(n), PyObject::Str(s)) => {
            if let Some(n) = n.to_usize() {
                Ok(py_str(&s.repeat(n)))
            } else if n.sign() == Sign::Minus {
                Ok(py_str(""))
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
                return Ok(if matches!(&*a.borrow(), PyObject::Tuple(_)) { a.clone() } else { b.clone() });
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
        (PyObject::Deque { data: v, maxlen }, PyObject::Int(n)) | (PyObject::Int(n), PyObject::Deque { data: v, maxlen }) => {
            // `deque.__mul__`/`__rmul__` preserves the deque's maxlen and
            // truncates the repetition to its LAST `maxlen` items
            // (`deque('abc', maxlen=5) * 2` == `deque('bcabc')`, matching
            // CPython — `test_mul` in `test_deque.py`); n <= 0 yields an
            // empty deque (maxlen still preserved).
            if let Some(n) = n.to_usize() {
                let mut result = VecDeque::new();
                let ok = match maxlen {
                    Some(_) => true, // bounded by maxlen, cannot overflow
                    None => v.len().checked_mul(n).map(|total| result.try_reserve_exact(total).is_ok()).unwrap_or(false),
                };
                if ok {
                    for _ in 0..n {
                        for item in v.iter() {
                            result.push_back(item.clone());
                            if let Some(maxlen) = maxlen {
                                while result.len() > *maxlen { result.pop_front(); }
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
    // A zero result takes the DIVISOR's sign (CPython: -0.0 % 1.0 == 0.0,
    // -0.0 % -1.0 == -0.0) — a plain `rem != 0.0` check treats -0.0 as "no
    // adjustment" and returns the wrong sign.
    if rem == 0.0 {
        return Ok(py_float(if b.is_sign_negative() { -0.0 } else { 0.0 }));
    }
    if (rem < 0.0) != (b < 0.0) {
        Ok(py_float(rem + b))
    } else {
        Ok(py_float(rem))
    }
}

/// Shared `float ** float` (also used for the mixed int/float cases)
/// helper for the `**` operator / `pow()` builtin — matches real CPython's
/// `float.__pow__`: `0.0 ** negative` raises `ZeroDivisionError` (NOT
/// `math.pow`'s own `ValueError: math domain error` — the two raise
/// DIFFERENT exception types for the same mathematical case), and a
/// genuine overflow (both operands finite, result isn't) raises
/// `OverflowError` instead of silently returning `inf`. Found via
/// CPython's own `test_math.py`/operator-level `pow()` overflow checks.
fn py_pow_float(x: f64, y: f64) -> PyResult<PyObjectRef> {
    // Only a FINITE negative exponent is an error — `0.0 ** -inf`
    // legitimately diverges to `inf` (same IEEE-754 `pow()` semantics as
    // `math.pow`'s analogous domain-error check).
    if x == 0.0 && y < 0.0 && y.is_finite() {
        return Err(PyError::ZeroDivisionError("0.0 cannot be raised to a negative power".to_string()));
    }
    // A finite negative base with a NON-INTEGER exponent defers to complex
    // pow (CPython: (-2.0)**0.5 is complex ~ (8.66e-17+1.41j)). -INF stays
    // on the real path — IEEE powf(-inf, -0.5) == 0.0, (-inf)**0.5 == +inf,
    // which the complex path would wrongly turn into a signed zero/NaN.
    if x < 0.0 && x.is_finite() && y.fract() != 0.0 && y.is_finite() {
        let r = (-x).powf(y);
        let theta = y * std::f64::consts::PI;
        return Ok(PyObjectRef::imm(PyObject::Complex(r * theta.cos(), r * theta.sin())));
    }
    let result = x.powf(y);
    if result.is_infinite() && x.is_finite() && y.is_finite() {
        return Err(PyError::overflow_error("(34, 'Numerical result out of range')"));
    }
    Ok(py_float(result))
}

pub fn py_pow(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return py_pow_float(ai as f64, bi as f64); }
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
        (PyObject::Float(a), PyObject::Float(b)) => py_pow_float(*a, *b),
        (PyObject::Int(a), PyObject::Float(b)) => py_pow_float(a.to_f64().unwrap(), *b),
        (PyObject::Float(a), PyObject::Int(b)) => py_pow_float(*a, b.to_f64().unwrap()),
        // `complex ** (int|float|complex)` and `(int|float) ** complex` were
        // entirely unhandled — found via CPython's own `test_complex.py`.
        // Uses exact repeated-squaring for a real integer exponent (matching
        // real CPython's own fast path, and precise for e.g. `(1+2j)**2`
        // rather than accumulating log/exp floating-point error), falling
        // back to the general `z**w = exp(w * ln z)` polar-form identity
        // otherwise (fractional or complex exponents).
        _ if as_complex_parts(&a_obj).is_some() && as_complex_parts(&b_obj).is_some()
            && (matches!(&*a_obj, PyObject::Complex(_, _)) || matches!(&*b_obj, PyObject::Complex(_, _))) =>
        {
            let (are, aim) = as_complex_parts(&a_obj).unwrap();
            let (bre, bim) = as_complex_parts(&b_obj).unwrap();
            complex_pow(are, aim, bre, bim)
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for **: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

fn complex_mul(are: f64, aim: f64, bre: f64, bim: f64) -> (f64, f64) {
    (are * bre - aim * bim, are * bim + aim * bre)
}

fn complex_pow_int(are: f64, aim: f64, n: i64) -> (f64, f64) {
    let neg = n < 0;
    let mut n = n.unsigned_abs();
    let mut result = (1.0f64, 0.0f64);
    let mut base = (are, aim);
    while n > 0 {
        if n & 1 == 1 { result = complex_mul(result.0, result.1, base.0, base.1); }
        base = complex_mul(base.0, base.1, base.0, base.1);
        n >>= 1;
    }
    if neg {
        let denom = result.0 * result.0 + result.1 * result.1;
        (result.0 / denom, -result.1 / denom)
    } else {
        result
    }
}

fn complex_pow(are: f64, aim: f64, bre: f64, bim: f64) -> PyResult<PyObjectRef> {
    let base_zero = are == 0.0 && aim == 0.0;
    // A non-finite result computed from FINITE inputs is an overflow
    // (repeated squaring / exp(w*ln z) spill to inf/nan); NaN inputs legitimately
    // propagate NaN instead.
    let inputs_finite = are.is_finite() && aim.is_finite() && bre.is_finite() && bim.is_finite();
    let overflow = |re: f64, im: f64| -> PyResult<PyObjectRef> {
        if inputs_finite && (!re.is_finite() || !im.is_finite()) {
            Err(PyError::overflow_error("complex exponentiation"))
        } else {
            Ok(PyObjectRef::imm(PyObject::Complex(re, im)))
        }
    };
    if bim == 0.0 && bre.fract() == 0.0 && bre.abs() < 1e15 {
        // Integer exponent: 0 to a negative power is an error.
        if base_zero && bre < 0.0 {
            return Err(PyError::zero_division());
        }
        let (re, im) = complex_pow_int(are, aim, bre as i64);
        return overflow(re, im);
    }
    // General case: z^w = exp(w * ln z), ln z = ln|z| + i*arg(z). A zero
    // base raised to a negative or complex power is a domain error; 0 to a
    // positive real power is 0; 0 to zero power is 1.
    if base_zero {
        if bim != 0.0 || bre < 0.0 {
            return Err(PyError::zero_division());
        }
        if bre == 0.0 {
            return Ok(PyObjectRef::imm(PyObject::Complex(1.0, 0.0)));
        }
        return Ok(PyObjectRef::imm(PyObject::Complex(0.0, 0.0)));
    }
    let r = (are * are + aim * aim).sqrt();
    let theta = aim.atan2(are);
    let (ere, eim) = complex_mul(bre, bim, r.ln(), theta);
    let exp_re = ere.exp();
    let (re, im) = (exp_re * eim.cos(), exp_re * eim.sin());
    overflow(re, im)
}

pub fn py_lshift(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return Err(PyError::value_error("negative shift count")); }
        // Was: unconditional `wrapping_shl` — Rust's `wrapping_shl` reduces
        // the SHIFT AMOUNT modulo 64 (not the result), so `1 << 50000`
        // computed `1i64.wrapping_shl(50000 % 64)` == `1 << 16` == `65536`
        // instead of the correct ~15,000-digit bigint — a silent, badly
        // wrong result for any shift amount that doesn't fit i64, not an
        // error or a slow-but-correct path. Confirmed via CPython's own
        // `test_pow.py::test_big_exp` (`1 << 50000`). Real CPython promotes
        // to arbitrary precision the instant a shift would lose bits —
        // same "checked op, BigInt fallback on overflow" pattern as
        // `py_pow`'s own `checked_pow` above. A round-trip check
        // (`result >> bi == ai`) after `checked_shl` catches every
        // overflowing case (including negative `ai`, where `>>` is
        // arithmetic/sign-extending in Rust, matching two's-complement
        // semantics), not just `bi >= 64`.
        if ai == 0 {
            return Ok(py_int(0));
        }
        if bi < 63 {
            if let Some(result) = ai.checked_shl(bi as u32) {
                if result >> (bi as u32) == ai {
                    return Ok(py_int(result));
                }
            }
        }
        let big_a = BigInt::from(ai);
        return Ok(py_int(big_a << (bi as usize)));
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

/// Clones the `PySet` out of `obj` (if it's a `Set`/`FrozenSet`) via a
/// SHORT borrow that's already dropped by the time this returns — paired
/// `bool` is `true` for `FrozenSet`. Used by `py_sub`/`py_bit_or`/
/// `py_bit_xor`/`py_bit_and`'s set arms specifically so their actual
/// `set_union`/`set_intersection`/`set_difference`/`set_symmetric_diff`
/// computation (which internally calls `.equals()` against colliding
/// members, possibly running an arbitrary hostile `__eq__`) never runs
/// while EITHER operand's own borrow is still held. The general match
/// below (`a.borrow()`/`b.borrow()` held for its whole body) would
/// otherwise panic with "RefCell already (mutably) borrowed" the instant
/// such a callback reentrantly touched either operand set — real,
/// deliberate CPython regression test: `test_set.py`'s
/// `check_set_op_does_not_crash`/`make_sets_of_bad_objects`.
pub(crate) fn extract_pyset(obj: &PyObjectRef) -> Option<(PySet, bool)> {
    match &*obj.borrow() {
        PyObject::Set(s) => Some((s.clone(), false)),
        PyObject::FrozenSet(s) => Some((s.clone(), true)),
        _ => None,
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
    if let (Some((sa, frozen)), Some((sb, _))) = (extract_pyset(a), extract_pyset(b)) {
        return set_union(&sa, &sb, frozen);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() | b)),
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
    if let (Some((sa, frozen)), Some((sb, _))) = (extract_pyset(a), extract_pyset(b)) {
        return set_symmetric_diff(&sa, &sb, frozen);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() ^ b)),
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
    if let (Some((sa, frozen)), Some((sb, _))) = (extract_pyset(a), extract_pyset(b)) {
        return set_intersection(&sa, &sb, frozen);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() & b)),
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
