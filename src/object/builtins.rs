// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the ~79 standalone
// `pub fn builtin_*` free functions (the builtins module's actual
// implementations: print, len, isinstance, issubclass, format, iter/next,
// eval/exec, and so on).
use super::*;


mod print;
pub use print::builtin_print;
pub(crate) use print::{call_method_rebound, print_with_vm};

mod builtin_type;
pub use builtin_type::{builtin_type_of, type_new_builtin};
pub(crate) use builtin_type::{dict_arg_to_hashmap, get_primitive_type, seed_primitive_type_cache};

mod attribute;
pub use attribute::{builtin_ascii, builtin_bin, builtin_chr, builtin_delattr, builtin_exit, builtin_getattr, builtin_hasattr, builtin_hex, builtin_input, builtin_oct, builtin_ord, builtin_setattr};

mod int;
pub use int::{builtin_int, builtin_int_from_bytes};
pub(crate) use int::check_int_str_digit_limit;

mod numeric;
pub use numeric::{builtin_abs, builtin_divmod, builtin_pow, builtin_round};
pub(crate) use numeric::{bigint_gcd, bigint_mod_inverse, round_half_even_rat};

mod functional;
pub use functional::{builtin_all, builtin_any, builtin_breakpoint, builtin_callable, builtin_enumerate, builtin_max, builtin_min, builtin_sorted, builtin_sum};

pub fn builtin_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("len() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Str(s) => Ok(py_int(s.chars().count())),
        PyObject::List(v) => Ok(py_int(v.len())),
        PyObject::Deque { data, .. } => Ok(py_int(data.len())),
        PyObject::Tuple(v) => Ok(py_int(v.len())),
        PyObject::Dict(d) => Ok(py_int(d.len())),
        PyObject::Globals(g) => Ok(py_int(g.borrow().len())),
        PyObject::Set(s) => Ok(py_int(s.len())),
        PyObject::FrozenSet(s) => Ok(py_int(s.len())),
        PyObject::Range { start, stop, step } => {
            let len = crate::object::ops_contains::range_len_values(start, stop, step);
            if len.to_i64().is_none() {
                return Err(PyError::overflow_error(
                    "Python int too large to convert to C ssize_t",
                ));
            }
            Ok(py_int(len))
        }
        PyObject::Bytes(b) => Ok(py_int(b.len())),
        PyObject::ByteArray(b) => Ok(py_int(b.len())),
        PyObject::Array(arr) => Ok(py_int(arr.data.len())),
        PyObject::MemoryView { .. } => {
            drop(obj);
            Ok(py_int(mv_len(&args[0])? as i64))
        }
        // Real Python's `list_iterator`/`range_iterator`/etc. all support
        // `len()` — it reports the number of REMAINING elements, not the
        // original sequence's length (used by `operator.length_hint`, and
        // directly by real code — real trigger: CPython's own
        // `test_iterlen.py`, whose whole purpose is exercising this exact
        // protocol across iterator types).
        PyObject::ListIter { list, index } => Ok(py_int(list.len().saturating_sub(*index))),
        PyObject::DequeIter {
            deque,
            index,
            start_len,
        } => {
            let remaining = {
                let dq = deque.borrow();
                if let PyObject::Deque { data, .. } = &*dq {
                    if data.len() != *start_len {
                        None
                    } else {
                        Some(data.len().saturating_sub(*index))
                    }
                } else {
                    None
                }
            };
            match remaining {
                Some(n) => Ok(py_int(n)),
                None => Ok(py_int(0)),
            }
        }
        PyObject::RangeIter {
            current,
            stop,
            step,
        } => {
            // Use BigInt throughout: `current`/`stop` can be near the i64
            // boundary (a range_iterator unpickled with adversarial bounds,
            // or a real near-i64::MAX/MIN range), and this arithmetic used
            // to overflow-panic in plain i64 (`stop - current + step - 1`)
            // instead of just returning the (possibly huge, but always
            // representable) remaining count.
            let current = current.clone();
            let stop = stop.clone();
            let step = step.clone();
            let zero = BigInt::from(0);
            let remaining = if step > zero && current < stop {
                (&stop - &current + &step - BigInt::from(1)) / &step
            } else if step < zero && current > stop {
                (&current - &stop - &step - BigInt::from(1)) / (-&step)
            } else {
                zero.clone()
            };
            Ok(py_int(remaining.max(zero)))
        }
        PyObject::Instance { typ, dict } => {
            let f = lookup_dunder_via_mro(typ, "__len__");
            let native = dict.get(NATIVE_BACKING_KEY).cloned();
            let type_name = obj.type_name();
            // Drop the borrow on args[0] before calling into `__len__` —
            // holding it across the call panics with "RefCell already
            // borrowed" the moment `__len__` mutates `self` (real trigger:
            // CPython's own `test_enumerate.py`'s `SeqWithWeirdLen.__len__`,
            // which does `self.called = True`).
            drop(obj);
            if let Some(f) = f {
                let result = call_bound_method(f, args[0].clone(), vec![])?;
                let n = result.borrow();
                if let PyObject::Int(i) = &*n {
                    // Real CPython rejects a negative `__len__()` result
                    // with `ValueError: __len__() should return >= 0` —
                    // this was missing entirely, silently accepting -1 as
                    // a length. Confirmed via CPython's own
                    // `test_bool.test_sane_len`, which asserts `bool()`'s
                    // and `len()`'s error messages for the same bad
                    // `__len__` values are identical — `bool()` delegates
                    // to this same function specifically so that holds.
                    if i.sign() == Sign::Minus {
                        return Err(PyError::value_error("__len__() should return >= 0"));
                    }
                    return Ok(py_int(i.clone()));
                }
                return Err(PyError::type_error("__len__() should return an int"));
            }
            if let Some(native) = native {
                return builtin_len(&[native]);
            }
            Err(PyError::type_error(format!(
                "object of type '{}' has no len()",
                type_name
            )))
        }
        // A class object itself, via its metaclass's `__len__` (e.g.
        // `len(SomeEnum)` — see the matching GET_ITER/builtin_iter handling
        // for why this needs metatype_of rather than ordinary lookup).
        PyObject::Type { .. } => {
            let f = metatype_of(&args[0]).and_then(|mt| lookup_dunder_via_mro(&mt, "__len__"));
            let type_name = obj.type_name();
            drop(obj);
            if let Some(f) = f {
                let result = call_bound_method(f, args[0].clone(), vec![])?;
                let n = result.borrow();
                if let PyObject::Int(i) = &*n {
                    if i.sign() == Sign::Minus {
                        return Err(PyError::value_error("__len__() should return >= 0"));
                    }
                    return Ok(py_int(i.clone()));
                }
                return Err(PyError::type_error("__len__() should return an int"));
            }
            Err(PyError::type_error(format!(
                "object of type '{}' has no len()",
                type_name
            )))
        }
        _ => Err(PyError::type_error(format!(
            "object of type '{}' has no len()",
            obj.type_name()
        ))),
    }
}

/// Cheap, best-effort size hint for materializing an arbitrary iterable
/// into a `Vec` (used by `list()`/`tuple()`). Real CPython pre-sizes via
/// `PyObject_LengthHint` before iterating, so a source with an O(1) `len()`
/// (e.g. `range(huge)`) fails fast with a single allocation attempt instead
/// of growing the backing buffer one doubling at a time — which, for
/// something like `list(range(sys.maxsize // 2))`, can consume the
/// system's entire RAM over many reallocations before ever failing (each
/// individual `push()` succeeds right up until physical memory runs out).
/// Returns `None` (not an error) when the object has no usable `__len__` —
/// callers should just skip pre-reservation and fall back to incremental
/// growth, which is fine for ordinary bounded iterables/generators.
fn iterable_length_hint(obj: &PyObjectRef) -> Option<usize> {
    let len_obj = builtin_len(std::slice::from_ref(obj)).ok()?;
    let borrowed = len_obj.borrow();
    match &*borrowed {
        PyObject::Int(n) => n.to_usize(),
        _ => None,
    }
}

// real `range()` accepts anything implementing `__index__`, not just a
// literal `int` (`crate::object::subscript::to_index` already implements
// that same "native int, or call `__index__` via mro" protocol for
// slicing) — found via CPython's own `test_range.py`, which constructs
// `range()` bounds from custom `__index__`-only objects.
fn range_index_arg(obj: &PyObjectRef) -> PyResult<num_bigint::BigInt> {
    to_index(obj)
}

pub fn builtin_range(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let one = num_bigint::BigInt::from(1);
    let zero = num_bigint::BigInt::from(0);
    match args.len() {
        1 => {
            let stop = range_index_arg(&args[0])?;
            Ok(PyObjectRef::imm(PyObject::Range {
                start: zero,
                stop,
                step: one,
            }))
        }
        2 => {
            let a = range_index_arg(&args[0])?;
            let b = range_index_arg(&args[1])?;
            Ok(PyObjectRef::imm(PyObject::Range {
                start: a,
                stop: b,
                step: one,
            }))
        }
        3 => {
            let a = range_index_arg(&args[0])?;
            let b = range_index_arg(&args[1])?;
            let s = range_index_arg(&args[2])?;
            if s == zero {
                return Err(PyError::value_error("range() arg 3 must not be zero"));
            }
            Ok(PyObjectRef::imm(PyObject::Range {
                start: a,
                stop: b,
                step: s,
            }))
        }
        _ => {
            let msg = if args.is_empty() {
                format!("range expected at least 1 argument, got 0")
            } else {
                format!("range expected at most 3 arguments, got {}", args.len())
            };
            Err(PyError::type_error(msg))
        }
    }
}

/// Validate underscore placement in a numeric string: underscores must sit
/// BETWEEN two digits (leading/trailing/double/adjacent-to-dot are invalid).
pub(crate) fn validate_underscores(s: &str) -> PyResult<String> {
    // Hex literals allow underscores between hex digits; decimal floats only
    // between plain digits (an underscore next to 'e'/'.'/start/end is bad).
    let is_hex = s.starts_with("0x") || s.starts_with("0X");
    let chars: Vec<char> = s.chars().collect();
    for i in 0..chars.len() {
        if chars[i] == '_' {
            let prev_ok = i > 0
                && if is_hex {
                    chars[i - 1].is_ascii_hexdigit()
                } else {
                    chars[i - 1].is_ascii_digit()
                };
            let next_ok = i + 1 < chars.len()
                && if is_hex {
                    chars[i + 1].is_ascii_hexdigit()
                } else {
                    chars[i + 1].is_ascii_digit()
                };
            if !(prev_ok && next_ok) {
                return Err(PyError::value_error(format!("invalid decimal literal")));
            }
        }
    }
    Ok(s.to_string())
}

/// Python-style bytes repr (`b'...'`, escaping non-printables) — used in
/// `float()` conversion error messages, which quote the original bytes.
fn python_bytes_repr(b: &[u8]) -> String {
    let s: String = b
        .iter()
        .map(|&byte| match byte {
            b'\\' => "\\\\".to_string(),
            b'\'' => "\\'".to_string(),
            b'\n' => "\\n".to_string(),
            b'\t' => "\\t".to_string(),
            b'\r' => "\\r".to_string(),
            0x20..=0x7e => (byte as char).to_string(),
            _ => format!("\\x{:02x}", byte),
        })
        .collect();
    s
}

/// `float(int)`: an int too large to be represented as a double raises
/// OverflowError (2**2000 -> "int too large to convert to float"), not inf.
pub(crate) fn bigint_to_float(i: &BigInt) -> PyResult<f64> {
    match i.to_f64() {
        Some(f) if f.is_finite() => Ok(f),
        _ => Err(PyError::overflow_error("int too large to convert to float")),
    }
}

pub fn builtin_float(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_float(0.0));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(i) => Ok(py_float(bigint_to_float(i)?)),
        // `float(True)` == 1.0 / `float(False)` == 0.0 (bool is int's
        // subtype).
        PyObject::Bool(b) => Ok(py_float(if *b { 1.0 } else { 0.0 })),
        PyObject::Float(f) => Ok(py_float(*f)),
        PyObject::Str(s) => {
            let s: &str = s;
            let s_orig = s;
            let s = s.trim_matches(|c: char| c.is_whitespace());
            let normalized: String = s
                .chars()
                .map(|c| match c {
                    '\u{0660}'..='\u{0669}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x0660)).unwrap_or(c)
                    }
                    '\u{06F0}'..='\u{06F9}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x06F0)).unwrap_or(c)
                    }
                    '\u{0966}'..='\u{096F}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x0966)).unwrap_or(c)
                    }
                    _ => c,
                })
                .collect();
            let normalized: String = validate_underscores(&normalized)?
                .chars()
                .filter(|&c| c != '_')
                .collect();
            // CPython's error quotes the ORIGINAL string with str repr
            // (%r), so control characters are escaped ('\t \n', '123\x00').
            let f: f64 = normalized.parse().map_err(|_| {
                PyError::value_error(format!(
                    "could not convert string to float: '{}'",
                    crate::object::escape_string(s_orig)
                ))
            })?;
            Ok(py_float(f))
        }
        PyObject::Bytes(b) => {
            // Bytes are scanned as raw ASCII float syntax (CPython does not
            // require valid UTF-8): a NUL or non-ASCII byte fails the parse
            // and reports the bytes repr.
            if b.iter().any(|&x| x >= 0x80) {
                return Err(PyError::value_error(format!(
                    "could not convert string to float: b'{}'",
                    python_bytes_repr(b)
                )));
            }
            let s: String = b.iter().map(|&x| x as char).collect();
            let s = s.trim_matches(|c: char| c.is_whitespace());
            let normalized: String = s
                .chars()
                .map(|c| match c {
                    '\u{0660}'..='\u{0669}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x0660)).unwrap_or(c)
                    }
                    '\u{06F0}'..='\u{06F9}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x06F0)).unwrap_or(c)
                    }
                    '\u{0966}'..='\u{096F}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x0966)).unwrap_or(c)
                    }
                    _ => c,
                })
                .collect();
            let normalized: String = validate_underscores(&normalized)?
                .chars()
                .filter(|&c| c != '_')
                .collect();
            // CPython's error uses the bytes repr (%r) with the ORIGINAL
            // content: "could not convert string to float: b'  123 456  '".
            let f: f64 = normalized.parse().map_err(|_| {
                PyError::value_error(format!(
                    "could not convert string to float: b'{}'",
                    python_bytes_repr(b)
                ))
            })?;
            Ok(py_float(f))
        }
        PyObject::ByteArray(b) => {
            if b.iter().any(|&x| x >= 0x80) {
                return Err(PyError::value_error(format!(
                    "could not convert bytearray to float: bytearray(b'{}')",
                    python_bytes_repr(b)
                )));
            }
            let s: String = b.iter().map(|&x| x as char).collect();
            let s = s.trim_matches(|c: char| c.is_whitespace());
            let normalized: String = s
                .chars()
                .map(|c| match c {
                    '\u{0660}'..='\u{0669}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x0660)).unwrap_or(c)
                    }
                    '\u{06F0}'..='\u{06F9}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x06F0)).unwrap_or(c)
                    }
                    '\u{0966}'..='\u{096F}' => {
                        char::from_u32('0' as u32 + (c as u32 - 0x0966)).unwrap_or(c)
                    }
                    _ => c,
                })
                .collect();
            let normalized: String = validate_underscores(&normalized)?
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let f: f64 = normalized.parse().map_err(|_| {
                PyError::value_error(format!(
                    "could not convert bytearray to float: bytearray(b'{}')",
                    python_bytes_repr(b)
                ))
            })?;
            Ok(py_float(f))
        }
        PyObject::MemoryView { .. } => {
            // float(memoryview) parses the memoryview's contiguous bytes as
            // a float string (float(memoryview(b'12.3')[1:4]) == 2.3).
            let bytes = crate::object::mv_tobytes(&args[0].clone())?;
            return builtin_float(&[PyObjectRef::imm(PyObject::Bytes(bytes))]);
        }
        PyObject::Array(arr) => {
            // Byte-typecoded arrays parse their contiguous buffer as a float
            // string (float(array('B', b' 3.14  ')) == 3.14).
            if matches!(arr.typecode, 'B' | 'b' | 'u') {
                let bytes: Vec<u8> = arr.data.iter().map(|&v| v as u8).collect();
                return builtin_float(&[PyObjectRef::imm(PyObject::Bytes(bytes))]);
            }
            return Err(PyError::value_error(format!(
                "could not convert string to float: array({:?}, ...)",
                arr.typecode
            )));
        }
        PyObject::Instance { typ, .. } => {
            let typ = typ.clone();
            let arg = args[0].clone();
            let type_name = get_type_name_for_instance(&typ);
            drop(obj);
            // A custom __float__ wins over the native base's string/backing
            // handling (float(FooStr('8')) calls FooStr.__float__, not the
            // string parser; float(OtherFloatSubclass(3.14)) falls through
            // to the float backing below since it defines no __float__).
            if let Some(f) = lookup_dunder_via_mro(&typ, "__float__") {
                let result = call_bound_method(f, arg.clone(), vec![])?;
                // Exact float result: used directly, no warning.
                if matches!(&result, PyObjectRef::SmallFloat(_))
                    || matches!(&*result.borrow(), PyObject::Float(_))
                {
                    return Ok(result);
                }
                // A float SUBCLASS result is deprecated but still usable;
                // any other type is an error (Foo4.__float__ returning int).
                let is_float_subclass = {
                    let rt = result.borrow();
                    matches!(&*rt, PyObject::Instance { typ: rtyp, .. }
                        if native_base_of_type(rtyp).as_deref() == Some("float"))
                };
                if is_float_subclass {
                    let v = native_backing_of(&result)
                        .and_then(|b| b.as_f64())
                        .unwrap_or(f64::NAN);
                    crate::modules::warnings_emit(
                        &format!("{}.__float__ returned non-float (type {}).  The ability to return an instance of a strict subclass of float is deprecated, and may be removed in a future version of Python.", type_name, result.borrow().type_name()),
                        "DeprecationWarning",
                    );
                    return Ok(py_float(v));
                }
                return Err(PyError::type_error(format!(
                    "{}.__float__ returned non-float (type {})",
                    type_name,
                    result.borrow().type_name()
                )));
            }
            // Native-base routing for subclasses that define no __float__:
            // str/bytes/bytearray parse their content; float subclasses are
            // already their value.
            let kind = native_base_of_type(&typ);
            if let Some(kind) = kind {
                if kind == "str" || kind == "bytes" || kind == "bytearray" {
                    let backing = native_backing_of(&arg).unwrap_or_else(|| arg.clone());
                    return builtin_float(&[backing]);
                }
                if kind == "float" {
                    if let Some(backing) = native_backing_of(&arg) {
                        return Ok(backing);
                    }
                }
            }
            // __index__ fallback (CPython's PyFloat_AsDouble).
            if let Some(f) = lookup_dunder_via_mro(&typ, "__index__") {
                let result = call_bound_method(f, arg.clone(), vec![])?;
                let v = result.borrow();
                if let PyObject::Int(i) = &*v {
                    return Ok(py_float(bigint_to_float(i)?));
                }
                return Err(PyError::type_error("__index__ returned non-int"));
            }
            Err(PyError::type_error(format!(
                "float() argument must be a string or number, not '{}'",
                type_name
            )))
        }
        _ => Err(PyError::type_error(format!(
            "float() argument must be a string or number, not '{}'",
            obj.type_name()
        ))),
    }
}

