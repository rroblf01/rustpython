// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds binary numeric/
// container operations (`+ - * / // % ** << >> | ^ &`) and the panic-
// tolerant stable merge sort used by `list.sort()`/`sorted()`.
use super::*;

mod mul;
pub use mul::py_mul;
mod pow;
pub use pow::py_pow;

pub fn try_dunder_binop(
    a: &PyObjectRef,
    b: &PyObjectRef,
    method: &str,
) -> PyResult<Option<PyObjectRef>> {
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

/// 3-argument dunder dispatch (`a.op(b, c)`), returning `Ok(None)` when the
/// method is absent or returns `NotImplemented` — used by 3-arg `pow()`.
pub fn try_dunder_ternop(
    a: &PyObjectRef,
    b: &PyObjectRef,
    c: &PyObjectRef,
    method: &str,
) -> PyResult<Option<PyObjectRef>> {
    let f = {
        let a_borrowed = a.borrow();
        match &*a_borrowed {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, method),
            _ => a_borrowed.get_attribute(method).ok(),
        }
    };
    if let Some(f) = f {
        let result = call_bound_method(f, a.clone(), vec![b.clone(), c.clone()])?;
        if !is_not_implemented(&result) {
            return Ok(Some(result));
        }
    }
    Ok(None)
}

/// Exact-`str` fresh-copy fast path for `+` (never mutates).
/// Skipping dunder dispatch for exact `str` pairs is semantics-preserving:
/// `str` defines neither `__add__` nor `__radd__` (see `attrs/str1.rs`/
/// `str2.rs` — only `__mul__`/`__mod__`), exact strs carry no instance dict,
/// and subclass instances are `Instance`, never `PyObject::Str`, so no
/// override can exist on either side. Returns `Ok(None)` when either side is
/// not exactly a `str`.
///
/// NOTE: in-place growth is deliberately NOT done here. A bare
/// `strong_count() == 2` check is unsound in the general handler: `y = x +
/// "b"` also presents count 2 (local `x` + operand) but the local survives,
/// so growing in place corrupts `x` (caught by `test_format`'s
/// `test_better_error_message_format`). CPython restricts in-place growth to
/// its `_BINARY_OP_INPLACE_ADD_UNICODE` super-instruction, which fires only
/// for `x = x + y` / `x += y` (STORE_FAST to the same slot, verified by
/// object identity). The same restriction applies here: in-place growth
/// lives only in the fused `SUPER_FASTC_BIN` / `SUPER_FAST2_BIN` handlers in
/// `src/vm/ops.rs` (which the fuser in `src/superinstr.rs` only emits for
/// same-slot store patterns), via `try_inplace_str_grow` below.
pub fn fast_str_add(left: &PyObjectRef, right: &PyObjectRef) -> PyResult<Option<PyObjectRef>> {
    let rb = right.borrow();
    let PyObject::Str(r) = &*rb else {
        return Ok(None);
    };
    let lb = left.borrow();
    if let PyObject::Str(l) = &*lb {
        let combined = crate::object::PyStr::concat_wtf8(l, r);
        return Ok(Some(py_str_pstr(&combined)));
    }
    Ok(None)
}

