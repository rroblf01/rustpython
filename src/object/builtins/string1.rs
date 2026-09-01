// Split out of the former monolithic object/builtins.rs — this file holds
// string/bytes/float/complex conversion builtins (`str`, `repr`, `bool`,
// `float`, `complex`, `format`) and their helpers.
use super::*;

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
pub(crate) fn python_bytes_repr(b: &[u8]) -> String {
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