/// `float.fromhex(s)` — a genuine class-level-only method (called unbound,
/// `float.fromhex("0x1.8p3")`, never as `x.fromhex()` on a float instance),
/// extracted out of what used to be a `bf_name == "float" && name ==
/// "fromhex"` inline closure in `get_attribute_impl` (`attrs.rs`) so it can
/// live in `float`'s own type dict now that `float` is a real `Type` (see
/// `NATIVE_VALUE_CTOR_KEY`'s doc comment) — that string-name dispatch never
/// fires for a real `Type` object, only for the old bare `BuiltinFunction`
/// shape.
/// Builds a float-classmethod result (`fromhex`) for the calling type: a
/// plain float for `float` itself, otherwise a REAL subclass construction so
/// a custom `__new__`/`__init__` runs (`F.fromhex(...)` where F's __new__
/// adds 1 yields value+1 as an F — test_float's HexFloatTestCase.
pub(crate) fn float_subclass_result(cls: &PyObjectRef, value: f64) -> PyResult<PyObjectRef> {
    let is_plain = matches!(&*cls.borrow(), PyObject::Type { name, .. } if name == "float");
    if is_plain {
        return Ok(py_float(value));
    }
    // A user-defined __new__ (Python Function) runs the real construction.
    if let Some(new_fn) = lookup_dunder_via_mro(cls, "__new__") {
        if matches!(&*new_fn.borrow(), PyObject::Function(_)) {
            return call_bound_method(new_fn, cls.clone(), vec![py_float(value)]);
        }
    }
    // Native __new__: build the instance with the value as its backing, then
    // run a custom Python __init__ if present (float's own native init is
    // skipped — the backing is already populated).
    let mut dict = AttrMap::new();
    dict.insert(NATIVE_BACKING_KEY.to_string(), py_float(value));
    let instance = PyObjectRef::new(PyObject::Instance {
        typ: cls.clone(),
        dict,
    });
    if let Some(init_fn) = lookup_dunder_via_mro(cls, "__init__") {
        let is_native = matches!(
            &*init_fn.borrow(),
            PyObject::BuiltinFunction { .. } | PyObject::Closure(_)
        );
        if !is_native {
            call_bound_method(init_fn, instance.clone(), vec![py_float(value)])?;
        }
    }
    Ok(instance)
}

pub(crate) fn float_fromhex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Bound as a classmethod: args[0] is the calling type, args[1] the string.
    if args.len() < 2 {
        return Err(PyError::type_error(
            "float.fromhex() requires exactly 1 argument",
        ));
    }
    let cls = &args[0];
    let s = args[1].str();
    let s = s.trim();
    // At most ONE leading sign ('++0x1.0p-0', '-+0x1.0p0' are invalid).
    {
        let mut c = s.chars();
        let first = c.next();
        let second = c.next();
        if matches!(first, Some('+') | Some('-')) && matches!(second, Some('+') | Some('-')) {
            return Err(PyError::value_error(
                "invalid hexadecimal floating-point literal",
            ));
        }
    }
    let lower = s.to_lowercase();
    // nan spellings (with optional sign, case-insensitive) — all produce the
    // same nan.
    if lower == "nan" || lower == "+nan" || lower == "-nan" {
        return float_subclass_result(cls, f64::NAN);
    }
    if lower == "inf"
        || lower == "+inf"
        || lower == "-inf"
        || lower == "infinity"
        || lower == "+infinity"
        || lower == "-infinity"
    {
        let sign = if lower.starts_with('-') { -1.0 } else { 1.0 };
        return float_subclass_result(cls, sign * f64::INFINITY);
    }
    let s = s.strip_prefix("+").unwrap_or(s);
    let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
    let s = s
        .strip_prefix('-')
        .unwrap_or(s.strip_prefix('+').unwrap_or(s));
    // A second sign ('++0x1.0p-0', '-+0x1.0p0') is invalid.
    if s.starts_with('+') || s.starts_with('-') {
        return Err(PyError::value_error(
            "invalid hexadecimal floating-point literal",
        ));
    }
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if s.is_empty() {
        return Err(PyError::value_error(
            "invalid hexadecimal floating-point literal",
        ));
    }
    // Split off the 'p' exponent FIRST — a mantissa without a dot
    // ('0x1p-1022') otherwise loses its exponent to the dot-split below.
    let (mantissa, exp_part) = s
        .split_once('p')
        .or_else(|| s.split_once('P'))
        .unwrap_or((s, ""));
    // A 'p'/'P' present but with no exponent digits after it ('0x0p') is
    // invalid, not an implicit exponent of 0.
    if exp_part.is_empty() && (s.contains('p') || s.contains('P')) {
        return Err(PyError::value_error(
            "invalid hexadecimal floating-point literal",
        ));
    }
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    // No hex digits before OR after the point ('0x.p0', '0x.') is invalid.
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(PyError::value_error(
            "invalid hexadecimal floating-point literal",
        ));
    }
    // Parse the mantissa EXACTLY: d = int * 16**frac_len + frac (a 17-digit
    // hex int overflows i64 — 0x10000000000000000 must be 2**64, not
    // silently 0); the fractional point is folded into the binary exponent
    // as 4 bits per fractional hex digit.
    let parse_hex = |t: &str| -> PyResult<BigInt> {
        if t.is_empty() {
            Ok(BigInt::from(0))
        } else {
            BigInt::parse_bytes(t.as_bytes(), 16).ok_or_else(|| {
                PyError::value_error(format!(
                    "invalid hexadecimal floating-point literal: '{}'",
                    args[0].str()
                ))
            })
        }
    };
    let int_d = parse_hex(int_part)?;
    let frac_d = parse_hex(frac_part)?;
    let frac_len = frac_part.len() as u64;
    let d = int_d * (BigInt::from(16u32).pow(frac_len as u32)) + frac_d;
    let exp: BigInt = if !exp_part.is_empty() {
        exp_part.parse().map_err(|_| {
            PyError::value_error(format!("invalid hex float exponent: {}", exp_part))
        })?
    } else {
        BigInt::from(0)
    };
    // value = d * 2**(exp - 4*frac_len); round-half-even to the nearest
    // f64 EXACTLY (d.to_f64() before scaling would lose the low bits that
    // decide subnormal rounding — 0x1.00000000000000001p-1075 rounds to
    // TINY, not 0).
    let k = {
        let adj = exp - BigInt::from(4) * BigInt::from(frac_len as i64);
        match adj.to_i64() {
            Some(e) => e,
            None => {
                if adj.sign() == num_bigint::Sign::Minus {
                    return Ok(py_float(sign * 0.0));
                }
                return Err(PyError::overflow_error(
                    "hexadecimal value too large to represent as a float",
                ));
            }
        }
    };
    let result = if d == BigInt::zero() {
        sign * 0.0
    } else {
        let t = d.bits() as i64; // bit length of d
        let e = t - 1 + k; // exponent of the most significant bit
        let r = if e > 1023 {
            f64::INFINITY
        } else if e >= -1022 {
            // normal range: doubles spaced 2**(e-52); n = round(d / 2**(t-53))
            let n = if t <= 53 {
                d.clone() << ((53 - t) as u64)
            } else {
                round_half_even_rat(&d, &(BigInt::from(1i64) << ((t - 53) as u64)))
            };
            if n >= (BigInt::from(1i64) << 53) {
                // rounded up into the next binade
                if e + 1 > 1023 {
                    f64::INFINITY
                } else {
                    ldexp_f64(1.0, (e + 1) as i32)
                }
            } else {
                ldexp_f64(n.to_f64().unwrap_or(0.0), (e - 52) as i32)
            }
        } else if e >= -1075 {
            // subnormal: doubles spaced 2**-1074; n = round(d * 2**1074 / 2**m)
            let m = -k;
            let n = if m >= 1074 {
                round_half_even_rat(&d, &(BigInt::from(1i64) << ((m - 1074) as u64)))
            } else {
                d << ((1074 - m) as u64)
            };
            ldexp_f64(n.to_f64().unwrap_or(0.0), -1074)
        } else {
            0.0
        };
        sign * r
    };
    // A finite hex literal that computes to infinity is an overflow
    // (0x1p1024, 0X1.fffffffffffff8p1023), not a real inf spelling (which
    // returned above).
    if result.is_infinite() {
        return Err(PyError::overflow_error(
            "hexadecimal value too large to represent as a float",
        ));
    }
    float_subclass_result(cls, result)
}

/// `x * 2**exp` without intermediate overflow/underflow — a naive
/// `x * 2.0f64.powi(exp)` overflows to inf for exp >= 1024 even when the
/// true value (e.g. `0x.fffffffffffff8p+1024` == the max normal) is
/// finite. Scales in 512-bit chunks, staying within f64 range.
pub(crate) fn ldexp_f64(x: f64, exp: i32) -> f64 {
    let mut x = x;
    let mut e = exp;
    let big = 2.0f64.powf(512.0);
    let small = 2.0f64.powf(-512.0);
    while e > 1023 {
        x *= big;
        e -= 512;
        if !x.is_finite() && x > 0.0 {
            // Overflow is genuine (value really is inf).
            return x;
        }
    }
    while e < -1022 {
        x *= small;
        e += 512;
        if x == 0.0 {
            return x;
        }
    }
    x * 2.0f64.powi(e)
}

/// `float.hex(x)` — the unbound, explicit-argument class-level form (`float.
/// hex(3.5)`, as opposed to `x.hex()` on a float instance, which goes
/// through a wholly separate `PyObject::Float(_)` instance arm elsewhere in
/// `attrs.rs`, unaffected by this). Same extraction rationale as
/// `float_fromhex` above.
pub(crate) fn float_class_hex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("hex() takes exactly 1 argument"));
    }
    let obj = args[0].borrow();
    if let PyObject::Float(v) = &*obj {
        let bits = v.to_bits();
        let sign = if (bits >> 63) != 0 { "-" } else { "" };
        let biased_exp = ((bits >> 52) & 0x7ff) as i64;
        let mantissa = bits & 0x000f_ffff_ffff_ffff;
        if biased_exp == 0x7ff {
            if mantissa == 0 {
                Ok(py_str(&format!("{}inf", sign)))
            } else {
                Ok(py_str(&format!("{}nan", sign)))
            }
        } else if *v == 0.0 {
            Ok(py_str(&format!("{}0x0.0p+0", sign)))
        } else {
            let hex_mantissa = format!("{:013x}", mantissa);
            if biased_exp == 0 {
                // Subnormal: same convention as the instance `x.hex()` —
                // raw 52-bit mantissa after a 0x0. prefix at fixed exponent
                // -1022 (NOT the 0x1.XXXXp-1023 normal form, which would
                // misrepresent the exact value).
                Ok(py_str(&format!("{}0x0.{}p-1022", sign, hex_mantissa)))
            } else {
                let exp = biased_exp - 1023;
                // Keep ALL 13 frac hex digits (CPython never trims them).
                Ok(py_str(&format!("{}0x1.{}p{:+}", sign, hex_mantissa, exp)))
            }
        }
    } else {
        Err(PyError::type_error("hex() argument must be float"))
    }
}

/// Parses a real CPython-style complex literal string (`complex("1+2j")`,
/// `complex("-3-4j")`, `complex("2j")`, `complex("(1+2j)")`) — finds the
/// LAST top-level `+`/`-` before the trailing `j`/`J` (skipping one right
/// after `e`/`E`, which is an exponent sign, not the real/imag separator).
fn parse_complex_str(s: &str) -> PyResult<(f64, f64)> {
    let malformed = || PyError::value_error(format!("complex() arg is a malformed string"));
    let s = s.trim();
    let inner = s
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(s)
        .trim();
    if inner.is_empty() {
        return Err(malformed());
    }
    if let Some(stripped) = inner.strip_suffix(['j', 'J']) {
        let bytes = stripped.as_bytes();
        let mut split_idx = None;
        for i in (1..bytes.len()).rev() {
            let c = bytes[i] as char;
            if c == '+' || c == '-' {
                let prev = bytes[i - 1] as char;
                if prev != 'e' && prev != 'E' {
                    split_idx = Some(i);
                    break;
                }
            }
        }
        match split_idx {
            Some(idx) => {
                let real_str = &stripped[..idx];
                let imag_str = &stripped[idx..];
                let re: f64 = real_str.parse().map_err(|_| malformed())?;
                let im: f64 = match imag_str {
                    "+" => 1.0,
                    "-" => -1.0,
                    _ => imag_str.parse().map_err(|_| malformed())?,
                };
                Ok((re, im))
            }
            None => {
                let im: f64 = match stripped {
                    "" | "+" => 1.0,
                    "-" => -1.0,
                    _ => stripped.parse().map_err(|_| malformed())?,
                };
                Ok((0.0, im))
            }
        }
    } else {
        let re: f64 = inner.parse().map_err(|_| malformed())?;
        Ok((re, 0.0))
    }
}

pub fn builtin_complex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(PyObjectRef::imm(PyObject::Complex(0.0, 0.0)));
    }
    let (re, im) = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Complex(re, im) => (*re, *im),
            PyObject::Int(i) => (i.to_f64().unwrap_or(0.0), 0.0),
            PyObject::Float(f) => (*f, 0.0),
            PyObject::Bool(b) => (if *b { 1.0 } else { 0.0 }, 0.0),
            PyObject::Str(s) => {
                if args.len() > 1 {
                    return Err(PyError::type_error(
                        "complex() can't take second arg if first is a string",
                    ));
                }
                parse_complex_str(s)?
            }
            // Custom `__complex__` was never consulted at all — same class
            // of gap just fixed for `divmod()`/`__divmod__` above. Real
            // trigger: `numbers.Complex`'s own mixin `__complex__`
            // (`Lib/numbers.py`, implemented via `self.real`/`self.imag`),
            // exercised directly by `test_abstract_numbers.py::test_real`
            // (`complex(MyReal(1))`).
            PyObject::Instance { typ, .. } => match lookup_dunder_via_mro(typ, "__complex__") {
                Some(f) => {
                    let f = f.clone();
                    let self_obj = args[0].clone();
                    drop(obj);
                    let result = call_bound_method(f, self_obj, vec![])?;
                    let result_borrow = result.borrow();
                    match &*result_borrow {
                        PyObject::Complex(re, im) => (*re, *im),
                        _ => return Err(PyError::type_error("__complex__ returned non-complex")),
                    }
                }
                None => {
                    return Err(PyError::type_error(format!(
                        "complex() argument must be a string or a number, not '{}'",
                        get_type_name_for_instance(typ)
                    )))
                }
            },
            _ => {
                return Err(PyError::type_error(format!(
                    "complex() argument must be a string or a number, not '{}'",
                    obj.type_name()
                )))
            }
        }
    };
    if args.len() > 1 {
        let imag_extra: f64 = {
            let obj = args[1].borrow();
            match &*obj {
                PyObject::Int(i) => i.to_f64().unwrap_or(0.0),
                PyObject::Float(f) => *f,
                PyObject::Bool(b) => {
                    if *b {
                        1.0
                    } else {
                        0.0
                    }
                }
                PyObject::Instance { typ, .. } => match lookup_dunder_via_mro(typ, "__complex__") {
                    Some(f) => {
                        let f = f.clone();
                        let self_obj = args[1].clone();
                        drop(obj);
                        let result = call_bound_method(f, self_obj, vec![])?;
                        let result_borrow = result.borrow();
                        match &*result_borrow {
                            PyObject::Complex(re, im) => {
                                if *im != 0.0 {
                                    return Err(PyError::type_error("complex() can't take second arg if first is a complex number with a nonzero imaginary part"));
                                }
                                *re
                            }
                            _ => {
                                return Err(PyError::type_error("__complex__ returned non-complex"))
                            }
                        }
                    }
                    None => {
                        return Err(PyError::type_error(format!(
                            "complex() second argument must be a number, not '{}'",
                            get_type_name_for_instance(typ)
                        )))
                    }
                },
                _ => {
                    return Err(PyError::type_error(format!(
                        "complex() second argument must be a number, not '{}'",
                        obj.type_name()
                    )))
                }
            }
        };
        return Ok(PyObjectRef::imm(PyObject::Complex(re, im + imag_extra)));
    }
    Ok(PyObjectRef::imm(PyObject::Complex(re, im)))
}