/// CPython-style in-place string growth for the fused same-slot handlers
/// (`x = x + y` / `x += y` where the store target is the load slot).
/// Sound iff ALL hold (checked here, mirroring CPython's DEOPT_IF identity
/// check plus the refcount reasoning in `_BINARY_OP_INPLACE_ADD_UNICODE`):
/// - `slot_val` (current content of the store slot) is the SAME allocation
///   as `left` (`Rc::ptr_eq`),
/// - that allocation has exactly 2 strong refs (the slot + the operand),
///   so no third party can observe the mutation,
/// - the `RefCell` is unborrowed right now (else fall back to a copy).
///
/// On success returns the (same, grown) object for the handler to store back
/// into the slot; on any doubt returns `Ok(None)` for the fresh-copy path.
pub fn try_inplace_str_grow(
    slot_val: &PyObjectRef,
    left: &PyObjectRef,
    right: &PyObjectRef,
) -> PyResult<Option<PyObjectRef>> {
    let rb = right.borrow();
    let PyObject::Str(r) = &*rb else {
        return Ok(None);
    };
    if let (PyObjectRef::Imm(slot_rc), PyObjectRef::Imm(left_rc)) = (slot_val, left) {
        if std::rc::Rc::ptr_eq(slot_rc, left_rc) && left.strong_count() == 2 {
            if let Ok(mut guard) = left_rc.try_borrow_mut() {
                if let PyObject::Str(l) = &mut *guard {
                    if l.reserve(r.len()) {
                        l.push_str_inplace(r.as_str());
                        return Ok(Some(left.clone()));
                    }
                }
            }
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
    if let Some(r) = fast_str_add(a, b)? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(a, b, "__add__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__radd__")? {
        return Ok(r);
    }
    // deque subclass with custom __new__ that returns non-deque should make
    // `d + deque(...)` / `d * n` raise TypeError (test_deque::test_bug_31608).
    // Detect via the subclass Instance's own dict containing "__new__".
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
                if dict.get_str("__new__").is_some() {
                    return true;
                }
            }
        }
        false
    };
    if (has_custom_new(a) && is_deque_like(b)) || (has_custom_new(b) && is_deque_like(a)) {
        return Err(PyError::type_error("cannot create 'deque' instances"));
    }
    // Deque subclasses without custom __new__ should still delegate to the
    // native backing and produce a new deque (handled in the match below via
    // the native_backing_of fallback). This check only fires for the hijacked
    // __new__ case that the test deliberately exercises.
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
        (
            PyObject::Deque {
                data: a,
                maxlen: am,
            },
            PyObject::Deque { data: b, .. },
        ) => {
            // `deque.__add__` preserves the LEFT operand's maxlen and
            // truncates the concatenation to it (`deque('abcdef', 4) +
            // deque('gh')` == `deque(['e','f','g','h'], maxlen=4)`).
            let mut data = a.clone();
            for item in b.iter() {
                data.push_back(item.clone());
                if let Some(maxlen) = am {
                    while data.len() > *maxlen {
                        data.pop_front();
                    }
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
            let a_cx = matches!(a, PyObject::Complex(..));
            let b_cx = matches!(b, PyObject::Complex(..));
            let a_real = matches!(a, PyObject::Int(_) | PyObject::Float(_) | PyObject::Bool(_));
            let b_real = matches!(b, PyObject::Int(_) | PyObject::Float(_) | PyObject::Bool(_));
            match (complex_parts_checked(a)?, complex_parts_checked(b)?) {
                (Some((ar, ai)), Some((br, bi))) => {
                    // complex + real → imag UNCHANGED (adding 0.0 would flip
                    // -0.0 to +0.0; test_add needs `complex(-0,-0)+(-0)` to
                    // keep imag -0.0). complex + complex → componentwise.
                    let (rr, ii) = if a_cx && b_real {
                        (ar + br, ai)
                    } else if b_cx && a_real {
                        (br + ar, bi)
                    } else {
                        (ar + br, ai + bi)
                    };
                    Ok(PyObjectRef::imm(PyObject::Complex(rr, ii)))
                }
                _ => Err(PyError::type_error(format!(
                    "unsupported operand type(s) for +: '{}' and '{}'",
                    a.type_name(),
                    b.type_name()
                ))),
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
            Err(PyError::type_error(format!(
                "unsupported operand type(s) for +: '{}' and '{}'",
                a_obj.type_name(),
                b_obj.type_name()
            )))
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
    if let Some(r) = try_dunder_binop(a, b, "__sub__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rsub__")? {
        return Ok(r);
    }
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
            match (complex_parts_checked(a)?, complex_parts_checked(b)?) {
                (Some((ar, ai)), Some((br, bi))) => {
                    // Sign-of-zero corner case (real CPython test:
                    // `test_complex.py`'s `test_sub`, matching CPython's own
                    // `_Py_rc_diff`/`_Py_c_diff` split in
                    // Objects/complexobject.c): when `a` is a plain
                    // real number (not itself a `Complex`), its imaginary
                    // part is not a genuine `0.0 - bi` subtraction — it's
                    // implicitly absent, so the result imaginary part is
                    // exactly `-bi` (negation), not `ai - bi` with a
                    // synthesized `ai = 0.0`. Those two formulas agree for
                    // every `bi` except `bi == 0.0`, where IEEE754 gives
                    // `0.0 - 0.0 == 0.0` but `-0.0 == -0.0`/`-(-0.0) ==
                    // 0.0` — CPython's real-object fast path always uses
                    // the negation form, so e.g. `-0.0 - complex(0.0, 0.0)`
                    // must yield `complex(-0.0, -0.0)`, not
                    // `complex(-0.0, 0.0)`.
                    let imag = if matches!(a, PyObject::Complex(..)) {
                        ai - bi
                    } else {
                        -bi
                    };
                    Ok(PyObjectRef::imm(PyObject::Complex(ar - br, imag)))
                }
                _ => Err(PyError::type_error(format!(
                    "unsupported operand type(s) for -: '{}' and '{}'",
                    a.type_name(),
                    b.type_name()
                ))),
            }
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for -: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
    }
}

pub fn py_div(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 {
            return Err(PyError::zero_division());
        }
        // Only safe as a pure `f64` division when BOTH operands are exactly
        // representable as `f64` (magnitude < 2**53) — IEEE754 division of
        // two exactly-represented doubles is itself correctly rounded, so
        // this reproduces CPython's correctly-rounded `int.__truediv__`
        // exactly. Outside that range this shortcut is wrong: rounding each
        // operand to the nearest f64 BEFORE dividing is a different
        // (looser) operation than correctly rounding the exact ratio, and
        // can be off by several ULPs — fall through to the correctly-
        // rounded BigInt path below instead.
        const SAFE: i64 = 1i64 << 53;
        if ai.unsigned_abs() < SAFE as u64 && bi.unsigned_abs() < SAFE as u64 {
            return Ok(py_float(ai as f64 / bi as f64));
        }
    }
    // Float fast path including SmallFloat (e.g. 0.0 is SmallFloat, not
    // PyObject::Float, so the match below would miss it and return nan
    // instead of ZeroDivisionError for 0.0/0.0 – breaking
    // linear_regression's constant-input check).
    if a.is_float_typed() || b.is_float_typed() {
        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
            if bf == 0.0 {
                return Err(PyError::zero_division());
            }
            return Ok(py_float(af / bf));
        }
    }
    if let Some(r) = try_dunder_binop(a, b, "__truediv__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rtruediv__")? {
        return Ok(r);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_float(correctly_rounded_int_div_f64(a, b)?)),
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 {
                return Err(PyError::zero_division());
            }
            Ok(py_float(a / b))
        }
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 {
                return Err(PyError::zero_division());
            }
            Ok(py_float(a.to_f64().unwrap() / b))
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() {
                return Err(PyError::zero_division());
            }
            Ok(py_float(a / b.to_f64().unwrap()))
        }
        (a, b) if matches!(a, PyObject::Complex(..)) || matches!(b, PyObject::Complex(..)) => {
            let a_cx = matches!(a, PyObject::Complex(..));
            let b_real = matches!(b, PyObject::Int(_) | PyObject::Float(_) | PyObject::Bool(_));
            match (complex_parts_checked(a)?, complex_parts_checked(b)?) {
                (Some((ar, ai)), Some((br, bi))) => {
                    if br == 0.0 && bi == 0.0 {
                        return Err(PyError::zero_division());
                    }
                    // complex / real → scalar division (keeps inf/nan signs:
                    // `complex(INF,NAN)/2 == (inf+nanj)`; complex/complex via
                    // Smith would give nan for the real part).
                    if a_cx && b_real {
                        return Ok(PyObjectRef::imm(PyObject::Complex(ar / br, ai / br)));
                    }
                    // real / complex → CPython `_Py_rc_quot` (its recovery uses
                    // `r.imag = 0.0 * (-a*y)`, giving the correct `-0.0` imag
                    // sign for `1/(inf+nanj)` that the complex/complex formula
                    // `ai*x - ar*y` would flush to `+0.0`; test_truediv).
                    if !a_cx {
                        let a_val = ar; // a is real, ai == 0.0
                        let (mut rr, mut ii);
                        if br.abs() >= bi.abs() {
                            let ratio = bi / br;
                            let denom = br + bi * ratio;
                            rr = a_val / denom;
                            ii = -a_val * ratio / denom;
                        } else {
                            let ratio = br / bi;
                            let denom = br * ratio + bi;
                            rr = a_val * ratio / denom;
                            ii = -a_val / denom;
                        }
                        if rr.is_nan()
                            && ii.is_nan()
                            && a_val.is_finite()
                            && (br.is_infinite() || bi.is_infinite())
                        {
                            let x = if br.is_infinite() {
                                br.signum()
                            } else {
                                0.0_f64.copysign(br)
                            };
                            let y = if bi.is_infinite() {
                                bi.signum()
                            } else {
                                0.0_f64.copysign(bi)
                            };
                            rr = 0.0 * (a_val * x);
                            ii = 0.0 * (-a_val * y);
                        }
                        return Ok(PyObjectRef::imm(PyObject::Complex(rr, ii)));
                    }
                    // Smith's algorithm (CPython's `_Py_c_quot`): avoids
                    // overflow and keeps `(1+0j) / (0.1+0j)` == 10.0 exactly,
                    // which the naive `(a*br + b*bi)/den` form loses.
                    let mut re;
                    let mut im;
                    if br.abs() >= bi.abs() {
                        let ratio = bi / br;
                        let denom = br + bi * ratio;
                        re = (ar + ai * ratio) / denom;
                        im = (ai - ar * ratio) / denom;
                    } else {
                        let ratio = br / bi;
                        let denom = br * ratio + bi;
                        re = (ar * ratio + ai) / denom;
                        im = (ai * ratio - ar) / denom;
                    }
                    // Recover infinities/zeros that computed as nan+nanj
                    // (CPython `_Py_c_quot` recovery, C11 G.5.2 `_Cdivd`).
                    if re.is_nan() && im.is_nan() {
                        if (ar.is_infinite() || ai.is_infinite())
                            && br.is_finite()
                            && bi.is_finite()
                        {
                            let x = if ar.is_infinite() { ar.signum() } else { 0.0_f64.copysign(ar) };
                            let y = if ai.is_infinite() { ai.signum() } else { 0.0_f64.copysign(ai) };
                            re = f64::INFINITY * (x * br + y * bi);
                            im = f64::INFINITY * (y * br - x * bi);
                        } else if (br.is_infinite() || bi.is_infinite())
                            && ar.is_finite()
                            && ai.is_finite()
                        {
                            let x = if br.is_infinite() { br.signum() } else { 0.0_f64.copysign(br) };
                            let y = if bi.is_infinite() { bi.signum() } else { 0.0_f64.copysign(bi) };
                            re = 0.0 * (ar * x + ai * y);
                            im = 0.0 * (ai * x - ar * y);
                        } else {
                            re = f64::NAN;
                            im = f64::NAN;
                        }
                    }
                    Ok(PyObjectRef::imm(PyObject::Complex(re, im)))
                }
                _ => Err(PyError::type_error(format!(
                    "unsupported operand type(s) for /: '{}' and '{}'",
                    a.type_name(),
                    b.type_name()
                ))),
            }
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for /: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
    }
}

// `DBL_MANT_DIG`/`DBL_MAX_EXP`/`DBL_MIN_EXP` from C's <float.h> for
// IEEE754 binary64 (`f64`).
const DBL_MANT_DIG: i64 = 53;
const DBL_MAX_EXP: i64 = 1024;
const DBL_MIN_EXP: i64 = -1021;

/// Correctly-rounded (round-half-to-even) conversion of the exact rational
/// `a/b` to the nearest `f64`, ported from CPython's `long_true_divide`
/// (`Objects/longobject.c`) — this is what `int.__truediv__` actually does,
/// and it is NOT the same operation as `a.to_f64() / b.to_f64()`: rounding
/// each operand to the nearest double independently, then dividing, loses
/// precision whenever either operand needs more than the double's 53
/// mantissa bits, and can land several ULPs away from the correctly-rounded
/// result of the exact ratio (verified against real CPython 3.14: over 200k
/// random `int/int` pairs beyond 2**53, the naive approach disagreed on
/// more than half). `Fraction.__float__`'s `frac_to_f64` (in
/// `src/modules/data/fractions.rs`) reuses this for the same reason.
pub fn correctly_rounded_int_div_f64(a: &BigInt, b: &BigInt) -> PyResult<f64> {
    if b.is_zero() {
        return Err(PyError::zero_division());
    }
    let negate = (a.sign() == Sign::Minus) != (b.sign() == Sign::Minus);
    if a.is_zero() {
        return Ok(if negate { -0.0 } else { 0.0 });
    }
    let a = a.abs();
    let b = b.abs();

    let a_bits = a.bits() as i64;
    let b_bits = b.bits() as i64;
    let diff = a_bits - b_bits;
    if diff > DBL_MAX_EXP {
        return Err(PyError::overflow_error(
            "integer division result too large for a float",
        ));
    }
    if diff < DBL_MIN_EXP - DBL_MANT_DIG - 1 {
        return Ok(if negate { -0.0 } else { 0.0 });
    }

    let shift = diff.max(DBL_MIN_EXP) - DBL_MANT_DIG - 2;
    let mut inexact = false;

    // x = abs(a * 2**-shift), tracking whether any shifted-out bit was set.
    let x: BigInt = if shift <= 0 {
        &a << (-shift) as u64
    } else {
        let shift_u = shift as u64;
        for i in 0..shift_u {
            if a.bit(i) {
                inexact = true;
                break;
            }
        }
        &a >> shift_u
    };

    // x //= b, remainder-nonzero also counts as inexact.
    let q = &x / &b;
    let r = &x % &b;
    if !r.is_zero() {
        inexact = true;
    }

    let x_bits = q.bits() as i64;
    debug_assert!(x_bits > 0);

    // Number of low bits of q that must be rounded away to fit DBL_MANT_DIG
    // significant bits (always 2 or 3, per CPython's comment/assertion).
    let extra_bits = x_bits.max(DBL_MIN_EXP - shift) - DBL_MANT_DIG;
    let extra_bits_u = extra_bits as u64;

    let guard_bit = q.bit(extra_bits_u - 1);
    let mut sticky = inexact;
    for i in 0..(extra_bits_u - 1) {
        if q.bit(i) {
            sticky = true;
            break;
        }
    }
    let kept_lsb = q.bit(extra_bits_u);
    let round_up = guard_bit && (sticky || kept_lsb);

    let dropped_mask = (BigInt::one() << extra_bits_u) - BigInt::one();
    let truncated = &q - (&q & &dropped_mask);
    let x_final = if round_up {
        &truncated + (BigInt::one() << extra_bits_u)
    } else {
        truncated
    };

    // Conversion to f64 is exact: x_final has at most DBL_MANT_DIG
    // significant bits (its low `extra_bits` bits are all zero).
    let dx = x_final.to_f64().unwrap_or(f64::INFINITY);

    // Check whether ldexp(dx, shift) would overflow a double (mirrors
    // CPython's check exactly, using the PRE-rounding bit length x_bits).
    if shift + x_bits >= DBL_MAX_EXP
        && (shift + x_bits > DBL_MAX_EXP || dx == 2f64.powi(x_bits as i32))
    {
        return Err(PyError::overflow_error(
            "integer division result too large for a float",
        ));
    }

    // A plain `dx * 2f64.powi(shift)` is wrong here: for a result near the
    // smallest subnormal double, `2f64.powi(shift)` alone underflows to
    // 0.0 (shift can be far more negative than any single finite double's
    // exponent range), silently zeroing an otherwise-representable tiny
    // result. `ldexp_f64` scales in bounded chunks instead, so it lands on
    // the correct (possibly subnormal) result.
    let result = crate::object::ldexp_f64(dx, shift as i32);
    Ok(if negate { -result } else { result })
}