/// Does `typ`'s mro/bases include a builtin exception class (`Exception`,
/// `OSError`, ...)? Those are `PyObject::BuiltinFunction` (the exception's
/// own constructor), never a real `PyObject::Type` — invisible to
/// `lookup_dunder_via_mro`'s dict-based walk, so a plain `class MyError
/// (Exception): pass` (no custom `__str__`/`__repr__` override) fell
/// through to the fully-generic `<MyError object>` instead of real
/// `BaseException.__str__`/`__repr__`'s args-based formatting (`str(exc)`
/// for a single-arg exception should be that arg's `str()`, not a useless
/// placeholder — this broke essentially every custom exception hierarchy's
/// error messages).
/// Read a class's OWN `_abc_registry` (set by `.register()`, the generic
/// `PyObject::Type` fallback method — see its own doc comment) — NEVER via
/// `get_attribute`/`lookup_dunder_via_mro`-style MRO walking. A virtual
/// registration against one ABC (`Complex.register(complex)`) must NOT be
/// visible when checking a MORE SPECIFIC descendant ABC (`Integral`) just
/// because `Integral` inherits from `Complex` — registration doesn't flow
/// "downward" in specificity that way. (`.register()` itself has the exact
/// same "must not read via MRO" requirement, for a different reason — see
/// its own inline comment.)
fn own_abc_registry(typ: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObject::Type { dict, .. } = &*typ.borrow() {
        dict.get_str("_abc_registry")
            .and_then(|r| {
                if let PyObject::FrozenSet(items) = &*r.borrow() {
                    Some(items.to_vec())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Is anything matching `matcher` registered against `base` ITSELF, or
/// against any REAL (regular inheritance, non-virtual) subclass of `base`?
/// Needed because registration should propagate to less-specific ABCs in
/// the same real hierarchy: `numbers.py`'s `Integral.register(int)` must
/// also make `issubclass(int, Real)`/`issubclass(int, Complex)` true,
/// since `Integral` is a real subclass of `Real`/`Complex` (NOT because
/// `int` itself was registered against them directly — it wasn't). Walks
/// `direct_subclasses_of` recursively; cheap in practice since real ABC
/// hierarchies (`numbers`, `collections.abc`) are shallow.
fn abc_registry_matches_in_subtree(
    base: &PyObjectRef,
    matcher: &dyn Fn(&PyObjectRef) -> bool,
) -> bool {
    if own_abc_registry(base).iter().any(|r| matcher(r)) {
        return true;
    }
    direct_subclasses_of(base)
        .iter()
        .any(|sub| abc_registry_matches_in_subtree(sub, matcher))
}

fn is_exception_type(typ: &PyObjectRef) -> bool {
    if let PyObject::Type { mro, bases, .. } = &*typ.borrow() {
        let entries = if mro.is_empty() { bases } else { mro };
        entries.iter().any(|b| {
            if let PyObject::BuiltinFunction { name, .. } = &*b.borrow() {
                is_builtin_exception_class_name(name)
            } else {
                false
            }
        })
    } else {
        false
    }
}

/// Real `BaseException.__str__`: no args -> `""`, one arg -> that arg's
/// `str()`, multiple args -> `str()` of the whole args tuple.
fn exception_instance_str(instance: &PyObjectRef) -> String {
    let args = if let PyObject::Instance { dict, .. } = &*instance.borrow() {
        dict.get("args").cloned()
    } else {
        None
    };
    match args.map(|a| a.borrow().clone()) {
        Some(PyObject::Tuple(items)) if items.len() == 1 => items[0].str(),
        Some(PyObject::Tuple(items)) if !items.is_empty() => py_tuple(items).str(),
        _ => String::new(),
    }
}

/// Real `BaseException.__repr__`: `ClassName(repr(arg1), repr(arg2), ...)`.
fn exception_instance_repr(instance: &PyObjectRef, class_name: &str) -> String {
    let args = if let PyObject::Instance { dict, .. } = &*instance.borrow() {
        dict.get("args").cloned()
    } else {
        None
    };
    let args_str = match args.map(|a| a.borrow().clone()) {
        Some(PyObject::Tuple(items)) => items
            .iter()
            .map(|a| a.repr())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    format!("{}({})", class_name, args_str)
}

pub fn str_maketrans_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `str.maketrans(x[, y[, z]])` — builds a translation table (a dict of
    // {char: replacement-or-None}) consumed by `str.translate`. Real
    // CPython's single-argument form takes a mapping whose keys are
    // length-1 strings; the 2/3-argument form maps first-string chars to
    // second-string chars (equal length required) with an optional third
    // string of chars to DELETE. Returns a plain `PyDict`.
    let mut table = PyDict::new();
    match args.len() {
        1 => {
            let mapping = &args[0];
            let items: Vec<(PyObjectRef, PyObjectRef)> = match &*mapping.borrow() {
                PyObject::Dict(d) => d.items(),
                _ => {
                    return Err(PyError::type_error(
                        "str.maketrans() argument 1 must be a mapping, not str",
                    ))
                }
            };
            for (k, v) in items {
                if k.str().chars().count() != 1 {
                    return Err(PyError::value_error(
                        "string keys in translate table must be of length 1",
                    ));
                }
                table.set(k, v)?;
            }
        }
        2 | 3 => {
            let x = args[0].str();
            let y = args[1].str();
            let x_chars: Vec<char> = x.chars().collect();
            let y_chars: Vec<char> = y.chars().collect();
            if x_chars.len() != y_chars.len() {
                return Err(PyError::value_error(
                    "the first two maketrans arguments must have equal length",
                ));
            }
            for (a, b) in x_chars.iter().zip(y_chars.iter()) {
                table.set(py_str(&a.to_string()), py_str(&b.to_string()))?;
            }
            if args.len() == 3 {
                for c in args[2].str().chars() {
                    table.set(py_str(&c.to_string()), py_none())?;
                }
            }
        }
        _ => {
            return Err(PyError::type_error(
                "str.maketrans() takes 1 or 3 arguments (2 given)",
            ))
        }
    }
    Ok(PyObjectRef::new(PyObject::Dict(Box::new(table))))
}

pub fn bytes_maketrans_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `bytes.maketrans(frm, to)` — returns a 256-byte translation table.
    if args.len() < 2 {
        return Err(PyError::type_error(
            "bytes.maketrans() takes exactly 2 arguments",
        ));
    }
    let frm: Vec<u8> = match &*args[0].borrow() {
        PyObject::Bytes(b) => b.clone(),
        _ => {
            return Err(PyError::type_error(
                "bytes.maketrans() argument 1 must be bytes",
            ))
        }
    };
    let to: Vec<u8> = match &*args[1].borrow() {
        PyObject::Bytes(b) => b.clone(),
        _ => {
            return Err(PyError::type_error(
                "bytes.maketrans() argument 2 must be bytes",
            ))
        }
    };
    if frm.len() != to.len() {
        return Err(PyError::value_error(
            "maketrans arguments must have same length",
        ));
    }
    let mut result: Vec<u8> = (0u16..=255).map(|i| i as u8).collect();
    for (i, &f) in frm.iter().enumerate() {
        result[f as usize] = to[i];
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
}

pub fn builtin_str(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(py_str(""))
    } else if args.len() >= 2 {
        // `str(bytes, encoding[, errors])` — decode using the given codec
        // (test_charmapcodec's test_constructorx: `str(b'abc', 'testcodec')`).
        if matches!(
            &*args[0].borrow(),
            PyObject::Bytes(_) | PyObject::ByteArray(_)
        ) {
            // The decode BuiltinMethod is stored with self_obj=None (an
            // unbound descriptor); call its raw `func` with [self, ...]
            // directly rather than call_bound_method (which would prepend
            // the stored None as an extra first arg -> "decode on non-bytes").
            let decode = args[0].borrow().get_attribute("decode")?;
            if let PyObject::BuiltinMethod { func: f, .. } = &*decode.borrow() {
                let mut call_args = vec![args[0].clone()];
                call_args.push(args[1].clone());
                if args.len() >= 3 {
                    call_args.push(args[2].clone());
                }
                return f(&call_args);
            }
            let mut call_args = vec![args[1].clone()];
            if args.len() >= 3 {
                call_args.push(args[2].clone());
            }
            return call_bound_method(decode, args[0].clone(), call_args);
        }
        // str(obj, encoding) on a non-bytes object is a TypeError in CPython.
        return Err(PyError::type_error(
            "decoding to str: need a bytes-like object, found type object",
        ));
    } else {
        // WeakProxy: str(proxy) forwards to str(target) if alive, else ReferenceError.
        if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
            if let Some(rc) = target.upgrade() {
                let target_ref = PyObjectRef::Mut(rc);
                return builtin_str(&[target_ref]);
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }
        let f = {
            let obj_borrowed = args[0].borrow();
            if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                lookup_dunder_via_mro(typ, "__str__")
            } else {
                None
            }
        };
        if let Some(f) = f {
            return call_bound_method(f, args[0].clone(), vec![]);
        }
        let is_exc = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
            is_exception_type(typ)
        } else {
            false
        };
        if is_exc {
            return Ok(py_str(&exception_instance_str(&args[0])));
        }
        // int->str digit limit (str(10**10000) raises ValueError), also for
        // int subclass instances (whose native backing is the int).
        if let Some(i) = int_value_or_backing(&args[0]) {
            check_int_to_str_limit(&i)?;
        }
        Ok(py_str(&args[0].str()))
    }
}

/// CPython's int->str digit limit: str()/repr()/format() of an int with more
/// decimal digits than the configured limit raises ValueError (guard against
/// a DoS from formatting astronomical integers).
pub(crate) fn check_int_to_str_limit(bi: &BigInt) -> PyResult<()> {
    let limit = INT_MAX_STR_DIGITS.with(|d| d.get());
    if limit <= 0 {
        return Ok(());
    }
    // Estimate the decimal digit count from the bit length (log10(2) ≈
    // 0.30103) so a truly enormous int is rejected without converting it.
    let est = (bi.bits() as f64 * 0.3010299956639812).ceil() as u64;
    if est > limit as u64 {
        return Err(PyError::value_error(format!(
            "Exceeds the limit ({}) digits for integer string conversion; use sys.set_int_max_str_digits()", limit
        )));
    }
    // Borderline: convert and count exactly.
    let digits = bi.to_string().trim_start_matches('-').len() as u64;
    if digits > limit as u64 {
        return Err(PyError::value_error(format!(
            "Exceeds the limit ({}) digits for integer string conversion; use sys.set_int_max_str_digits()", limit
        )));
    }
    Ok(())
}

/// The BigInt value of an `int` (or a transparent int-subclass instance via
/// its native backing), if any.
pub(crate) fn int_value_or_backing(obj: &PyObjectRef) -> Option<BigInt> {
    match &*obj.borrow() {
        PyObject::Int(i) => Some(i.clone()),
        PyObject::Instance { .. } => {
            let native = native_backing_of(obj)?;
            let nb = native.borrow();
            match &*nb {
                PyObject::Int(i) => Some(i.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn builtin_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("repr() takes exactly one argument"));
    }
    let f = {
        let obj_borrowed = args[0].borrow();
        match &*obj_borrowed {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__repr__"),
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![]);
    }
    let class_name = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
        if is_exception_type(typ) {
            Some(typ.borrow().type_name().to_string())
        } else {
            None
        }
    } else {
        None
    };
    if let Some(class_name) = class_name {
        return Ok(py_str(&exception_instance_repr(&args[0], &class_name)));
    }
    if let Some(i) = int_value_or_backing(&args[0]) {
        check_int_to_str_limit(&i)?;
    }
    Ok(py_str(&args[0].repr()))
}

pub fn builtin_bool(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `bool(x=10)` -> TypeError (kwargs arrive as a trailing dict).
    if let Some(last) = args.last() {
        if matches!(&*last.borrow(), PyObject::Dict(_)) {
            return Err(PyError::type_error("bool() takes no keyword arguments"));
        }
    }
    if args.len() > 1 {
        return Err(PyError::type_error("bool() takes at most 1 argument"));
    }
    if args.is_empty() {
        return Ok(py_bool(false));
    }
    let typ_opt = {
        let obj = args[0].borrow();
        if let PyObject::Instance { typ, .. } = &*obj {
            let has_bool = lookup_dunder_via_mro(typ, "__bool__");
            let has_len = lookup_dunder_via_mro(typ, "__len__");
            // Distinguish "no __bool__/__len__ at all" from "the attribute
            // exists but is None" — the latter (class A: __bool__ = None)
            // must STILL raise TypeError ('A' cannot be interpreted as a
            // boolean), not silently fall back to truthiness. Real CPython
            // reserves a slot when __bool__/__len__ is set to None.
            let has_bool_slot = has_bool.is_some();
            let has_len_slot = has_len.is_some();
            if has_bool_slot || has_len_slot {
                Some(typ.clone())
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(typ) = typ_opt {
        // Unlike the infallible `.truthy()` (used for implicit if/while/and/or
        // truth-testing, which must never hang even on a malformed
        // `__bool__`), the explicit `bool()` builtin CAN and must raise the
        // real CPython error when `__bool__` doesn't return an actual `bool`
        // (e.g. `def __bool__(self): return self`) — confirmed via CPython's
        // own `test_bool.test_convert_to_bool`.
        if let Some(f) = lookup_dunder_via_mro(&typ, "__bool__") {
            // `__bool__ = None` (a broken slot) must raise "'<Type>' cannot
            // be interpreted as a boolean" — real CPython's exact error for
            // test_blocked's `class A: __bool__ = None`.
            if matches!(&*f.borrow(), PyObject::None) {
                return Err(PyError::type_error(format!(
                    "'{}' cannot be interpreted as a boolean",
                    typ.borrow().type_name()
                )));
            }
            let result = call_bound_method(f, args[0].clone(), vec![])?;
            return match result {
                PyObjectRef::SmallBool(b) => Ok(py_bool(b)),
                other => Err(PyError::type_error(format!(
                    "__bool__ should return bool, returned {}",
                    other.borrow().type_name()
                ))),
            };
        }
        if lookup_dunder_via_mro(&typ, "__len__").is_some() {
            // Delegate to `builtin_len` itself rather than re-deriving the
            // same validation here — CPython's own `test_bool.test_sane_len`
            // asserts `bool()`'s and `len()`'s error messages for the same
            // bad `__len__` return value are byte-for-byte IDENTICAL (real
            // CPython's `bool()` calls the same `PyObject_Size` under the
            // hood); sharing this code is what guarantees that instead of
            // two hand-written messages silently drifting apart.
            let n = builtin_len(&[args[0].clone()])?;
            return Ok(py_bool(n.as_i64().unwrap_or(0) != 0));
        }
    }
    Ok(py_bool(args[0].truthy()))
}

pub fn builtin_list(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `list(iterable, extra)` is always wrong; keyword rejection happens in
    // call_function (where the real keyword list is visible), so a POSITIONAL
    // dict stays valid (`list({'a': 1})` -> `['a']`).
    if args.len() > 1 {
        return Err(PyError::type_error(format!(
            "list() takes at most 1 argument ({} given)",
            args.len()
        )));
    }
    if args.is_empty() {
        Ok(py_list(Vec::new()))
    } else {
        // Convert iterable to list
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => Ok(py_list(v.clone())),
            PyObject::Tuple(v) => Ok(py_list(v.clone())),
            PyObject::Str(s) => {
                let items: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                Ok(py_list(items))
            }
            PyObject::Set(s) => Ok(py_list(s.to_vec())),
            _ => {
                drop(obj);
                // Try general iteration protocol via iter() + next() — any
                // error from `builtin_iter` must propagate AS-IS, not get
                // replaced with a generic "cannot convert" TypeError: an
                // object WITH `__iter__` whose call raises some OTHER
                // exception (real trigger: CPython's own `list_tests.py`'s
                // `test_constructor_exception_handling`, `class F: def
                // __iter__(self): raise KeyboardInterrupt`) had that
                // exception silently swallowed and replaced. `builtin_iter`
                // already produces the correct, more accurate message for
                // the genuinely-not-iterable case too (`'int' object is not
                // iterable`, matching real CPython exactly) — the previous
                // generic message here was actually a REGRESSION from that.
                let it = builtin_iter(&[args[0].clone()])?;
                let mut collected = Vec::new();
                if let Some(hint) = iterable_length_hint(&args[0]) {
                    if collected.try_reserve_exact(hint).is_err() {
                        return Err(PyError::memory_error("could not allocate list"));
                    }
                }
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(val) => collected.push(val),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(py_list(collected))
            }
        }
    }
}

pub fn builtin_deque(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `deque(iterable=None, maxlen=None)` — `maxlen` may come positionally
    // (`deque('abc', 3)`) or as a keyword (`deque('abc', maxlen=3)`); the
    // VM packs keywords into a trailing `PyObject::Dict`.
    let mut positional_maxlen: Option<PyObjectRef> = None;
    let mut iterable: Option<PyObjectRef> = None;
    let mut kw_maxlen: Option<PyObjectRef> = None;
    let mut iterable_seen = false;
    for arg in args.iter() {
        if let PyObject::Dict(d) = &*arg.borrow() {
            for (k, v) in d.items() {
                if k.str() == "maxlen" {
                    if kw_maxlen.is_some() || positional_maxlen.is_some() {
                        return Err(PyError::type_error(
                            "deque() got multiple values for argument 'maxlen'",
                        ));
                    }
                    kw_maxlen = Some(v);
                } else {
                    return Err(PyError::type_error(format!(
                        "deque() got an unexpected keyword argument '{}'",
                        k.str()
                    )));
                }
            }
        } else if !iterable_seen {
            iterable = Some(arg.clone());
            iterable_seen = true;
        } else if positional_maxlen.is_none() {
            positional_maxlen = Some(arg.clone());
        } else {
            return Err(PyError::type_error("deque() takes at most 2 arguments"));
        }
    }
    if positional_maxlen.is_some() && kw_maxlen.is_some() {
        return Err(PyError::type_error(
            "deque() got multiple values for argument 'maxlen'",
        ));
    }
    let maxlen_ref = positional_maxlen.or(kw_maxlen);
    let maxlen = if let Some(m) = maxlen_ref {
        // `maxlen=None` (positional or keyword) means UNBOUNDED.
        if matches!(&*m.borrow(), PyObject::None) {
            None
        } else {
            let n = m
                .as_i64()
                .ok_or_else(|| PyError::type_error("an integer is required"))?;
            if n < 0 {
                return Err(PyError::value_error("maxlen must be non-negative"));
            }
            Some(n as usize)
        }
    } else {
        None
    };
    let mut data: VecDeque<PyObjectRef> = VecDeque::new();
    if let Some(iter) = iterable {
        // Iterate through the real iterator protocol (NOT `__len__` +
        // `__getitem__`) — a lying source (`seq_tests.LyingTuple`) reports a
        // wrong `__len__` but a real iterator yields the true contents.
        let it = builtin_iter(&[iter])?;
        loop {
            match builtin_next(&[it.clone()]) {
                Ok(v) => {
                    data.push_back(v);
                    if let Some(maxlen) = maxlen {
                        if data.len() > maxlen {
                            data.pop_front();
                        }
                    }
                }
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(py_deque(data, maxlen))
}

pub fn builtin_tuple(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_tuple(Vec::new()));
    }
    if args.len() > 1 {
        return Err(PyError::type_error(format!(
            "tuple expected at most 1 argument, got {}",
            args.len()
        )));
    }
    // `tuple(t)` returns the SAME tuple (identity optimization,
    // test_tuple::test_constructors: `t0_3 is tuple(t0_3)`).
    if let PyObject::Tuple(_) = &*args[0].borrow() {
        return Ok(args[0].clone());
    }
    // `tuple(x)` accepts ANY iterable in real Python, not just the handful
    // of native container shapes this used to special-case — e.g.
    // `tuple(map(...))`, generators, custom `__iter__` objects all raised
    // "cannot convert '...' to tuple" instead of actually iterating. Same
    // general fix already applied to `set()`/`str.join`: materialize
    // through the real iterator protocol. Keep the List/Tuple fast paths
    // (avoid a full iterator round-trip for the overwhelmingly common
    // cases) and the Str fast path (per-character, not per-codepoint-int).
    {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => return Ok(py_tuple(v.clone())),
            PyObject::Tuple(v) => return Ok(py_tuple(v.clone())),
            PyObject::Str(s) => {
                let items: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                return Ok(py_tuple(items));
            }
            _ => {}
        }
    }
    let iterator = builtin_iter(&[args[0].clone()])?;
    let mut items: Vec<PyObjectRef> = Vec::new();
    if let Some(hint) = iterable_length_hint(&args[0]) {
        if items.try_reserve_exact(hint).is_err() {
            return Err(PyError::memory_error("could not allocate tuple"));
        }
    }
    loop {
        match builtin_next(&[iterator.clone()]) {
            Ok(v) => items.push(v),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(py_tuple(items))
}

/// `dict.__repr__` — a per-type repr so `type(x).__repr__` differs across
/// native container types (CPython's pprint dispatches on
/// `type(object).__repr__`; a shared identity repr made dict and deque
/// collide and route dicts through pprint's deque formatter).
pub fn builtin_dict_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__repr__ requires 1 argument"));
    }
    Ok(py_str(&args[0].repr()))
}

/// `deque.__repr__` — see `builtin_dict_repr`.
pub fn builtin_deque_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__repr__ requires 1 argument"));
    }
    Ok(py_str(&args[0].repr()))
}

// Per-type `__repr__` functions. Each native container/type must have its
// OWN `__repr__` function object: CPython's pprint dispatches on
// `type(object).__repr__` (and other code compares `x.__repr__ is
// y.__repr__`), so sharing one identity repr across all types made them
// indistinguishable (dict routed through pprint's deque formatter, etc.).
macro_rules! native_repr_fn {
    ($name:ident) => {
        pub fn $name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.is_empty() {
                return Err(PyError::type_error("__repr__ requires 1 argument"));
            }
            Ok(py_str(&args[0].repr()))
        }
    };
}
native_repr_fn!(builtin_list_repr);
native_repr_fn!(builtin_tuple_repr);
native_repr_fn!(builtin_str_repr);
native_repr_fn!(builtin_bytes_repr);
native_repr_fn!(builtin_bytearray_repr);
native_repr_fn!(builtin_int_repr);
native_repr_fn!(builtin_float_repr);
native_repr_fn!(builtin_complex_repr);
native_repr_fn!(builtin_bool_repr);
native_repr_fn!(builtin_set_repr);
native_repr_fn!(builtin_frozenset_repr);
native_repr_fn!(builtin_slice_repr);

pub fn builtin_dict(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_dict());
    }
    let mut d = PyDict::new();
    let arg = args[0].borrow();
    match &*arg {
        PyObject::Dict(other) => {
            for (k, v) in other.items() {
                d.set(k, v)?;
            }
        }
        _ => {
            drop(arg);
            // Mapping-like objects (anything with a `.keys()` method —
            // notably dict *subclass* instances like Counter/defaultdict/
            // ChainMap, which are never literally PyObject::Dict) are
            // copied key-by-key, matching real `dict(mapping)` semantics.
            // Only objects without `.keys()` fall back to being treated
            // as an iterable of (key, value) pairs.
            let type_name = args[0].borrow().type_name();
            let is_view = matches!(type_name.as_str(), "dict_items" | "dict_keys" | "dict_values" | "KeysView" | "ItemsView" | "ValuesView" | "MappingView");
            let keys_method = if is_view { None } else { args[0].borrow().get_attribute("keys").ok() };
            if let Some(keys_raw) = keys_method {
                let keys_iterable = call_bound_method(keys_raw, args[0].clone(), vec![])?;
                let it = builtin_iter(&[keys_iterable])?;
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(key) => {
                            let value = py_getitem(&args[0], &key)?;
                            d.set(key, value)?;
                        }
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            } else {
                // An iterable of (key, value) pairs
                let it = builtin_iter(&[args[0].clone()])?;
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(pair) => {
                            let pair_b = pair.borrow();
                            let items: Vec<PyObjectRef> = match &*pair_b {
                                PyObject::Tuple(v) | PyObject::List(v) => v.clone(),
                                _ => return Err(PyError::type_error("cannot convert dictionary update sequence element to a sequence")),
                            };
                            if items.len() != 2 {
                                return Err(PyError::value_error(format!(
                                    "dictionary update sequence element has length {}; 2 is required", items.len()
                                )));
                            }
                            drop(pair_b);
                            d.set(items[0].clone(), items[1].clone())?;
                        }
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
    // Real `dict()` only ever takes ONE positional argument
    // (`dict(mapping_or_iterable=(), /, **kwargs)`) — this project's own
    // calling convention packs a call's keyword arguments into ONE extra
    // trailing dict appended to `args` (see `call_function`'s
    // `PyObject::BuiltinFunction` handling), so any `args[1..]` seen here is
    // never a second genuine positional, always that packed kwargs dict.
    // Merge it on top (kwargs win on key collisions, matching real
    // `dict(d, **kwargs)` semantics) — needed for real stdlib code like
    // `argparse.py`'s `dict(kwargs, dest=dest, option_strings=[])`, which
    // previously silently dropped `dest`/`option_strings` entirely.
    for extra in &args[1..] {
        if let PyObject::Dict(other) = &*extra.borrow() {
            for (k, v) in other.items() {
                d.set(k, v)?;
            }
        }
    }
    Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
}

pub fn builtin_set(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_set());
    }
    // `set(x)` accepts any iterable in real Python — dicts (yielding keys),
    // generators, custom `__iter__` objects, etc., not just the handful of
    // native container shapes this used to special-case (which also had
    // its own bug: `set("abc")` produced a set of *codepoint ints* instead
    // of single-character strings, since the Str arm bypassed the normal
    // per-character string wrapping every other string-iteration path
    // uses). Materializing through the real iterator protocol fixes both
    // at once and matches the same general fix already applied to
    // `str.join`.
    let iterator = crate::object::builtin_iter(&[args[0].clone()])?;
    let mut elts: Vec<PyObjectRef> = Vec::new();
    loop {
        match crate::object::builtin_next(&[iterator.clone()]) {
            Ok(v) => elts.push(v),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(PyObjectRef::new(PyObject::Set(PySet::from_vec(elts)?)))
}

pub fn builtin_bytes(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())))
    } else {
        // Buffer protocol: try memoryview first (e.g. bytes(MyBuffer()) where MyBuffer defines __buffer__)
        {
            let obj = args[0].clone();
            if let Ok(mv) = crate::object::builtin_memoryview(&[obj.clone()]) {
                if let Ok(bytes) = crate::object::mv_tobytes(&mv) {
                    return Ok(PyObjectRef::imm(PyObject::Bytes(bytes)));
                }
            }
        }
        // PickleBuffer and memoryview are bytes-like via buffer protocol
        {
            let b = args[0].borrow();
            if let PyObject::Instance { typ, dict } = &*b {
                let is_pb = if let PyObject::Type { name, .. } = &*typ.borrow() {
                    name == "PickleBuffer"
                } else {
                    false
                };
                if is_pb {
                    let released = dict
                        .get("_released")
                        .map(|v| v.truthy())
                        .unwrap_or(false);
                    if released {
                        return Err(PyError::value_error(
                            "operation forbidden on released PickleBuffer object",
                        ));
                    }
                    let underlying = dict.get("_obj").cloned().unwrap_or_else(py_none);
                    drop(b);
                    return builtin_bytes(&[underlying]);
                }
            }
            if let PyObject::MemoryView { released, .. } = &*b {
                if *released {
                    return Err(PyError::value_error(
                        "operation forbidden on released memoryview object",
                    ));
                }
                // fall through to dedicated memoryview handling below after drop
            }
        }
        // memoryview -> bytes is a direct tobytes copy
        if matches!(&*args[0].borrow(), PyObject::MemoryView { .. }) {
            let bytes = crate::object::mv_tobytes(&args[0])?;
            return Ok(PyObjectRef::imm(PyObject::Bytes(bytes)));
        }
        let obj = args[0].borrow();
        match &*obj {
            // Same fix as `bytearray(n)` above: `bytes(n)` zero-fills a
            // buffer of length `n`, it doesn't wrap `n` as a single byte
            // value.
            PyObject::Int(i) => {
                let n = i
                    .to_i64()
                    .ok_or_else(|| PyError::value_error("bytes() argument must be non-negative"))?;
                if n < 0 {
                    return Err(PyError::value_error(
                        "bytes() argument must be non-negative",
                    ));
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(vec![0u8; n as usize])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::imm(PyObject::Bytes(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Set(items) | PyObject::FrozenSet(items) => {
                let mut result = Vec::new();
                for item in items.to_vec() {
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            _ => {
                drop(obj);
                // Same fix as `builtin_list`'s matching site: propagate
                // `builtin_iter`'s error as-is rather than replacing it with
                // a generic message (swallowing a real exception raised
                // from inside a custom `__iter__`).
                let it = builtin_iter(&[args[0].clone()])?;
                let mut result = Vec::new();
                loop {
                    let item = match builtin_next(&[it.clone()]) {
                        Ok(val) => val,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    };
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
        }
    }
}

/// bytes.fromhex(string) -> bytes
///
/// Create a bytes object from a string of hexadecimal digits.
pub fn builtin_bytes_fromhex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "bytes.fromhex() takes exactly 1 argument (0 given)",
        ));
    }
    let s = args[0].str();
    // Remove spaces (CPython allows spaces in the hex string)
    let s = s.replace(' ', "");
    if s.len() % 2 != 0 {
        return Err(PyError::value_error("hex string must be of even length"));
    }
    let mut result = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hex_pair = std::str::from_utf8(chunk)
            .map_err(|_| PyError::value_error("non-hexadecimal number found"))?;
        let byte = u8::from_str_radix(hex_pair, 16).map_err(|_| {
            PyError::value_error(format!(
                "non-hexadecimal number found in fromhex() arg at position {}",
                s.find(hex_pair).unwrap_or(0)
            ))
        })?;
        result.push(byte);
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
}

pub fn builtin_bytearray(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::new(PyObject::ByteArray(Vec::new())))
    } else {
        let obj = args[0].borrow();
        match &*obj {
            // Real `bytearray(n)` (a single non-negative int argument)
            // creates a zero-filled buffer of length `n` — NOT a
            // single-element buffer holding the byte value `n` (that's
            // `bytes([n])`, a completely different construction). This
            // matched the length-1 anti-pattern instead, silently breaking
            // the extremely common "pre-allocate an I/O buffer"
            // idiom — found via `struct.pack_into`'s own doctest-style
            // idiom `bytearray(10)`.
            PyObject::Int(i) => {
                let n = i.to_i64().ok_or_else(|| {
                    PyError::value_error("bytearray() argument must be non-negative")
                })?;
                if n < 0 {
                    return Err(PyError::value_error(
                        "bytearray() argument must be non-negative",
                    ));
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(vec![0u8; n as usize])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::new(PyObject::ByteArray(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytearray() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytearray() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytearray() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytearray() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytearray() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytearray() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Range { .. } => {
                // Any iterable of ints (range, generators, custom __iter__)
                // is valid (test_pprint: bytearray(range(5))).
                drop(obj);
                let it = builtin_iter(&[args[0].clone()])?;
                let mut result = Vec::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(item) => {
                            let n = item.as_i64().ok_or_else(|| {
                                PyError::value_error("bytearray() requires int in range 0-255")
                            })?;
                            if n < 0 || n > 255 {
                                return Err(PyError::value_error(
                                    "bytearray() requires int in range 0-255",
                                ));
                            }
                            result.push(n as u8);
                        }
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            _ => Err(PyError::type_error(format!(
                "cannot convert '{}' to bytearray",
                obj.type_name()
            ))),
        }
    }
}

pub fn builtin_frozenset(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::imm(PyObject::FrozenSet(PySet::new())))
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Set(s) => Ok(PyObjectRef::imm(PyObject::FrozenSet(s.clone()))),
            PyObject::FrozenSet(s) => Ok(PyObjectRef::imm(PyObject::FrozenSet(s.clone()))),
            PyObject::List(v) => {
                let mut set = PySet::new();
                for item in v {
                    set.add(item.clone())?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Tuple(v) => {
                let mut set = PySet::new();
                for item in v {
                    set.add(item.clone())?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Str(s) => {
                let mut set = PySet::new();
                for ch in s.chars() {
                    set.add(py_str(&ch.to_string()))?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Bytes(b) => {
                let mut set = PySet::new();
                for &byte in b {
                    set.add(py_int(byte as i64))?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Range { .. } => {
                drop(obj);
                let it = builtin_iter(&[args[0].clone()])?;
                let mut set = PySet::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(item) => set.add(item.clone())?,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            _ => Err(PyError::type_error(format!(
                "cannot convert '{}' to frozenset",
                obj.type_name()
            ))),
        }
    }
}

pub fn builtin_format(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        0 => Err(PyError::type_error("format() requires at least 1 argument")),
        1 => Ok(py_str(&args[0].str())),
        2 => {
            if matches!(&*args[1].borrow(), PyObject::None) {
                return Err(PyError::type_error(
                    "format() argument 2 must be str, not NoneType",
                ));
            }
            let spec = args[1].str();
            if spec.is_empty() {
                return Ok(py_str(&args[0].str()));
            }
            // Use the comprehensive format_with_spec from vm.rs
            let result = crate::vm::format_with_spec(&args[0], &spec)
                .map_err(|e| PyError::value_error(format!("Format spec: {}", e)))?;
            Ok(py_str(&result))
        }
        _ => Err(PyError::type_error("format() takes at most 2 arguments")),
    }
}

pub fn builtin_object(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Create a new bare object instance
    let object_type = PyObjectRef::new(PyObject::Type {
        name: "object".to_string(),
        dict: Box::new(TypeDict::default()),
        bases: vec![],
        mro: vec![],
    });
    Ok(PyObjectRef::new(PyObject::Instance {
        typ: object_type,
        dict: AttrMap::new(),
    }))
}

pub fn builtin_hash(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hash() takes exactly one argument"));
    }
    // Delegate entirely to `PyObjectRef::hash()` — the SAME method
    // `PyDict`/`PySet` call internally to hash a key. This used to
    // reimplement a SEPARATE algorithm per type here (FNV-1a for
    // str/bytes/bytearray/tuple, vs. the byte-multiplier/char-multiplier
    // algorithms `PyObjectRef::hash()`/`PyObject::hash()` actually use for
    // dict/set storage) — meaning `hash(x)` as seen by Python code could
    // disagree with the hash ACTUALLY used when `x` is placed in a dict/set,
    // a correctness invariant `hash()` exists specifically to uphold.
    // Confirmed via `hash("hello")` returning a completely different value
    // depending on whether "hello" happened to be represented as an inline
    // `SmallStr` or a boxed `PyObject::Str` — both must, and now do, agree.
    // This also fixes a second bug: the old `_` catch-all silently hashed
    // genuinely unhashable types (`list`, `dict`, `set`) by pointer instead
    // of raising `TypeError: unhashable type: '...'` like real Python.
    Ok(py_int(args[0].hash()? as i64))
}

pub fn builtin_slice(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        1 => {
            let stop = args[0].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start: none.clone(),
                stop,
                step: none,
            }))
        }
        2 => {
            let start = args[0].clone();
            let stop = args[1].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start,
                stop,
                step: none,
            }))
        }
        3 => Ok(PyObjectRef::imm(PyObject::Slice {
            start: args[0].clone(),
            stop: args[1].clone(),
            step: args[2].clone(),
        })),
        _ => Err(PyError::type_error("slice() takes at most 3 arguments")),
    }
}

pub fn builtin_dir(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return crate::object::with_vm_mut(|vm| {
            let frame = vm
                .frames
                .last()
                .ok_or_else(|| crate::object::PyError::runtime_error("no frame"))?;
            let mut names: Vec<PyObjectRef> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            // Locals via InternedMap (used for some scopes)
            for (k, _) in frame.locals.iter() {
                let s = crate::interner::lookup_str(k);
                if seen.insert(s.to_string()) {
                    names.push(py_str(s));
                }
            }
            // Fast locals via varnames
            for (idx, var_id) in frame.code.varnames.iter().enumerate() {
                if frame
                    .fast_locals
                    .get(idx)
                    .and_then(|v| v.as_ref())
                    .is_some()
                {
                    let s = crate::interner::lookup_str(*var_id);
                    if seen.insert(s.to_string()) {
                        names.push(py_str(s));
                    }
                }
            }
            // For module frames, locals/varnames may be empty, but globals holds the module dict
            if names.is_empty() {
                for (k, _) in frame.globals.borrow().iter() {
                    let s = crate::interner::lookup_str(*k);
                    if seen.insert(s.to_string()) {
                        names.push(py_str(s));
                    }
                }
            }
            names.sort_by(|a, b| a.str().cmp(&b.str()));
            Ok(py_list(names))
        })?;
    }
    let obj = args[0].borrow();
    let mut names = Vec::new();
    match &*obj {
        PyObject::Instance { dict, typ } => {
            for key in dict.keys() {
                names.push(py_str(key));
            }
            // Instance dir() must also surface the class's own and inherited
            // members (CPython: dir(x) == sorted(set(vars(x)) | union of
            // vars over type(x).__mro__)). Without this, `dir(obj)` on any
            // user class omitted every method -- breaking patterns like
            // configparser's ConverterMapping, which discovers getters via
            // dir(parser).
            if let PyObject::Type {
                dict: tdict,
                mro,
                ..
            } = &*typ.borrow()
            {
                let mut seen = std::collections::HashSet::new();
                for key in tdict.keys() {
                    let s = interner::lookup_str(*key);
                    if s.starts_with("__native") {
                        continue;
                    }
                    if seen.insert(*key) {
                        names.push(py_str(s));
                    }
                }
                for base in mro {
                    if let PyObject::Type {
                        dict: base_dict, ..
                    } = &*base.borrow()
                    {
                        for key in base_dict.keys() {
                            let s = interner::lookup_str(*key);
                            if s.starts_with("__native") {
                                continue;
                            }
                            if seen.insert(*key) {
                                names.push(py_str(s));
                            }
                        }
                    }
                }
            }
        }
        PyObject::Module { dict, .. } => {
            for key in dict.keys() {
                names.push(py_str(interner::lookup_str(*key)));
            }
        }
        PyObject::Type { dict, mro, .. } => {
            // `dir(SomeClass)` must include every name reachable via the
            // class's OWN dict AND every ancestor's (`mro`) — real
            // CPython's `dir()` on a class is `sorted(set().union(*(vars(c)
            // for c in cls.__mro__)))`. This previously only read the
            // class's OWN dict, so `dir()` on ANY class with inherited
            // members (i.e. virtually every class with a base other than
            // bare `object`) silently omitted every name defined on a
            // parent — confirmed via `class Combined(Mixin, unittest.
            // TestCase): pass`, where `dir(Combined)` omitted `Mixin`'s own
            // `test_*` methods even though `hasattr`/`getattr` found them
            // fine (a real, general attribute-LOOKUP-vs-`dir()`-enumeration
            // gap this was hiding) — `unittest`'s own `TestLoader.
            // getTestCaseNames` uses exactly this `dir()` call to discover
            // test methods, so this silently dropped every test defined on
            // a mixin base for any multiple-inheritance test class.
            let mut seen = std::collections::HashSet::new();
            for key in dict.keys() {
                let s = interner::lookup_str(*key);
                if s.starts_with("__native") {
                    continue;
                }
                if seen.insert(*key) {
                    names.push(py_str(s));
                }
            }
            for base in mro {
                if let PyObject::Type {
                    dict: base_dict, ..
                } = &*base.borrow()
                {
                    for key in base_dict.keys() {
                        let s = interner::lookup_str(*key);
                        if s.starts_with("__native") {
                            continue;
                        }
                        if seen.insert(*key) {
                            names.push(py_str(s));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    // Add basic attributes for all types EXCEPT modules — real
    // `dir(some_module)` is just `sorted(vars(some_module).keys())`, with
    // no implicit `__class__`/`__dir__` added (those come from a type's
    // MRO walk, which modules don't participate in the same way instances/
    // classes do). Confirmed via `test_pkg.py`, which compares `dir()` on
    // freshly-imported packages against an exact expected list that does
    // NOT include either.
    if !matches!(&*obj, PyObject::Module { .. }) {
        names.push(py_str("__class__"));
        names.push(py_str("__dir__"));
    }
    names.sort_by(|a, b| {
        let a = a.borrow();
        let b = b.borrow();
        if let (PyObject::Str(a), PyObject::Str(b)) = (&*a, &*b) {
            a.cmp(b)
        } else {
            std::cmp::Ordering::Equal
        }
    });
    Ok(py_list(names))
}

pub fn builtin_globals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm
            .frames
            .last()
            .ok_or_else(|| PyError::runtime_error("no frame"))?;
        // Return a LIVE view of the frame's globals so mutations
        // (`globals()['len'] = f`, test_dynamic::test_globals_shadow_builtins)
        // are visible to `LOAD_GLOBAL`, which reads the same backing map.
        Ok(PyObjectRef::new(PyObject::Globals(frame.globals.clone())))
    })?
}

pub fn builtin_locals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm
            .frames
            .last()
            .ok_or_else(|| PyError::runtime_error("no frame"))?;
        let mut d = crate::object::PyDict::new();
        for (k, v) in frame.locals.iter() {
            let name = crate::interner::lookup(k);
            d.set(py_str(&name), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
    })?
}



/// Invoke `func` in a fresh disposable VM, supporting KEYWORD arguments —
/// unlike `call_bound_method` (which only forwards positionals). Needed by
/// `atexit._run_exitfuncs` (`register(func, 3, key='value')` callbacks), and
/// generally useful for running a user `Function` from native code with the
/// full calling convention without re-entering the live VM's execute loop
/// (which is what `vm.call_function` does from inside a builtin, and which
/// misbehaves for user Functions).
pub fn call_function_disposable(
    func: &PyObjectRef,
    args: Vec<PyObjectRef>,
    keywords: Vec<(String, PyObjectRef)>,
) -> PyResult<PyObjectRef> {
    match &*func.borrow() {
        PyObject::BuiltinFunction { func: f, .. } => f(&args),
        PyObject::Closure(c) => c(&args),
        PyObject::BuiltinMethod {
            func: f, self_obj, ..
        } => {
            let mut all = vec![self_obj.clone()];
            all.extend(args);
            f(&all)
        }
        PyObject::BoundMethod { func, self_obj } => {
            // A bound method (user-defined `self.method`) reached through the
            // disposable path — prepend its bound self, then dispatch the
            // underlying callable (real trigger: atexit's `_run_exitfuncs`
            // reporting through a `sys.unraisablehook` that is a bound method,
            // as `test.support.catch_unraisable_exception` installs).
            let mut all = vec![self_obj.clone()];
            all.extend(args);
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut vm = crate::vm::VirtualMachine::new();
            vm.call_function(func.clone(), all, keywords)
        }
        PyObject::Function(_) => {
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut vm = crate::vm::VirtualMachine::new();
            vm.call_function(func.clone(), args, keywords)
        }
        PyObject::Type { .. } => {
            // Calling a class (user class / namedtuple factory result /
            // builtin type) from a native closure — route through a fresh VM
            // so instance creation runs the normal __new__/__init__ path.
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut vm = crate::vm::VirtualMachine::new();
            vm.call_function(func.clone(), args, keywords)
        }
        _ => Err(PyError::type_error(format!(
            "'{}' object is not callable",
            func.borrow().type_name()
        ))),
    }
}

pub fn call_bound_method(
    func: PyObjectRef,
    self_obj: PyObjectRef,
    args: Vec<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    match &*func.borrow() {
        PyObject::BuiltinMethod {
            func: f,
            self_obj: s,
            ..
        } => {
            let mut all_args = vec![s.clone()];
            all_args.push(self_obj);
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::BuiltinFunction { func: f, .. } => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::Closure(func) => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            func(&all_args)
        }
        PyObject::Function(ref f) => {
            // See `NativeDispatchRecursionGuard`'s own doc comment (`core.rs`)
            // — without this, recursion flowing through this disposable-VM
            // dispatch path (e.g. `A.__call__ = A(); A()()`) overflows the
            // real native stack instead of raising a catchable
            // `RecursionError`, since each nested call resets its own fresh
            // VM's frame counter to zero.
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let code = &f.code;
            let g = &f.globals;
            let defaults = &f.defaults;
            let fname = &f.code.name;
            let closure = &f.closure;
            if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                eprintln!(
                    "CALL_BOUND_METHOD (disposable VM): fname={} code_name={} filename={}",
                    fname, code.name, code.filename
                );
            }
            if std::env::var("RPY_DEBUG_CBM").is_ok() {
                eprintln!(
                    "call_bound_method: fname={} varnames={:?} args.len()={} arg_count={}",
                    fname,
                    code.varnames,
                    args.len(),
                    code.arg_count
                );
            }
            // The disposable VM is constructed BEFORE the frame (and the
            // frame borrows ITS `builtins` map, not a separately-built one)
            // deliberately: `vm.rs`'s `call_function` special-cases `type(x)`
            // (and a few other builtins) by POINTER IDENTITY against
            // `self.builtins.get("type")` — if the frame's own `builtins` map
            // were built via a second, independent `create_builtins()` call
            // (as this used to do), it would contain a structurally-identical
            // but NOT pointer-identical "type" object, so that identity check
            // silently fails and `type(x)` falls through to being treated as
            // an ordinary call to the `type` class (constructing a bogus
            // instance-of-`type`) instead of returning `x`'s real type.
            // Confirmed general via a Django-free repro: `type(self).__name__`
            // inside a function invoked through `call_bound_method` (e.g. any
            // user-defined `__repr__` reached via the `repr()` builtin)
            // raised `AttributeError: 'type' object has no attribute
            // '__name__'` — `type(self)` had silently returned a fresh
            // `type`-instance instead of the real class object.
            let mut vm = crate::vm::VirtualMachine::new();
            let mut frame = crate::vm::Frame::new(
                code.clone(),
                g.clone(),
                std::rc::Rc::clone(&vm.builtins),
                None,
            );
            // Without this, ANY closure-capturing function invoked via
            // call_bound_method (repr()/str()/hash()/comparisons/other
            // native builtins that call a user-defined dunder this way,
            // instead of through the normal CALL opcode's own frame setup
            // in vm.rs, which already does set this) silently lost every
            // free variable — "variable 'x' not found" the moment the
            // function's body referenced one. Confirmed general via a
            // Django-free repro: a `dataclass`-generated `__repr__` closing
            // over its own field-name list worked fine called directly
            // (`obj.__repr__()`) but raised NameError through `repr(obj)`.
            frame.closure = Box::new(closure.clone());
            let code = code.clone();
            let defaults = defaults.clone();
            // Set self at index 0
            if !code.varnames.is_empty() {
                frame.fast_locals[0] = Some(self_obj.clone());
                frame.insert_local(crate::interner::lookup_str(code.varnames[0]), self_obj);
            }
            let npos = args.len();
            let named_params = if code.vararg_name.is_some() || code.kwarg_name.is_some() {
                code.varnames
                    .iter()
                    .position(|n| {
                        code.vararg_name.as_ref().map(|b| b.as_str())
                            == Some(crate::interner::lookup_str(*n))
                            || code.kwarg_name.as_ref().map(|b| b.as_str())
                                == Some(crate::interner::lookup_str(*n))
                    })
                    .unwrap_or(code.varnames.len())
            } else {
                code.varnames.len()
            };
            for i in 0..npos.min(named_params.saturating_sub(1)) {
                let idx = i + 1;
                if idx < code.varnames.len() {
                    frame.fast_locals[idx] = Some(args[i].clone());
                    frame.insert_local(
                        crate::interner::lookup_str(code.varnames[idx]),
                        args[i].clone(),
                    );
                }
            }
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in (named_params.saturating_sub(1))..npos {
                    extra.push(args[i].clone());
                }
                let vararg_val = py_tuple(extra);
                // Must ALSO land in `fast_locals` — `LOAD_FAST` reads that,
                // not the `insert_local` name dict. Missing this meant any
                // `*args`-taking function/method invoked through THIS
                // disposable-VM path (constructing an instance whose class
                // was invoked via `map()`/`filter()`/etc., e.g. `map(TestCase,
                // testMethodNames)` calling each `TestCase.__init__(self,
                // *args, **kwargs)`) raised "local variable 'args' referenced
                // before assignment" the instant the function body read its
                // own vararg tuple — real trigger: CPython's own
                // `test_descr.py`'s `OperatorsTest.__init__(self, *args,
                // **kwargs)`, loaded via `unittest`'s `loadTestsFromTestCase`.
                if let Some(idx) = code
                    .varnames
                    .iter()
                    .position(|n| crate::interner::lookup_str(*n) == vararg_name.as_str())
                {
                    if idx < frame.fast_locals.len() {
                        frame.fast_locals[idx] = Some(vararg_val.clone());
                    }
                }
                frame.insert_local(vararg_name.as_str(), vararg_val);
            }
            if npos > named_params.saturating_sub(1) && code.vararg_name.is_none() {
                return Err(PyError::type_error(format!(
                    "{}() takes {} positional argument but {} was given",
                    fname,
                    code.arg_count,
                    npos + 1
                )));
            }
            if npos < named_params.saturating_sub(1) {
                let num_defaults = code.num_defaults;
                for i in npos..named_params.saturating_sub(1) {
                    let idx = i + 1;
                    if idx < code.varnames.len() {
                        let default_idx =
                            num_defaults.saturating_sub(named_params.saturating_sub(1) - i);
                        if default_idx < defaults.len() {
                            let val = defaults[default_idx].clone();
                            frame.fast_locals[idx] = Some(val.clone());
                            frame
                                .insert_local(crate::interner::lookup_str(code.varnames[idx]), val);
                        }
                    }
                }
            }
            // This path never receives keyword arguments at all (its own
            // signature is positional-args-only), so `**kwargs` always ends
            // up empty — but it still needs to be BOUND to an empty dict,
            // not left entirely unset, or a `**kwargs`-taking function body
            // hits the exact same "local variable referenced before
            // assignment" as the vararg case just above the moment it reads
            // its own kwarg parameter (real trigger: the same
            // `OperatorsTest.__init__(self, *args, **kwargs)` scenario,
            // whose body explicitly re-unpacks `**kwargs` into another call).
            if let Some(kwarg_name) = &code.kwarg_name {
                if let Some(idx) = code
                    .varnames
                    .iter()
                    .position(|n| crate::interner::lookup_str(*n) == kwarg_name.as_str())
                {
                    if idx < frame.fast_locals.len() {
                        frame.fast_locals[idx] = Some(py_dict());
                    }
                }
                if !frame.contains_local(kwarg_name) {
                    frame.insert_local(kwarg_name.as_str(), py_dict());
                }
            }
            if std::env::var("RPY_DEBUG_CBM").is_ok() {
                eprintln!(
                    "call_bound_method: fast_locals after setup = {:?}",
                    frame
                        .fast_locals
                        .iter()
                        .map(|v| v.as_ref().map(|x| x.repr()))
                        .collect::<Vec<_>>()
                );
            }
            vm.push_frame(frame);
            vm.execute()
        }
        PyObject::BoundMethod { func, .. } => {
            let mut all_args = vec![self_obj.clone()];
            all_args.extend(args);
            call_bound_method(func.clone(), self_obj, all_args)
        }
        // The single-argument form of the `type` builtin itself (`type(x)`
        // — get x's real type) reaching this path as a plain callable
        // value, e.g. passed as a `key=` function to a native higher-order
        // builtin (`itertools.groupby(data, type)` — real trigger:
        // CPython's own `Lib/statistics.py`). `type` is represented as a
        // real `PyObject::Type{name:"type",..}` here (not a
        // `BuiltinFunction`, unlike most other native callables), so it
        // fell through to the generic "object is not callable" error
        // otherwise. Delegates to the same `builtin_type_of` helper the
        // real CALL-opcode path and `.__class__` both already use — not a
        // new implementation. Other `PyObject::Type` calls (constructing an
        // actual instance, or `type(name, bases, dict)`) still aren't
        // supported through THIS path — only real Python code going
        // through the live VM's own `call_function` gets that; this is
        // scoped to the one case that's actually reachable from a native
        // higher-order builtin's disposable-VM-free call convention.
        PyObject::Type { name, .. } if name == "type" && args.is_empty() => {
            builtin_type_of(&[self_obj])
        }
        _ => Err(PyError::type_error("object is not callable")),
    }
}



pub fn builtin_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() == 1 {
        if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
            if let Some(rc) = target.upgrade() {
                return builtin_iter(&[PyObjectRef::Imm(rc)]);
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }
    }
    // Two-argument form: `iter(callable, sentinel)` — calls `callable()`
    // repeatedly, yielding each result until one equals `sentinel`. Real,
    // commonly-used Python (`iter(file.readline, '')` is the classic
    // idiom), not just a test-only construct.
    if args.len() == 2 {
        if !builtin_callable(&[args[0].clone()])?.truthy() {
            return Err(PyError::type_error(format!(
                "iter(v, w): v must be callable"
            )));
        }
        return Ok(PyObjectRef::new(PyObject::CallSentinelIter {
            func: args[0].clone(),
            sentinel: args[1].clone(),
            exhausted: false,
        }));
    }
    if args.len() != 1 {
        return Err(PyError::type_error("iter() takes exactly one argument"));
    }
    // Check for __iter__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__iter__"),
            PyObject::Generator { .. } => {
                // Generators are their own iterator (return self)
                return Ok(args[0].clone());
            }
            // A class object itself, iterable via its metaclass's
            // `__iter__` (e.g. `iter(SomeEnum)` / `list(SomeEnum)`) — see
            // the matching GET_ITER opcode handling in vm.rs for why this
            // needs metatype_of rather than ordinary attribute lookup.
            PyObject::Type { .. } => {
                metatype_of(&args[0]).and_then(|mt| lookup_dunder_via_mro(&mt, "__iter__"))
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![]);
    }
    if let Some(native) = native_backing_of(&args[0]) {
        return builtin_iter(&[native]);
    }
    // Real Python's "old-style sequence iteration" fallback: an object with
    // `__getitem__` but no `__iter__` is still iterable — `for x in obj:`
    // calls `obj[0]`, `obj[1]`, ... until `IndexError`. Checked AFTER the
    // `__iter__` lookup above (which already returned if present) and
    // BEFORE the native-type match below (native types needing this exist
    // as their own dedicated arms already).
    if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
        if lookup_dunder_via_mro(typ, "__getitem__").is_some() {
            return Ok(PyObjectRef::new(PyObject::GetItemIter {
                obj: args[0].clone(),
                index: 0,
            }));
        }
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Tuple(v) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: v.clone(),
            index: 0,
        })),
        PyObject::Str(s) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: s.chars().map(|c| py_str(&c.to_string())).collect(),
            index: 0,
        })),
        PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: b.iter().map(|byte| py_int(*byte as i64)).collect(),
            index: 0,
        })),
        PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: b.iter().map(|byte| py_int(*byte as i64)).collect(),
            index: 0,
        })),
        PyObject::Array(arr) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: arr
                .data
                .iter()
                .map(|v| {
                    if crate::object::array_typecode_is_float(arr.typecode) {
                        py_float(*v)
                    } else if arr.typecode == 'w' || arr.typecode == 'u' {
                        let ch = (*v as u32).try_into().ok().and_then(char::from_u32).unwrap_or('\0');
                        py_str(&ch.to_string())
                    } else {
                        py_int(*v as i64)
                    }
                })
                .collect(),
            index: 0,
        })),
        PyObject::MemoryView { .. } => {
            drop(obj);
            let len = mv_len(&args[0])?;
            let mut items = Vec::with_capacity(len);
            for i in 0..len {
                items.push(mv_getitem(&args[0], &py_int(i as i64))?);
            }
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: items,
                index: 0,
            }))
        }
        // `iter(a_set)` must return a real ITERATOR (advanceable via
        // `builtin_next`), not the bare materialized list `py_list` builds —
        // a raw `PyObject::List` isn't itself a valid iterator shape (unlike
        // `List`/`Dict` just above and below, both correctly wrapped in
        // `ListIter`). This meant `for x in frozenset(...)`/`for x in
        // some_set:` raised `TypeError: 'frozenset' object is not
        // iterable` outright — a foundational gap for two of Python's most
        // basic builtin container types. Real trigger: vendoring
        // `_strptime.py` (`for i in calendar.day_abbr` style iteration
        // deeper in its own dependency chain hits a frozenset somewhere in
        // `unicodedata`/locale data) — but the bug is general, not specific
        // to that file.
        PyObject::Set(s) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: s.to_vec(),
            index: 0,
        })),
        PyObject::FrozenSet(s) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: s.to_vec(),
            index: 0,
        })),
        PyObject::Range { start, stop, step } => Ok(PyObjectRef::new(PyObject::RangeIter {
            current: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
        })),
        PyObject::List(v) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: v.clone(),
            index: 0,
        })),
        PyObject::Deque { data, .. } => Ok(PyObjectRef::new(PyObject::DequeIter {
            deque: args[0].clone(),
            index: 0,
            start_len: data.len(),
        })),
        PyObject::Dict(d) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: d.keys(),
            index: 0,
        })),
        PyObject::Globals(g) => {
            let keys: Vec<PyObjectRef> = g
                .borrow()
                .keys()
                .map(|k| py_str(interner::lookup_str(*k)))
                .collect();
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: keys,
                index: 0,
            }))
        }
        // `iter(f)`/`for line in f:` — see the matching `GET_ITER` opcode
        // handling in `vm.rs` for the full story; this is the SEPARATE
        // free-function path (`iter(f)` called explicitly, or anything
        // routing through `collect_iterable`) that needs the identical fix.
        PyObject::File { file, binary, .. } => {
            use std::io::Read;
            let mut rest = Vec::new();
            file.borrow_mut()
                .read_to_end(&mut rest)
                .map_err(|e| PyError::os_error_from_io(&e))?;
            let mut lines: Vec<PyObjectRef> = Vec::new();
            let mut current: Vec<u8> = Vec::new();
            for byte in rest {
                current.push(byte);
                if byte == b'\n' {
                    lines.push(if *binary {
                        PyObjectRef::imm(PyObject::Bytes(current.clone()))
                    } else {
                        py_str(&String::from_utf8_lossy(&current))
                    });
                    current.clear();
                }
            }
            if !current.is_empty() {
                lines.push(if *binary {
                    PyObjectRef::imm(PyObject::Bytes(current.clone()))
                } else {
                    py_str(&String::from_utf8_lossy(&current))
                });
            }
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: lines,
                index: 0,
            }))
        }
        // Already an iterator object (one of `builtin_next`'s own
        // recognized variants) — `iter(it)` on an existing iterator
        // just returns it unchanged, matching real Python.
        PyObject::ListIter { .. }
        | PyObject::RangeIter { .. }
        | PyObject::CycleIter { .. }
        | PyObject::EnumerateIter { .. }
        | PyObject::MapIterator { .. }
        | PyObject::FilterIterator { .. }
        | PyObject::ZipIterator { .. }
        | PyObject::FutureAwaitIterator { .. }
        | PyObject::GroupByIter { .. }
        | PyObject::GetItemIter { .. }
        | PyObject::CallSentinelIter { .. }
        | PyObject::DequeIter { .. }
        | PyObject::DequeRevIter { .. } => Ok(args[0].clone()),
        // Anything else (plain functions, ints, ...) is genuinely not
        // iterable. The previous fallback (`Ok(args[0].clone())`)
        // silently treated ANY object as if it were already a valid
        // iterator instead of raising here — `builtin_next` then had no
        // recognized shape to advance either, and its OWN fallback
        // apparently tried calling the object as if `__next__` meant
        // "call it", reentrantly re-borrowing the same `RefCell` and
        // panicking with "RefCell already borrowed" instead of a clean
        // `TypeError` (confirmed via `operator.countOf(countOf, countOf)`
        // — a non-iterable `BuiltinFunction` passed to `iter()` — from
        // CPython's own `test_iter.py::test_countOf`).
        other => Err(PyError::type_error(format!(
            "'{}' object is not iterable",
            other.type_name()
        ))),
    }
}