pub fn py_floor_div(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 {
            return Err(PyError::zero_division());
        }
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
        let signs_differ = big_a.sign() != big_b.sign();
        return if signs_differ && &(&big_a % &big_b) != &BigInt::zero() {
            Ok(py_int((&big_a / &big_b) - 1))
        } else {
            Ok(py_int(&big_a / &big_b))
        };
    }
    if let Some(r) = try_dunder_binop(a, b, "__floordiv__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rfloordiv__")? {
        return Ok(r);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() {
                return Err(PyError::zero_division());
            }
            let signs_differ = a.sign() != b.sign();
            if signs_differ && &(a % b) != &BigInt::zero() {
                Ok(py_int((a / b) - 1))
            } else {
                Ok(py_int(a / b))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 {
                return Err(PyError::zero_division());
            }
            Ok(py_float((a / b).floor()))
        }
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 {
                return Err(PyError::zero_division());
            }
            Ok(py_float((a.to_f64().unwrap() / b).floor()))
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() {
                return Err(PyError::zero_division());
            }
            Ok(py_float((a / b.to_f64().unwrap()).floor()))
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for //: '{}' and '{}'",
            type_display_name(&a_obj),
            type_display_name(&b_obj)
        ))),
    }
}

/// Real type name for "unsupported operand type(s)" messages: an
/// `instance` should be reported as its actual class name (`Fraction`,
/// not `instance`), matching CPython.
fn type_display_name(o: &PyObject) -> String {
    if let PyObject::Instance { typ, .. } = o {
        crate::object::get_type_name_for_instance(typ)
    } else {
        o.type_name()
    }
}

pub fn py_mod(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 {
            return Err(PyError::zero_division());
        }
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
    if let Some(r) = try_dunder_binop(a, b, "__mod__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rmod__")? {
        return Ok(r);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() {
                return Err(PyError::zero_division());
            }
            let rem = a % b;
            if !rem.is_zero() && (rem.sign() == Sign::Minus) != (b.sign() == Sign::Minus) {
                Ok(py_int(rem + b))
            } else {
                Ok(py_int(rem))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 {
                return Err(PyError::zero_division());
            }
            py_float_mod(*a, *b)
        }
        // Mixed int/float `%` (`5 % 2.0`, `5.0 % 2`) was missing entirely —
        // fell to the `_` catch-all TypeError below instead of promoting
        // to float like every other mixed-numeric-tower operator here does.
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 {
                return Err(PyError::zero_division());
            }
            py_float_mod(a.to_f64().unwrap(), *b)
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() {
                return Err(PyError::zero_division());
            }
            py_float_mod(*a, b.to_f64().unwrap())
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for %: '{}' and '{}'",
            type_display_name(&a_obj),
            type_display_name(&b_obj)
        ))),
    }
}

/// Python's `%` for floats follows the DIVISOR's sign (floor-mod), unlike
/// Rust's `%` (which follows the dividend, like C's `fmod`) — e.g. real
/// Python's `7.5 % -3.0 == -1.5`, not `1.5`.
pub(crate) fn py_float_mod(a: f64, b: f64) -> PyResult<PyObjectRef> {
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

pub fn py_lshift(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 {
            return Err(PyError::value_error("negative shift count"));
        }
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
    if let Some(r) = try_dunder_binop(a, b, "__lshift__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rlshift__")? {
        return Ok(r);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            let shift = b
                .to_usize()
                .ok_or_else(|| PyError::value_error("negative shift count"))?;
            Ok(py_int(a << shift))
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for <<: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
    }
}

pub fn py_rshift(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 {
            return Err(PyError::value_error("negative shift count"));
        }
        if bi >= 64 {
            return Ok(py_int(if ai < 0 { -1i64 } else { 0i64 }));
        }
        return Ok(py_int(ai.wrapping_shr(bi as u32)));
    }
    if let Some(r) = try_dunder_binop(a, b, "__rshift__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rrshift__")? {
        return Ok(r);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            let shift = b
                .to_usize()
                .ok_or_else(|| PyError::value_error("negative shift count"))?;
            Ok(py_int(a >> shift))
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for >>: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
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
    for item in b.to_vec() {
        result.add(item)?;
    }
    Ok(wrap_set_result(result, as_frozen))
}
fn set_intersection(a: &PySet, b: &PySet, as_frozen: bool) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() {
        if b.contains(&item)? {
            result.add(item)?;
        }
    }
    Ok(wrap_set_result(result, as_frozen))
}
fn set_difference(a: &PySet, b: &PySet, as_frozen: bool) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() {
        if !b.contains(&item)? {
            result.add(item)?;
        }
    }
    Ok(wrap_set_result(result, as_frozen))
}
fn set_symmetric_diff(a: &PySet, b: &PySet, as_frozen: bool) -> PyResult<PyObjectRef> {
    Ok(wrap_set_result(a.symmetric_difference(b)?, as_frozen))
}