/// `range_iterator.__setstate__(state)` — real CPython's pickle protocol
/// uses this to restore a saved iteration position (`state` = number of
/// items already consumed). Since `RangeIter.current` already tracks the
/// LIVE position (not the original start), this only produces the exactly
/// correct absolute position when called on a freshly-created iterator (the
/// only realistic real-world use — restoring right after `__reduce__`/
/// unpickling, before any `next()` calls) — advancing `current` by
/// `state * step` from wherever it currently sits. Found via CPython's own
/// `test_range.py::test_iterator_setstate`.
pub fn range_iter_setstate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__setstate__() takes exactly one argument",
        ));
    }
    let state = to_index(&args[1]).map_err(|_| {
        PyError::type_error(format!(
            "an integer is required (got type {})",
            args[1].borrow().type_name()
        ))
    })?;
    let mut obj = args[0].try_borrow_mut()?;
    if let PyObject::RangeIter { current, step, .. } = &mut *obj {
        *current += &state * &*step;
    }
    Ok(py_none())
}

/// `list_iterator.__setstate__(state)` — same protocol as `range_iterator`'s
/// above, but simpler: `ListIter.index` already IS the absolute position, so
/// this just sets it directly (clamped to the list's length, matching real
/// CPython's own clamping behavior for an out-of-range state).
pub fn list_iter_setstate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__setstate__() takes exactly one argument",
        ));
    }
    let state = to_index(&args[1]).map_err(|_| {
        PyError::type_error(format!(
            "an integer is required (got type {})",
            args[1].borrow().type_name()
        ))
    })?;
    let mut obj = args[0].try_borrow_mut()?;
    if let PyObject::ListIter { list, index } = &mut *obj {
        let n = state.to_usize().unwrap_or(0).min(list.len());
        *index = n;
    }
    Ok(py_none())
}