fn i64_binop(
    a: &PyObjectRef,
    b: &PyObjectRef,
    f: impl Fn(i64, i64) -> i64,
) -> Option<PyResult<PyObjectRef>> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Some(Ok(py_int(f(ai, bi))));
    }
    None
}

pub fn py_bit_or(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    // RegexFlag (IntFlag) `|` returns a RegexFlag (CPython: `re.I|re.S`).
    if let Some(r) = crate::object::regexflag_binop(a, b, |x, y| x | y) {
        return r;
    }
    // Bool __or__ returns bool, not int (CPython: True | True -> True, but True | 1 -> int)
    if let (Some(av), Some(bv)) = (a.as_i64(), b.as_i64()) {
        if matches!(&*a.borrow(), PyObject::Bool(_)) && matches!(&*b.borrow(), PyObject::Bool(_)) {
            return Ok(py_bool((av | bv) != 0));
        }
    }
    if let Some(r) = i64_binop(a, b, |x, y| x | y) {
        return r;
    }
    if let Some(r) = try_dunder_binop(a, b, "__or__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__ror__")? {
        return Ok(r);
    }
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
                if let Ok(Some(v)) = a.get(&k) {
                    merged.set(k, v)?;
                }
            }
            for k in b.keys() {
                if let Ok(Some(v)) = b.get(&k) {
                    merged.set(k, v)?;
                }
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(merged))))
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for |: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
    }
}