pub fn builtin_next(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("next() takes at least 1 argument"));
    }
    // Check for __next__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__next__"),
            PyObject::Generator { .. } => {
                drop(obj);
                let next_func = args[0].borrow().get_attribute("__next__")?;
                let (_n, f) = {
                    let b = next_func.borrow();
                    if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                        (name.clone(), *func)
                    } else {
                        return Err(PyError::runtime_error("expected __next__ method"));
                    }
                };
                let result = f(&[args[0].clone()]);
                // Convert raise StopIteration into PyError::StopIteration for next() protocol
                if let Err(ref e) = result {
                    if is_stop_iteration_error(e) {
                        // `next(gen, default)` — exhausted iterators return
                        // the default instead of raising StopIteration
                        // (difflib.py's `_line_iterator` relies on it via
                        // `next(diff_lines_iterator, 'X')`).
                        if args.len() >= 2 {
                            return Ok(args[1].clone());
                        }
                        return Err(PyError::StopIteration);
                    }
                }
                return result;
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        let result = call_bound_method(f, args[0].clone(), vec![]);
        // Convert raise StopIteration into PyError::StopIteration for next() protocol
        if let Err(PyError::Exception(_, ref exc)) = result {
            let is_stop = match &*exc.borrow() {
                PyObject::Exception { typ, .. } if typ == "StopIteration" => true,
                _ => false,
            };
            if is_stop {
                return Err(PyError::StopIteration);
            }
        }
        return result;
    }
    // Fallback to list-based iteration
    // Inline types (SmallInt etc.) and Imm types (Str, Tuple, Int, etc.)
    // are not iterable iterators — return TypeError without calling
    // borrow_mut on something that doesn't support it.
    match args[0] {
        PyObjectRef::SmallInt(_)
        | PyObjectRef::SmallBool(_)
        | PyObjectRef::SmallFloat(_)
        | PyObjectRef::SmallStr(_)
        | PyObjectRef::None
        | PyObjectRef::Imm(_) => {
            return Err(PyError::type_error(format!(
                "'{}' object is not an iterator",
                args[0].get_type_name()
            )));
        }
        _ => {}
    }
    // `GroupByIter` handled as its own, separate pre-check (not inside the
    // `match &mut *obj` below) because its advance logic must call
    // arbitrary Python code (the key function, `equals()` on keys) WITHOUT
    // holding this object's own `borrow_mut()` — otherwise a reentrant
    // `next()` on this SAME groupby object from within that callback
    // (real, deliberately adversarial CPython regression test:
    // `test_groupby_reentrant_eq_does_not_crash`, gh-143543) hits the exact
    // same double-borrow panic this restructuring exists to avoid. Extract
    // the state under a SHORT borrow, do all the scanning/calling with NO
    // borrow held at all, then a second SHORT borrow to write the result
    // back.
    let is_groupby = matches!(&*args[0].borrow(), PyObject::GroupByIter { .. });
    if is_groupby {
        let (source, key_func, mut pending, exhausted) = {
            let mut obj = args[0].borrow_mut();
            if let PyObject::GroupByIter {
                source,
                key_func,
                pending,
                exhausted,
            } = &mut *obj
            {
                (source.clone(), key_func.clone(), pending.take(), *exhausted)
            } else {
                unreachable!()
            }
        };
        if exhausted {
            return if args.len() >= 2 {
                Ok(args[1].clone())
            } else {
                Err(PyError::stop_iteration())
            };
        }
        let compute_key = |v: &PyObjectRef| -> PyResult<PyObjectRef> {
            match &key_func {
                Some(f) => call_bound_method(f.clone(), v.clone(), vec![]),
                None => Ok(v.clone()),
            }
        };
        // First item of this group: either carried over from the previous
        // call's lookahead, or freshly read from the source.
        let (this_key, first_val) = match pending.take() {
            Some((k, v)) => (k, v),
            None => match builtin_next(&[source.clone()]) {
                Ok(v) => {
                    let k = compute_key(&v)?;
                    (k, v)
                }
                Err(PyError::StopIteration) => {
                    // `next()` on a non-iterator must raise TypeError, not panic: only
                    // Mut-wrapped values are borrow_mut-able below, so an Imm/inline value
                    // (str, int, tuple, Function, ...) reaching this point would hit
                    // borrow_mut's "non-Mut value" panic instead (repro: `next('abc')`).
                    if !matches!(args[0], PyObjectRef::Mut(_)) {
                        return Err(PyError::type_error(format!(
                            "'{}' is not an iterator",
                            args[0].borrow().type_name()
                        )));
                    }
                    let mut obj = args[0].borrow_mut();
                    if let PyObject::GroupByIter { exhausted, .. } = &mut *obj {
                        *exhausted = true;
                    }
                    return if args.len() >= 2 {
                        Ok(args[1].clone())
                    } else {
                        Err(PyError::stop_iteration())
                    };
                }
                Err(e) => return Err(e),
            },
        };
        let mut group = vec![first_val];
        let mut new_pending = None;
        let mut new_exhausted = false;
        loop {
            match builtin_next(&[source.clone()]) {
                Ok(v) => {
                    let k = compute_key(&v)?;
                    if this_key.equals(&k)? {
                        group.push(v);
                    } else {
                        new_pending = Some((k, v));
                        break;
                    }
                }
                Err(PyError::StopIteration) => {
                    new_exhausted = true;
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        {
            let mut obj = args[0].borrow_mut();
            if let PyObject::GroupByIter {
                pending, exhausted, ..
            } = &mut *obj
            {
                *pending = new_pending;
                *exhausted = new_exhausted;
            }
        }
        return Ok(py_tuple(vec![
            this_key,
            PyObjectRef::new(PyObject::ListIter {
                list: group,
                index: 0,
            }),
        ]));
    }
    // Same reentrancy concern as `GroupByIter` just above: advancing this
    // needs to call the underlying object's own `__getitem__` (arbitrary
    // Python), so extract state under a short borrow, call with NO borrow
    // held, then a second short borrow to write the new index back.
    let getitem_state = {
        let obj = args[0].borrow();
        if let PyObject::GetItemIter { obj: inner, index } = &*obj {
            Some((inner.clone(), *index))
        } else {
            None
        }
    };
    let call_sentinel_state = {
        let obj = args[0].borrow();
        if let PyObject::CallSentinelIter {
            func,
            sentinel,
            exhausted,
        } = &*obj
        {
            Some((func.clone(), sentinel.clone(), *exhausted))
        } else {
            None
        }
    };
    if let Some((func, sentinel, exhausted)) = call_sentinel_state {
        if exhausted {
            return if args.len() >= 2 {
                Ok(args[1].clone())
            } else {
                Err(PyError::stop_iteration())
            };
        }
        let result = builtin_call(&func, &[])?;
        if result.equals(&sentinel)? {
            let mut obj = args[0].borrow_mut();
            if let PyObject::CallSentinelIter { exhausted, .. } = &mut *obj {
                *exhausted = true;
            }
            return if args.len() >= 2 {
                Ok(args[1].clone())
            } else {
                Err(PyError::stop_iteration())
            };
        }
        return Ok(result);
    }
    if let Some((inner, index)) = getitem_state {
        return match py_getitem(&inner, &py_int(index)) {
            Ok(v) => {
                let mut obj = args[0].borrow_mut();
                if let PyObject::GetItemIter { index, .. } = &mut *obj {
                    *index += 1;
                }
                Ok(v)
            }
            // Real Python accepts a Python-level `raise IndexError(...)`
            // from a custom `__getitem__` just as readily as this
            // interpreter's own native `PyError::IndexError` — not checking
            // the `PyError::Exception` form too meant a completely
            // ordinary `class C: def __getitem__(self, i): if i >= n: raise
            // IndexError` (the standard idiom) would propagate the
            // IndexError instead of stopping iteration.
            Err(ref e)
                if matches!(e, PyError::IndexError(_))
                    || matches!(e, PyError::Exception(_, exc) if matches!(&*exc.borrow(), PyObject::Exception { typ, .. } if crate::vm::is_exception_subclass(typ, "IndexError"))) =>
            {
                // Advance past the failed index so a subsequent append doesn't
                // make an already-exhausted iterator appear to have more items
                // (test_list::test_exhausted_iterator expects `exhit` to stay []
                // after `a.append(9)` while `empit` at the same index does see
                // the new item — the difference is that `exhit` has already
                // attempted and failed at that index, while `empit` hasn't).
                let mut obj = args[0].borrow_mut();
                if let PyObject::GetItemIter { index, .. } = &mut *obj {
                    *index += 1;
                }
                drop(obj);
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            }
            Err(e) => Err(e),
        };
    }
    let mut obj = args[0].borrow_mut();
    match &mut *obj {
        PyObject::List(v) => {
            if v.is_empty() {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                // Convert to ListIter for O(1) iteration
                let list = std::mem::take(v);
                *obj = PyObject::ListIter { list, index: 0 };
                drop(obj);
                let mut obj = args[0].borrow_mut();
                if let PyObject::ListIter { list, index } = &mut *obj {
                    let v = list[*index].clone();
                    *index += 1;
                    Ok(v)
                } else {
                    unreachable!()
                }
            }
        }
        PyObject::ListIter { list, index } => {
            if *index >= list.len() {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                let v = list[*index].clone();
                *index += 1;
                Ok(v)
            }
        }
        PyObject::CycleIter { items, index } => {
            if items.is_empty() {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                let v = items[*index % items.len()].clone();
                *index += 1;
                Ok(v)
            }
        }
        PyObject::EnumerateIter { source, pos, start } => {
            // Genuinely lazy — pulls one item from the underlying `source`
            // iterator per call instead of the OLD approach (a
            // pre-materialized `items: Vec<PyObjectRef>`, built by eagerly
            // draining the whole input up front in `builtin_enumerate`).
            // That eager drain hung forever on any genuinely infinite
            // iterable (`itertools.cycle(...)`, `itertools.count()` past
            // its own internal materialization cap) — confirmed via the
            // simplest repro, `enumerate(itertools.cycle([1,2,3]))`, which
            // never even got to yield its first pair.
            let idx = *start + *pos;
            *pos += 1;
            let source = source.clone();
            drop(obj);
            match builtin_next(&[source]) {
                Ok(val) => Ok(py_tuple(vec![py_int(idx as i64), val])),
                Err(PyError::StopIteration) => {
                    if args.len() >= 2 {
                        Ok(args[1].clone())
                    } else {
                        Err(PyError::stop_iteration())
                    }
                }
                Err(e) => Err(e),
            }
        }
        PyObject::MapIterator { func, iterator } => {
            let iter = iterator.as_ref().clone();
            let next = builtin_next(&[iter]);
            match next {
                Ok(val) => {
                    if func.borrow().type_name() == "NoneType" {
                        Ok(val)
                    } else {
                        let mapped = builtin_call(func, &[val])?;
                        Ok(mapped)
                    }
                }
                Err(e) => {
                    if args.len() >= 2 {
                        Ok(args[1].clone())
                    } else {
                        Err(e)
                    }
                }
            }
        }
        PyObject::FilterIterator { func, iterator } => {
            let iter = iterator.as_ref().clone();
            loop {
                let next = builtin_next(&[iter.clone()]);
                match next {
                    Ok(val) => {
                        // `filter(None, iterable)` keeps only the TRUTHY
                        // elements of `iterable` itself (equivalent to
                        // `filter(bool, iterable)`) — the previous
                        // `is_none() || call(...).truthy()` short-circuited
                        // to unconditionally `true` whenever `func` was
                        // `None`, silently keeping EVERY element (including
                        // falsy ones like `0`/`""`/`[]`) instead of
                        // filtering by truthiness at all.
                        let should_keep = if func.borrow().type_name() == "NoneType" {
                            val.truthy()
                        } else {
                            builtin_call(func, &[val.clone()])?.truthy()
                        };
                        if should_keep {
                            return Ok(val);
                        }
                    }
                    Err(e) => {
                        if args.len() >= 2 {
                            return Ok(args[1].clone());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        }
        PyObject::ZipIterator { iterators } => {
            let mut results = Vec::with_capacity(iterators.len());
            for it in iterators.iter() {
                match builtin_next(&[it.clone()]) {
                    Ok(val) => results.push(val),
                    Err(e) => {
                        if args.len() >= 2 {
                            return Ok(args[1].clone());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            Ok(py_tuple(results))
        }
        PyObject::RangeIter {
            current,
            stop,
            step,
        } => {
            let exhausted = (step.sign() == num_bigint::Sign::Plus && current >= stop)
                || (step.sign() == num_bigint::Sign::Minus && current <= stop);
            if exhausted {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                let v = py_int(current.clone());
                *current += &*step;
                Ok(v)
            }
        }
        PyObject::DequeIter {
            deque,
            index,
            start_len,
        } => {
            let native = crate::object::native_backing_of(deque).or_else(|| {
                let dq = deque.borrow();
                if matches!(&*dq, PyObject::Deque { .. }) {
                    Some(deque.clone())
                } else {
                    None
                }
            }).ok_or_else(|| PyError::runtime_error("deque iterator over non-deque"))?;
            let (is_done, maybe_item) = {
                let nb = native.borrow();
                if let PyObject::Deque { data, .. } = &*nb {
                    if data.len() != *start_len {
                        return Err(PyError::runtime_error("deque mutated during iteration"));
                    }
                    if *index >= data.len() {
                        (true, None)
                    } else {
                        (false, Some(data[*index].clone()))
                    }
                } else {
                    (true, None)
                }
            };
            if is_done {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else if let Some(v) = maybe_item {
                if let PyObject::DequeIter { index, .. } = &mut *obj {
                    *index += 1;
                }
                Ok(v)
            } else {
                Err(PyError::runtime_error("deque iterator over non-deque"))
            }
        }
        PyObject::DequeRevIter {
            deque,
            index,
            start_len,
        } => {
            let native = crate::object::native_backing_of(deque).or_else(|| {
                let dq = deque.borrow();
                if matches!(&*dq, PyObject::Deque { .. }) {
                    Some(deque.clone())
                } else {
                    None
                }
            }).ok_or_else(|| PyError::runtime_error("deque iterator over non-deque"))?;
            let (is_done, maybe_item) = {
                let nb = native.borrow();
                if let PyObject::Deque { data, .. } = &*nb {
                    if data.len() != *start_len {
                        return Err(PyError::runtime_error("deque mutated during iteration"));
                    }
                    if *index < 0 || (*index as usize) >= data.len() {
                        (true, None)
                    } else {
                        (false, Some(data[*index as usize].clone()))
                    }
                } else {
                    (true, None)
                }
            };
            if is_done {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else if let Some(v) = maybe_item {
                if let PyObject::DequeRevIter { index, .. } = &mut *obj {
                    *index -= 1;
                }
                Ok(v)
            } else {
                Err(PyError::runtime_error("deque iterator over non-deque"))
            }
        }
        _ => Err(PyError::type_error(format!(
            "'{}' is not an iterator",
            obj.type_name()
        ))),
    }
}


pub fn builtin_id(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("id() takes exactly one argument"));
    }
    Ok(py_int(args[0].get_id() as i64))
}

pub fn builtin_vars(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("vars() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Instance { dict, typ } => {
            // NormalDist and other __slots__ classes without __dict__ should
            // raise TypeError for vars() – CPython's `test_statistics`
            // expects `vars(NormalDist(...))` to fail. Our slotted instances
            // still carry a dict for attribute storage, so vars() would
            // incorrectly succeed. Check the type's __slots__.
            if let PyObject::Type { dict: tdict, .. } = &*typ.borrow() {
                if let Some(slots) = tdict.get_str("__slots__") {
                    let is_slots_without_dict = match &*slots.borrow() {
                        PyObject::Tuple(items) => !items.iter().any(|v| v.str() == "__dict__"),
                        PyObject::Str(s) => s != "__dict__",
                        _ => true,
                    };
                    if is_slots_without_dict {
                        return Err(PyError::type_error("vars() argument must have __dict__ attribute"));
                    }
                }
            }
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(k), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
        }
        PyObject::Module { dict, .. } => {
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(interner::lookup_str(*k)), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
        }
        _ => Err(PyError::type_error(format!(
            "vars() argument must have __dict__ attribute"
        ))),
    }
}

thread_local! {
    // `isinstance()`/`issubclass()` recurse in PLAIN RUST (calling
    // themselves directly, not through `call_function`) when the
    // `classinfo` argument is a tuple that itself contains tuples —
    // arbitrarily deeply, per real Python semantics (`isinstance(x, (a,
    // (b, (c, ...))))`). `vm.rs`'s `call_function` recursion-depth guard
    // (see its own doc comment) only covers actual Python-level function
    // calls, not this native-Rust recursion, so a deeply/infinitely nested
    // classinfo tuple had no depth limit at all — confirmed general via
    // CPython's own `test_isinstance.py`'s `blowstack()` helper, which
    // builds an ever-growing nested tuple in a `while True:` loop
    // expecting `RecursionError` to interrupt it "eventually": without a
    // guard here, it never does, hanging forever instead of failing fast.
    static ISINSTANCE_RECURSION_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

struct IsinstanceRecursionGuard;

impl IsinstanceRecursionGuard {
    fn enter() -> PyResult<Self> {
        let depth = ISINSTANCE_RECURSION_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        if depth > 1000 {
            ISINSTANCE_RECURSION_DEPTH.with(|c| c.set(c.get() - 1));
            return Err(PyError::recursion_error("maximum recursion depth exceeded"));
        }
        Ok(IsinstanceRecursionGuard)
    }
}

impl Drop for IsinstanceRecursionGuard {
    fn drop(&mut self) {
        ISINSTANCE_RECURSION_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

pub fn builtin_isinstance(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let trace = std::env::var("RPY_TRACE_IS").is_ok();
    if trace {
        let o_t = args[0].borrow().type_name();
        let c_t = args[1].borrow().type_name();
        let c_meta = crate::object::metatype_of(&args[1])
            .map(|m| m.borrow().type_name())
            .unwrap_or_else(|| "None".into());
        eprintln!("IS-ENTER obj={} class={} class_meta={}", o_t, c_t, c_meta);
    }
    if args.len() != 2 {
        return Err(PyError::type_error(
            "isinstance() takes exactly 2 arguments",
        ));
    }
    // `isinstance(x, int | str)` — a PEP 604 union used as the second
    // argument checks membership against ANY of its parts, same as the
    // tuple-of-types form just below (real CPython treats `X | Y` and
    // `(X, Y)` identically here). Checked before borrowing `args[1]` as
    // `class` below since `union_args` does its own borrow internally.
    if let Some(members) = crate::modules::union_args(&args[1]) {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in &members {
            // A union member can be the literal `None` singleton (`int |
            // None`, matching real CPython's own PEP 604 syntax) rather than
            // `NoneType` itself — `isinstance(x, None)` isn't meaningful
            // (`None` isn't a class), so check against `type(None)` instead.
            let t = if matches!(&*t.borrow(), PyObject::None) {
                builtin_type_of(&[py_none()])?
            } else {
                t.clone()
            };
            let check_args = vec![args[0].clone(), t];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let obj = args[0].borrow();
    let class = args[1].borrow();
    // A class with a custom `__instancecheck__` (collections.abc ABCs like
    // Hashable/Iterable/Sized) delegates the check to it.
    if let PyObject::Type { dict, .. } = &*class {
        if let Some(ic) = dict.get_str("__instancecheck__") {
            if !matches!(&*ic.borrow(), PyObject::None) {
                let result = call_bound_method(ic.clone(), args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
    }
    // Fallback: `__instancecheck__` defined on the class's METACLASS (the
    // standard CPython placement). The dict-level lookup above only fires
    // for the non-standard in-class placement; dynamically built protocol
    // /ABC classes (`_ProtocolMeta('X', (object,), ns)`) carry the hook on
    // their metaclass instead, and — because dynamic class creation with a
    // custom metaclass currently drops the provided namespace into the
    // new Type's dict — the metaclass MRO is the ONLY place the hook is
    // reachable from for them.
    if let Some(meta) = crate::object::metatype_of(&args[1]) {
        if std::env::var("RPY_TRACE_IS").is_ok() {
            eprintln!("IS-TRACE fallback: class={} meta={}",
                      class.type_name(), meta.borrow().type_name());
        }
        if !meta.is(&args[1]) {
            if let Some(f) = crate::object::lookup_dunder_via_mro(&meta, "__instancecheck__") {
                let result =
                    call_bound_method(f, args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
    }
    // Handle tuple of types: isinstance(x, (type1, type2, ...))
    if let PyObject::Tuple(types) = &*class {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    match (&*obj, &*class) {
        (
            PyObject::Type { .. },
            PyObject::Type {
                name: class_name, ..
            },
        ) => {
            // isinstance(SomeClass, X): every class is an instance of its
            // metaclass (a custom one if it was built via `metaclass=`,
            // otherwise plain `type` — e.g. `isinstance(Foo, type)` is
            // True for any ordinary class `Foo`). Walk the metaclass's own
            // mro for `X`, exactly like the Instance case above walks the
            // *class's* mro for ordinary instance checks. Deliberately
            // avoids fetching the canonical `type`/`object` singletons via
            // `with_vm_mut` for the no-custom-metaclass fallback below —
            // `isinstance()` runs deep inside live call chains constantly,
            // and grabbing a second aliasing `&mut VirtualMachine` there
            // reliably segfaulted in testing; a plain name comparison
            // (matching how BuiltinFunction-represented native bases are
            // already recognized elsewhere in this function) avoids it.
            if let Some(mt) = metatype_of(&args[0]) {
                if mt.is(&args[1]) {
                    return Ok(py_bool(true));
                }
                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                    for c in mro {
                        if c.is(&args[1]) {
                            return Ok(py_bool(true));
                        }
                    }
                }
                return Ok(py_bool(false));
            }
            Ok(py_bool(class_name == "type" || class_name == "object"))
        }
        (PyObject::Instance { typ, .. }, PyObject::Type { .. }) => {
            // isinstance(obj, Cls): Cls must appear in obj's OWN type's mro
            // — this used to walk `Cls`'s mro checking for `obj`'s exact
            // class name instead (backwards: comparing the wrong object's
            // ancestry, and by name instead of identity), so
            // `isinstance(subclass_instance, ParentClass)` was always False
            // unless the instance's class name happened to literally match
            // one of Cls's own ancestors' names. Confirmed via a minimal,
            // Django-free repro: `isinstance(CharField(), Field)` (a plain
            // one-level subclass check) returning False — this broke every
            // `isinstance(other, Field)`-style guard in real code (Django's
            // `Field.__lt__`/`__eq__`, used by `@total_ordering` and
            // `bisect`-based field ordering during model construction).
            // Direct identity check FIRST, independent of `mro` — a
            // properly-built class (via `default_build_class`) always has
            // itself as `mro[0]`, so this was previously redundant with
            // the mro walk below and easy to miss. But several ad-hoc
            // native `PyObject::Type`s built directly in Rust (this
            // session's own `Fraction`, `namedtuple`-generated classes,
            // `HTTPConnection`, ...) set `mro: vec![]` (empty) since they
            // aren't constructed through the real class-creation
            // machinery — for those, `isinstance(instance, ExactOwnType)`
            // was ALWAYS `False`, even for the most basic possible check,
            // since the type never appeared in its own (empty) mro at
            // all. Confirmed via `isinstance(Fraction(1,2), Fraction)` and
            // `isinstance(a_namedtuple_instance, ThatNamedtupleType)`,
            // both `False` before this fix.
            if typ.is(&args[1]) {
                return Ok(py_bool(true));
            }
            let typ_ref = typ.borrow();
            if let PyObject::Type { mro, .. } = &*typ_ref {
                for c in mro {
                    if c.is(&args[1]) {
                        return Ok(py_bool(true));
                    }
                }
            }
            drop(typ_ref);
            // `ABCMeta.register(subclass)`-style virtual subclass checks
            // (see `.register`'s own doc comment, `get_attribute`'s
            // `PyObject::Type` arm) — `class` (the ABC) records registered
            // classes in its own `_abc_registry` frozenset (read via
            // `own_abc_registry`, NOT `get_attribute` — see its own doc
            // comment for why inherited/MRO-walked registries are wrong
            // here); `obj`'s class (or any of ITS ancestors, for a real
            // subclass of a registered virtual-subclass root) counts too.
            if abc_registry_matches_in_subtree(&args[1], &|registered| {
                typ.is(registered)
                    || matches!(&*typ.borrow(), PyObject::Type { mro, .. } if mro.iter().any(|c| c.is(registered)))
            }) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Instance { typ, .. }, _) => {
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                _ => class.str(),
            };
            // `class Foo(list): ...` — Foo transparently subclasses a
            // native builtin, so isinstance(foo, list) must also be true.
            if let Some(native_kind) = native_base_of_type(typ) {
                if native_kind == class_name {
                    return Ok(py_bool(true));
                }
            }
            // `class MyError(Exception): ...` — MyError's instances must
            // also be `isinstance(x, Exception)` (and `AttributeError`,
            // `BaseException`, etc, for whatever it really derives from).
            // Builtin exception "classes" are `PyObject::BuiltinFunction`s
            // that never appear in a custom class's own `mro` (only real
            // `PyObject::Type` bases do), so the mro-walk arm just above
            // can't see this relationship at all — without this, NO custom
            // exception subclass was ever recognized as an instance of any
            // of its real (builtin) ancestors, which also broke `except
            // Exception:`/`except SomeBuiltinBase:` for literally any
            // user-defined exception class (CHECK_EXC_MATCH, `vm.rs`, and
            // `builtin_issubclass` below share this exact same gap and fix).
            if let PyObject::BuiltinFunction { .. } = &*class {
                if let Some(base_name) = find_exception_base_name(typ) {
                    if crate::vm::is_exception_subclass(&base_name, &class_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(
                typ.borrow().type_name() == class_name || class_name == "object",
            ))
        }
        // `isinstance(TypeError, type)` (and similar for any builtin
        // exception "class") — real CPython: every exception class's
        // metaclass is `type`, so this must be True. Since builtin
        // exception classes are represented as plain `PyObject::BuiltinFunction`s
        // here (not `PyObject::Type`s), the generic name-based hierarchy
        // check in the catch-all arm below instead compared this
        // function's own native `type_name()` ("builtin_function_or_method")
        // against "type" in the EXCEPTION hierarchy table (nonsensical —
        // that table is for exception ANCESTRY, not metaclass checks) and
        // always returned False. Real trigger: `unittest`'s own
        // `case.py`'s `_is_subtype`, `isinstance(expected, type) and
        // issubclass(expected, basetype)` — used by `assertRaises(SomeErr)`
        // to validate its argument — always failed, making
        // `self.assertRaises(TypeError)` (or ANY builtin exception class)
        // raise `TypeError: assertRaises() arg 1 must be an exception type
        // or tuple of exception types` instead of actually working.
        (PyObject::BuiltinFunction { name, .. }, PyObject::Type { name: cname, .. })
            if is_builtin_exception_class_name(name) =>
        {
            Ok(py_bool(cname == "type" || cname == "object"))
        }
        _ => {
            let obj_type = args[0].borrow().type_name();
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => class.str(),
            };
            // Direct type name match for built-in types (int, str, list, etc.)
            if obj_type == class_name {
                return Ok(py_bool(true));
            }
            // `bool` is real CPython's one primitive with an actual
            // inheritance relationship to another primitive
            // (`bool.__bases__ == (int,)`) — `isinstance(True, int)` and
            // `isinstance(True, object)` must both be `True`. A `bool`
            // value here is always the PRIMITIVE `PyObjectRef::SmallBool`/
            // `PyObject::Bool` shape (never a `PyObject::Instance`, since
            // `bool` can't be subclassed — see `default_build_class`'s
            // explicit block for that), so this can't be expressed via the
            // generic mro-walk arms above (which all key off an `Instance`'s
            // `typ`) — a narrow, direct name check is the simplest correct
            // fix, mirroring this function's existing style for other
            // primitive/name-based special cases.
            if obj_type == "bool" && (class_name == "int" || class_name == "object") {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s other `_abc_registry` fallback —
            // this one covers a PRIMITIVE (inline `SmallInt`/`SmallStr`/...
            // or boxed `Int`/`Str`/...) checked against an ABC that
            // registered the matching builtin type name (real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)` — needed for
            // `isinstance(5, numbers.Integral)` to work at all).
            if let PyObject::Type { .. } = &*class {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    registered_name == obj_type
                }) {
                    return Ok(py_bool(true));
                }
            }
            // Exception hierarchy — but only for objects that can actually
            // BE exceptions: real `PyObject::Exception` instances (builtin
            // or user-defined subclass), or builtin exception CLASS names.
            // `is_exception_subclass` maps unknown type names to `Exception`
            // by default (so user-defined `class MyError(Exception)` bodies
            // resolve correctly), which wrongly made ANY primitive value —
            // `isinstance('x', BaseException)`, `isinstance([], ValueError)`
            // — resolve through the exception table and return True. Real
            // trigger: `Lib/test/support/os_helper.py`'s `FakePath.__fspath__`
            // guards on `isinstance(self.path, BaseException)`, and a str
            // path incorrectly took the exception-raising branch, so every
            // `os.path.*`/`open()` call handed a `FakePath` failed.
            if matches!(&*obj, PyObject::Exception { .. })
                || is_builtin_exception_class_name(&obj_type)
            {
                return Ok(py_bool(crate::vm::is_exception_subclass(
                    &obj_type,
                    &class_name,
                )));
            }
            Ok(py_bool(false))
        }
    }
}

/// Real `open()`/`os.*` path arguments accept `str`, `bytes`, or any
/// `os.PathLike` — but `PyObjectRef::str()` on a `PyObject::Bytes` value
/// deliberately returns Python's own `repr()`-style `"b'...'"` form (matching
/// real CPython's `str(b"x")` semantics — NOT a bug on its own), which is
/// exactly the WRONG thing to feed to a filesystem call. Real trigger:
/// CPython's own `dbm/dumb.py` (via `os.fsencode`), which builds its
/// `.dat`/`.dir`/`.bak` filenames as `bytes` and passes them straight to
/// `io.open()` — `open(b"/tmp/x.dat", "w")` tried to open a file literally
/// named `b'/tmp/x.dat'` (quotes, `b` prefix and all), which obviously
/// doesn't exist, raising a confusing `OSError` instead of writing to the
/// intended path.
pub(crate) fn path_arg_to_string(obj: &PyObjectRef) -> String {
    // PEP 519 `os.PathLike` protocol: any object with a `__fspath__()`
    // method (real code: `pathlib.Path`, and plenty of test-only wrappers
    // like `Lib/test/support/os_helper.py`'s `FakePath`) must have THAT
    // called to get the real path, instead of falling through to
    // `.str()` — which, for a plain custom class instance with no
    // `__str__` override, produces its REPR (`<FakePath '/tmp/xxx'>`,
    // literally including the wrapper's own class name and quoting) rather
    // than the real path string, so every `open()`/`os.*` call given such
    // a wrapped path silently tried to open a nonexistent file named after
    // its own repr instead. Confirmed via `test_dbm.py::test_whichdb`,
    // which explicitly exercises `FakePath`-wrapped paths.
    let f = {
        let o = obj.borrow();
        if let PyObject::Instance { typ, .. } = &*o {
            lookup_dunder_via_mro(typ, "__fspath__")
        } else {
            None
        }
    };
    if let Some(f) = f {
        if let Ok(result) = call_bound_method(f, obj.clone(), vec![]) {
            return path_arg_to_string(&result);
        }
    }
    if let PyObject::Bytes(b) = &*obj.borrow() {
        String::from_utf8_lossy(b).into_owned()
    } else {
        obj.str()
    }
}

pub fn builtin_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "open() missing required argument 'file'",
        ));
    }
    // A keyword call (e.g. the extremely common `open(path, encoding="utf-8")`,
    // with NO explicit `mode`) reaches every plain `BuiltinFunction` with its
    // keywords packed into a dict APPENDED as the last positional arg (see
    // `vm.rs`'s `call_function`) — this was read directly as `args[1]` (the
    // "mode" position) whenever ANY keyword was passed, regardless of
    // whether `mode` itself was one of them. `dict.str()` on that packed
    // kwargs dict produced something like `"{'encoding': 'utf-8'}"`, which
    // contains none of 'r'/'w'/'a'/'+' — so NO read/write/append flag ever
    // got set, and the file open failed with a raw, confusing `OSError:
    // must specify at least one of read, write, or append access` instead
    // of just opening for reading (the real, correct default). Confirmed
    // via `test_baseexception.py::test_inheritance`'s own `open(path,
    // encoding="utf-8")` call. Now separates a trailing kwargs dict (if
    // present — `open()`'s real parameters are never legitimately a dict
    // themselves, so this is unambiguous) and reads `mode` from either the
    // second positional arg or the `mode` keyword, defaulting to `"r"`.
    let (pos_args, kwargs) = match args.last() {
        Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
            (&args[..args.len() - 1], Some(last))
        }
        _ => (args, None),
    };
    let filename = path_arg_to_string(&pos_args[0]);
    let mode = if pos_args.len() > 1 {
        pos_args[1].str()
    } else if let Some(kw) = kwargs {
        if let PyObject::Dict(d) = &*kw.borrow() {
            d.get(&py_str("mode"))
                .ok()
                .flatten()
                .map(|v| v.str())
                .unwrap_or_else(|| "r".to_string())
        } else {
            "r".to_string()
        }
    } else {
        "r".to_string()
    };
    // A trailing `+` ("r+"/"w+"/"a+", real CPython's "and updating" suffix)
    // means the file is opened for BOTH reading and writing — was
    // completely ignored here, so "rb+" (read-write, don't truncate, don't
    // create — the exact mode `dbm/dumb.py` uses to append new values to
    // its own data file) only ever opened for reading, and a subsequent
    // `f.write(...)` failed with a raw OS-level "Bad file descriptor"
    // instead of writing.
    let has_plus = mode.contains('+');
    // Mode `'x'` = exclusive create (real CPython's third create flag,
    // alongside `'w'` create+truncate and `'a'` create+append): creates the
    // file but FAILS with `FileExistsError` if it already exists. Was
    // completely unrecognized — `open(path, 'xb')` (the very common "write
    // this file only if I'm not clobbering something" idiom, used all over
    // CPython's own test suite) set NO read/write flag at all, failing with
    // a raw `OSError: must specify at least one of read, write, or append
    // access`. It implies write, exactly like `'w'`.
    let has_x = mode.contains('x');
    let mut opts = std::fs::File::options();
    opts.read(mode.contains('r') || has_plus)
        .write(mode.contains('w') || mode.contains('a') || has_plus || has_x)
        .append(mode.contains('a'))
        .create(mode.contains('w') || mode.contains('a') || has_x)
        .truncate(mode.contains('w'));
    if has_x {
        opts.create_new(true);
    }
    let file = opts
        .open(&filename)
        .map_err(|e| PyError::os_error_from_io(&e))?;
    let binary = mode.contains('b');
    Ok(PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(file)),
        name: filename,
        binary,
        pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        closed: false,
    }))
}


// Python-semantics modulo for `BigInt` (result takes the SIGN OF THE
// DIVISOR, unlike Rust's `%`, which takes the sign of the dividend) — needed
// by `builtin_pow`'s 3-arg form below, whose test coverage explicitly checks
// negative moduli (`test_pow.py::test_negative_exponent` sweeps `m` from
// -50 to 49).

pub fn builtin_reversed(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("reversed() takes exactly one argument"));
    }
    // Check type with a short-lived borrow to avoid holding the RefCell
    // borrow while iterating (which could trigger borrow_mut conflicts).
    // `range` needs its own O(1) case (real CPython's `range.__reversed__`)
    // — without this it fell into the generic "drain every element into a
    // Vec, then reverse" fallback further down, which for a `range` spanning
    // billions of elements tries to materialize the WHOLE thing first. Same
    // unbounded-incremental-growth bug as the `list()`/`list * n` memory
    // bombs fixed elsewhere (confirmed via CPython's own `test_range.py`,
    // `test_range_iterators`, whose `reversed(range(start, end, step))`
    // calls span ranges up to ~2**33 elements — enough to consume all
    // available RAM before ever finishing). `range`'s length is always
    // O(1) to compute, so the reversed sequence can be derived directly,
    // arithmetically, without ever iterating the original.
    {
        let obj = args[0].borrow();
        if let PyObject::Range { start, stop, step } = &*obj {
            let (start, stop, step) = (start.clone(), stop.clone(), step.clone());
            let empty = (step.sign() == num_bigint::Sign::Plus && start >= stop)
                || (step.sign() == num_bigint::Sign::Minus && start <= stop);
            if empty {
                return Ok(PyObjectRef::new(PyObject::RangeIter {
                    current: num_bigint::BigInt::from(0),
                    stop: num_bigint::BigInt::from(0),
                    step: num_bigint::BigInt::from(1),
                }));
            }
            let raw_len = &stop - &start;
            let q = &raw_len / &step;
            let count = if (&raw_len % &step).sign() != num_bigint::Sign::NoSign {
                q.abs() + 1
            } else {
                q.abs()
            };
            let last = &start + (&count - 1) * &step;
            let new_stop = &start - &step;
            return Ok(PyObjectRef::new(PyObject::RangeIter {
                current: last,
                stop: new_stop,
                step: -step,
            }));
        }
    }
    let kind = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(_) => 1,
            PyObject::Tuple(_) => 2,
            PyObject::Str(_) => 3,
            _ => 0,
        }
    };
    if kind != 0 {
        let obj = args[0].borrow();
        return match &*obj {
            PyObject::List(v) => {
                let mut rev = v.clone();
                rev.reverse();
                // A GetItemIter (no __len__), NOT a ListIter: real CPython's
                // reversed-list iterator has no `__len__`, so
                // `len(reversed([1,2,3]))` must raise TypeError
                // (test_list::test_reversed).
                Ok(PyObjectRef::new(PyObject::GetItemIter {
                    obj: PyObjectRef::new(PyObject::List(rev)),
                    index: 0,
                }))
            }
            PyObject::Tuple(v) => {
                let mut rev = v.clone();
                rev.reverse();
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: rev,
                    index: 0,
                }))
            }
            PyObject::Str(s) => {
                let chars: Vec<PyObjectRef> =
                    s.chars().rev().map(|c| py_str(&c.to_string())).collect();
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: chars,
                    index: 0,
                }))
            }
            _ => unreachable!(),
        };
    }
    // Native-backed sequence subclasses (e.g. `class A(tuple): ...`) store
    // their value in `__native__` and have no explicit `__len__`/
    // `__getitem__` in the type's MRO (tuple's own dunders are not in the
    // type dict). `builtin_len` already falls back to the native backing,
    // but the `lookup_dunder_via_mro` check below would still fail and raise
    // "argument to reversed() must be a sequence" — real trigger:
    // `test_tuple.py::test_free_after_iterating` which does `reversed(A())`
    // where `A` is a tuple subclass.
    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
        if let Some(native) = dict.get(crate::object::NATIVE_BACKING_KEY).cloned() {
            let kind2 = {
                let nb = native.borrow();
                match &*nb {
                    PyObject::List(_) => 1,
                    PyObject::Tuple(_) => 2,
                    PyObject::Str(_) => 3,
                    _ => 0,
                }
            };
            if kind2 != 0 {
                let nb = native.borrow();
                return match &*nb {
                    PyObject::List(v) => {
                        let mut rev = v.clone();
                        rev.reverse();
                        Ok(PyObjectRef::new(PyObject::GetItemIter {
                            obj: PyObjectRef::new(PyObject::List(rev)),
                            index: 0,
                        }))
                    }
                    PyObject::Tuple(v) => {
                        let mut rev = v.clone();
                        rev.reverse();
                        Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }))
                    }
                    PyObject::Str(s) => {
                        let chars: Vec<PyObjectRef> =
                            s.chars().rev().map(|c| py_str(&c.to_string())).collect();
                        Ok(PyObjectRef::new(PyObject::ListIter { list: chars, index: 0 }))
                    }
                    _ => unreachable!(),
                };
            }
        }
    }
    // Native deque: reversed(deque) returns a dedicated reverse iterator
    // (deque_reverse_iterator) that is itself callable with a deque argument
    // (test_deque::test_reversed_new: klass = type(reversed(deque())) ;
    // list(klass(deque(s))) == list(reversed(s))). The generic drain fallback
    // below would instead return a list_iterator which is not callable with a
    // deque and fails that test.
    {
        let is_deque_native = matches!(&*args[0].borrow(), PyObject::Deque { .. });
        if is_deque_native {
            let (deque, start_len) = {
                let b = args[0].borrow();
                if let PyObject::Deque { data, .. } = &*b {
                    (args[0].clone(), data.len())
                } else {
                    unreachable!()
                }
            };
            let idx = if start_len == 0 { -1 } else { (start_len as isize) - 1 };
            return Ok(PyObjectRef::new(PyObject::DequeRevIter {
                deque,
                index: idx,
                start_len,
            }));
        }
        // Deque subclass (Instance with native backing == Deque) — same behavior
        // as the native case above (reversed(DequeSubclass(...)) must also be
        // a deque_reverse_iterator, not a generic list_iterator).
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            if let Some(native) = dict.get(crate::object::NATIVE_BACKING_KEY).cloned() {
                if matches!(&*native.borrow(), PyObject::Deque { .. }) {
                    let start_len = {
                        let nb = native.borrow();
                        if let PyObject::Deque { data, .. } = &*nb {
                            data.len()
                        } else {
                            0
                        }
                    };
                    let idx = if start_len == 0 { -1 } else { (start_len as isize) - 1 };
                    return Ok(PyObjectRef::new(PyObject::DequeRevIter {
                        deque: args[0].clone(),
                        index: idx,
                        start_len,
                    }));
                }
            }
        }
    }
    // Real Python's `reversed(obj)` protocol for a plain instance (no native
    // fast path above): use `obj.__reversed__()` if defined, else `obj[len(
    // obj)-1]`, `obj[len(obj)-2]`, ..., `obj[0]` via `__len__`+`__getitem__`
    // — NEVER by draining a FORWARD iterator and reversing the result
    // (the previous fallback below, which this replaces for the Instance
    // case). That forward-drain approach only happens to work for
    // `__iter__`-based objects with a genuine end; for a `__len__`+
    // `__getitem__`-only object whose `__getitem__` never raises `IndexError`
    // for an out-of-range index (a real, deliberate CPython regression
    // test's own `Seq` class: `__getitem__` unconditionally `return
    // index` — CPython's `reversed()` never needs `IndexError` from such an
    // object since it's bounded by `__len__` instead), forward-draining
    // hangs FOREVER. Found via `test_enumerate.py`'s `TestReversed.test_gc`
    // — this only started hanging once `builtin_iter`'s own new `__getitem__`
    // fallback (see `GetItemIter`) made `iter()` succeed on such objects at
    // all, where it previously raised a quick (if wrong) `TypeError`.
    let instance_typ = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
        Some(typ.clone())
    } else {
        None
    };
    if let Some(typ) = &instance_typ {
        if let Some(f) = lookup_dunder_via_mro(typ, "__reversed__") {
            // `__reversed__ = None` is real Python's documented way to
            // explicitly DISABLE reversal on a class that would otherwise
            // qualify via `__len__`/`__getitem__` — must raise `TypeError`
            // outright (matching real CPython, and `test_enumerate.py`'s
            // own `TestReversed.test_objmethods::Blocked` class), not fall
            // through to the `__len__` fallback below (which `Blocked`
            // would otherwise satisfy) or try calling `None` as a function
            // (not callable — previously produced a confusing unrelated
            // error instead of a clean `TypeError`).
            if matches!(&*f.borrow(), PyObject::None) {
                return Err(PyError::type_error(format!(
                    "'{}' object is not reversible",
                    get_type_name_for_instance(typ)
                )));
            }
            return call_bound_method(f, args[0].clone(), vec![]);
        }
        // Real Python's `reversed()` fallback (no `__reversed__`) STRICTLY
        // requires `__len__` — it does NOT support the same "call
        // `__getitem__` until `IndexError`" protocol forward iteration
        // does. An object with `__getitem__` but no `__len__` (real
        // trigger: `test_enumerate.py`'s own `TestReversed.test_objmethods`,
        // `class NoLen: def __getitem__(self, i): return 1`) must raise
        // `TypeError` here, NOT fall through to the generic "unknown type:
        // drain via iteration" path below — that path now succeeds (via
        // `GetItemIter`) but drains FOREVER for an object whose
        // `__getitem__` never raises `IndexError` for any index (which
        // `reversed()` never needed to rely on in the first place, since
        // real CPython bounds the count via `__len__` instead).
        return if lookup_dunder_via_mro(typ, "__len__").is_some()
            && lookup_dunder_via_mro(typ, "__getitem__").is_some()
        {
            let len = builtin_len(&[args[0].clone()])?
                .as_i64()
                .ok_or_else(|| PyError::type_error("__len__() should return an int"))?;
            let mut v = Vec::with_capacity(len.max(0) as usize);
            let mut i = len - 1;
            while i >= 0 {
                v.push(py_getitem(&args[0], &py_int(i))?);
                i -= 1;
            }
            Ok(PyObjectRef::new(PyObject::ListIter { list: v, index: 0 }))
        } else {
            Err(PyError::type_error(
                "argument to reversed() must be a sequence",
            ))
        };
    }
    // Unknown type: drain via iteration (no active borrow on args[0])
    let mut v = Vec::new();
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => v.push(val),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    v.reverse();
    Ok(PyObjectRef::new(PyObject::ListIter { list: v, index: 0 }))
}