pub fn py_bit_xor(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(r) = crate::object::regexflag_binop(a, b, |x, y| x ^ y) {
        return r;
    }
    // Bool __xor__ returns bool, not int (CPython: True ^ True -> False, but True ^ 1 -> int)
    if let (Some(av), Some(bv)) = (a.as_i64(), b.as_i64()) {
        if matches!(&*a.borrow(), PyObject::Bool(_)) && matches!(&*b.borrow(), PyObject::Bool(_)) {
            return Ok(py_bool((av ^ bv) != 0));
        }
    }
    if let Some(r) = i64_binop(a, b, |x, y| x ^ y) {
        return r;
    }
    if let Some(r) = try_dunder_binop(a, b, "__xor__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rxor__")? {
        return Ok(r);
    }
    if let (Some((sa, frozen)), Some((sb, _))) = (extract_pyset(a), extract_pyset(b)) {
        return set_symmetric_diff(&sa, &sb, frozen);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() ^ b)),
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for ^: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
    }
}

pub fn py_bit_and(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(r) = crate::object::regexflag_binop(a, b, |x, y| x & y) {
        return r;
    }
    // Bool __and__ returns bool, not int (CPython: True & True -> True, but True & 1 -> int)
    if let (Some(av), Some(bv)) = (a.as_i64(), b.as_i64()) {
        if matches!(&*a.borrow(), PyObject::Bool(_)) && matches!(&*b.borrow(), PyObject::Bool(_)) {
            return Ok(py_bool((av & bv) != 0));
        }
    }
    if let Some(r) = i64_binop(a, b, |x, y| x & y) {
        return r;
    }
    if let Some(r) = try_dunder_binop(a, b, "__and__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rand__")? {
        return Ok(r);
    }
    if let (Some((sa, frozen)), Some((sb, _))) = (extract_pyset(a), extract_pyset(b)) {
        return set_intersection(&sa, &sb, frozen);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() & b)),
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for &: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
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
pub(crate) fn py_stable_sort_by<F>(
    items: Vec<PyObjectRef>,
    is_less: &F,
) -> PyResult<Vec<PyObjectRef>>
where
    F: Fn(&PyObjectRef, &PyObjectRef) -> PyResult<bool>,
{
    py_stable_sort_by_generic(items, is_less)
}