pub fn builtin_issubclass(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error(
            "issubclass() takes exactly 2 arguments",
        ));
    }
    // `issubclass(cls, int | str)` — same PEP 604 union-membership check as
    // `builtin_isinstance`'s matching case just above.
    if let Some(members) = crate::modules::union_args(&args[1]) {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in &members {
            let t = if matches!(&*t.borrow(), PyObject::None) {
                builtin_type_of(&[py_none()])?
            } else {
                t.clone()
            };
            let check_args = vec![args[0].clone(), t];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    // Custom __subclasscheck__ (e.g. os.PathLike virtual subclass)
    {
        let maybe_sc = {
            let base_b = args[1].borrow();
            if let PyObject::Type { dict, .. } = &*base_b {
                dict.get_str("__subclasscheck__").cloned()
            } else {
                None
            }
        };
        if let Some(sc) = maybe_sc {
            if !matches!(&*sc.borrow(), PyObject::None) {
                let result = call_bound_method(sc, args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
        if let Some(meta) = crate::object::metatype_of(&args[1]) {
            if !meta.is(&args[1]) {
                if let Some(f) = crate::object::lookup_dunder_via_mro(&meta, "__subclasscheck__") {
                    let result = call_bound_method(f, args[1].clone(), vec![args[0].clone()])?;
                    return Ok(result);
                }
            }
        }
    }
    // Handle tuple of types: issubclass(cls, (type1, type2, ...))
    let base = args[1].borrow();
    if let PyObject::Tuple(types) = &*base {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let cls = args[0].borrow();
    drop(base);
    let base = args[1].borrow();
    match (&*cls, &*base) {
        (PyObject::Type { mro: cls_mro, .. }, PyObject::Type { .. }) => {
            // Direct identity check first — same fix, same reason, as
            // `builtin_isinstance`'s matching arm just above: several
            // ad-hoc native `PyObject::Type`s (`Fraction`, `namedtuple`-
            // generated classes, ...) have an empty `mro`, so
            // `issubclass(X, X)` was `False` even for the most basic
            // possible check.
            if args[0].is(&args[1]) {
                return Ok(py_bool(true));
            }
            let base_tn = base.type_name();
            for c in cls_mro {
                if c.borrow().type_name() == base_tn {
                    return Ok(py_bool(true));
                }
            }
            // See `builtin_isinstance`'s matching fallback for
            // `ABCMeta.register`-style virtual subclass checks.
            if abc_registry_matches_in_subtree(&args[1], &|registered| {
                cls_mro.iter().any(|c| c.is(registered))
            }) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Type { mro: cls_mro, .. }, _) => {
            // Non-Type second argument: compare by name. `.str()` on a
            // BuiltinFunction (how builtin exception "classes" like
            // `Exception` are represented) returns its full repr
            // (`<built-in function Exception>`), not the bare name — must
            // extract that explicitly or every name comparison below
            // (including the exception-ancestry fix just past the mro
            // walk) silently never matches.
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                _ => base.str(),
            };
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            for c in cls_mro {
                if c.borrow().type_name() == base_name {
                    return Ok(py_bool(true));
                }
            }
            // `issubclass(MyError, Exception)` where MyError is a real
            // user-defined `class MyError(Exception): ...` — Exception (a
            // `BuiltinFunction`, not a `Type`) never appears in MyError's own
            // `mro`, so the walk above can't see this. Same gap/fix as
            // isinstance()'s Instance/BuiltinFunction arm just above.
            if matches!(&*base, PyObject::BuiltinFunction { .. }) {
                if let Some(base_exc_name) = find_exception_base_name(&args[0]) {
                    if crate::vm::is_exception_subclass(&base_exc_name, &base_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(false))
        }
        // Built-in exception "classes" (e.g. KeyError, ValueError) are
        // represented as BuiltinFunction constructors rather than Type
        // objects — resolve ancestry by name via the same table `except`
        // and isinstance() use, instead of only accepting real Type values.
        // A BuiltinFunction that is NOT a recognized exception class is not
        // a class at all (real CPython: issubclass(abs, X) raises TypeError)
        // — without this gate, `is_exception_subclass`'s catch-all mapped
        // ANY BuiltinFunction name (abs, print, ...) to Exception, so
        // `issubclass(abs, BaseException)` returned True.
        (PyObject::BuiltinFunction { name: cls_name, .. }, _) => {
            if !is_builtin_exception_class_name(cls_name) {
                return Err(PyError::type_error("issubclass() arg 1 must be a class"));
            }
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => base.str(),
            };
            // Everything (including every exception "class") is a subclass of
            // `object` — `issubclass(Exception, object)` must be True
            // (test_baseexception::test_builtins_new_style).
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            if crate::vm::is_exception_subclass(cls_name, &base_name) {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s `_abc_registry` fallback — this
            // covers `issubclass(int, numbers.Integral)`-style checks
            // where the SUBCLASS side (`int`/`float`/`complex`/...) is
            // itself a `BuiltinFunction`, not a `Type`. Real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)`.
            if let PyObject::Type { .. } = &*base {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    &registered_name == cls_name
                }) {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_bool(false))
        }
        // `Opcode::WITH_EXIT` (`vm.rs`) resolves a raised `PyObject::Exception`'s
        // `exc_type` argument to `__exit__` by looking its `typ` name up in the
        // CURRENT frame's builtins — which only ever holds the ~70 core
        // exception names (`add_exc_type!` in `create_builtins`), never a
        // module-scoped custom exception like `struct.error`/`pickle.PickleError`
        // (those live only in their own module's dict, not global builtins).
        // When that lookup misses, it falls back to a bare `PyObject::Str`
        // holding just the name, as a last-resort placeholder — so
        // `issubclass(exc_type, struct.error)` inside `unittest`'s own
        // `_AssertRaisesBaseContext.__exit__` reached here with a plain string
        // for `cls`. This codebase already treats built-in/module exception
        // "classes" as interchangeable by name everywhere else (the
        // `BuiltinFunction`/`Type`/`Str` arms just above), so extending that
        // same name-based comparison to a bare string `cls` here is consistent
        // for that INTERNAL fallback specifically — but a real user calling
        // `issubclass("hello", BaseException)` must still get real Python's
        // `TypeError: issubclass() arg 1 must be a class` (previously this
        // arm accepted ANY string unconditionally, so a plain string that
        // merely happened to arrive here — confirmed via CPython's own
        // `test_baseexception.py`, whose `test_inheritance`/`test_catch_string`
        // pass arbitrary strings including plain object values from
        // `builtins.__dict__` — silently returned `False`/`True` by name
        // comparison instead of raising). Gated on `cls_name` actually being
        // one of the recognized builtin/module exception names — every
        // legitimate internal fallback string is drawn from exactly that set
        // (`add_exc_type!`'s own names, or a module exception registered via
        // `is_builtin_exception_class_name`), so an unrecognized string can
        // only be genuine user input, not this internal fallback.
        (PyObject::Str(cls_name), _) if is_builtin_exception_class_name(cls_name) => {
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => base.str(),
            };
            Ok(py_bool(crate::vm::is_exception_subclass(
                cls_name, &base_name,
            )))
        }
        _ => {
            if std::env::var("RPY_DEBUG_ISSUBCLASS").is_ok() {
                eprintln!(
                    "issubclass() FAIL: arg0={:?}/{} arg1={:?}/{}",
                    cls.type_name(),
                    cls.repr(),
                    base.type_name(),
                    base.repr()
                );
            }
            Err(PyError::type_error("issubclass() arg 1 must be a class"))
        }
    }
}

pub fn builtin_help(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        println!("Welcome to RustPython 0.1.0!");
        println!();
        println!("Available built-in functions:");
        println!("  abs()  all()  any()  ascii()  bin()  bool()  breakpoint()");
        println!("  bytearray()  bytes()  callable()  chr()  compile()  delattr()");
        println!("  dict()  dir()  divmod()  enumerate()  eval()  exec()  exit()");
        println!("  filter()  float()  format()  frozenset()  getattr()  globals()");
        println!("  hasattr()  hash()  help()  hex()  id()  input()  int()");
        println!("  isinstance()  issubclass()  iter()  len()  list()  locals()");
        println!("  map()  max()  memoryview()  min()  next()  object()  oct()");
        println!("  open()  ord()  pow()  print()  property()  range()  repr()");
        println!("  reversed()  round()  set()  setattr()  slice()  sorted()");
        println!("  staticmethod()  str()  sum()  super()  tuple()  type()  vars()");
        println!("  zip()");
        println!();
        println!("Available error types:");
        println!("  BaseException  Exception  TypeError  ValueError  ZeroDivisionError");
        println!("  NameError  AttributeError  IndexError  KeyError  RuntimeError");
        println!("  StopIteration  AssertionError  OSError  ImportError  LookupError");
        println!("  ArithmeticError  OverflowError  NotImplementedError  RecursionError");
        println!("  KeyboardInterrupt  SystemExit  ModuleNotFoundError  FileNotFoundError");
        println!("  PermissionError  UnicodeDecodeError  UnicodeEncodeError");
        println!();
        println!("Type help(object) for information about a specific object.");
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Type { name, dict, .. } => {
                println!("Help on class {}:", name);
                if let Some(doc) = dict.get_str("__doc__") {
                    println!("  {}", doc.str());
                }
                println!();
                println!("Methods:");
                for (key, val) in dict.iter() {
                    if matches!(
                        &*val.borrow(),
                        PyObject::Function(_) | PyObject::BuiltinFunction { .. }
                    ) {
                        println!("  {}()", interner::lookup_str(*key));
                    }
                }
            }
            PyObject::Function(ref f) => {
                let name = &f.code.name;
                let dict = &f.dict;
                println!("Help on function {}:", name);
                if let Some(doc) = dict.get("__doc__") {
                    println!("  {}", doc.str());
                }
            }
            PyObject::BuiltinFunction { name, .. } => {
                println!("Help on built-in function {}:", name);
            }
            _ => {
                println!("Help on {}:", obj.type_name());
                println!("  Type: {}", obj.type_name());
            }
        }
    }
    Ok(py_none())
}