/// Generic form of [`py_stable_sort_by`] — same panic-tolerant merge sort,
/// but over an arbitrary element type `T` rather than only `PyObjectRef`.
/// Needed for `list.sort(key=...)`/`sorted(key=...)`'s decorate-sort-
/// undecorate path, which sorts `(key, original_item)` pairs rather than
/// bare `PyObjectRef`s — that path used to call `Vec::sort_by` directly and
/// inherited the same "user-provided comparison function does not
/// correctly implement a total order" panic this generic helper exists to
/// avoid (see `py_stable_sort_by`'s own doc comment; confirmed via
/// `test_sort.py`'s `cmp_to_key`-wrapped intentionally-inconsistent
/// comparator).
///
/// `is_less` returns `PyResult<bool>`, not a bare `bool` — a comparison can
/// genuinely raise (e.g. `int < str` -> `TypeError`, or a hostile `__lt__`
/// that raises `ValueError`), and real CPython's `list.sort()` aborts the
/// sort and propagates that exception rather than silently treating a
/// raising comparison as "not less than" (confirmed via `test_sort.py`'s
/// `TestOptimizedCompares` — `test_not_all_tuples`, `test_safe_object_
/// compare`, `test_unsafe_object_compare` all rely on the `TypeError`/
/// `ValueError` from an incompatible/hostile comparison actually reaching
/// the caller). This is independent from the "panic-tolerant" property
/// above: an inconsistent-but-non-raising comparator (returns `Ok(bool)`
/// values that don't form a total order) is still tolerated exactly as
/// before, we just no longer swallow a genuine `Err`.
pub(crate) fn py_stable_sort_by_generic<T, F>(mut items: Vec<T>, is_less: &F) -> PyResult<Vec<T>>
where
    T: Clone,
    F: Fn(&T, &T) -> PyResult<bool>,
{
    let n = items.len();
    if n <= 1 {
        return Ok(items);
    }
    let right = items.split_off(n / 2);
    let left = py_stable_sort_by_generic(items, is_less)?;
    let right = py_stable_sort_by_generic(right, is_less)?;
    let mut merged = Vec::with_capacity(left.len() + right.len());
    let mut li = 0;
    let mut ri = 0;
    while li < left.len() && ri < right.len() {
        if is_less(&right[ri], &left[li])? {
            merged.push(right[ri].clone());
            ri += 1;
        } else {
            merged.push(left[li].clone());
            li += 1;
        }
    }
    merged.extend_from_slice(&left[li..]);
    merged.extend_from_slice(&right[ri..]);
    Ok(merged)
}
