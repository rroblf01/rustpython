// Split out of the former monolithic object/builtins.rs — this file holds
// `int` builtin family (`int()`, `int.from_bytes`) and its helpers.
use super::*;

/// True iff `c` is a valid digit character for the given base.
fn is_base_digit_char(c: char, base: u32) -> bool {
    match c {
        '0'..='9' => (c as u32 - '0' as u32) < base,
        'a'..='z' => (c as u32 - 'a' as u32 + 10) < base,
        'A'..='Z' => (c as u32 - 'A' as u32 + 10) < base,
        _ => false,
    }
}

/// Maps a Unicode DECIMAL digit (any Nd block: Devanagari १२३, Arabic-Indic
/// ١٢٣, Tamil, Khmer, Fullwidth, ...) to its ASCII equivalent — `int('१२३')`
/// == 123 in CPython. Rust's `char::to_digit` only handles ASCII, so the Nd
/// blocks are listed explicitly.
fn unicode_decimal_digit(c: char) -> Option<char> {
    let cp = c as u32;
    const BLOCKS: [u32; 40] = [
        0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66,
        0x0CE6, 0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946,
        0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0,
        0xA9F0, 0xAA50, 0xABF0, 0xFF10, 0x104A0, 0x11066, 0x110F0,
    ];
    for &start in &BLOCKS {
        if cp >= start && cp < start + 10 {
            return char::from_u32('0' as u32 + (cp - start));
        }
    }
    None
}

/// Shared `int()` string/bytes parsing: trims, strips underscores, splits the
/// sign, detects `0x`/`0o`/`0b` prefixes when no base is given, and parses.
/// `repr_str` is the `%r`-style rendering used in the ValueError message
/// (`'½'` for a str, `b'123\x00'` for bytes).
fn int_from_digit_string(
    s: &str,
    base_obj: Option<&PyObjectRef>,
    repr_str: &str,
) -> PyResult<PyObjectRef> {
    let s_trim = s.trim();
    // Normalize Unicode decimal digits (१२३ -> 123, any Nd category) to
    // ASCII so parsing/validation see plain digits; everything else is kept
    // for the underscore validation and prefix detection below.
    let s_norm: String = s_trim
        .chars()
        .map(|c| unicode_decimal_digit(c).unwrap_or(c))
        .collect();
    let s_clean: String = s_norm.chars().filter(|&c| c != '_').collect();
    let (sign, body) = match s_clean.as_bytes().first() {
        Some(b'-') => (-1, &s_clean[1..]),
        Some(b'+') => (1, &s_clean[1..]),
        _ => (1, &s_clean[..]),
    };
    let parse = |body: &str, base: u32| -> Option<PyObjectRef> {
        // An empty digit body ('0x' with no digits) is invalid — BigInt's
        // parser would treat "" as 0.
        if body.is_empty() {
            return None;
        }
        if let Ok(n) = i64::from_str_radix(body, base) {
            return Some(py_int(sign * n));
        }
        BigInt::parse_bytes(body.as_bytes(), base).map(|n| py_int(if sign < 0 { -n } else { n }))
    };
    let make_err = |base: i64| {
        PyError::value_error(format!(
            "invalid literal for int() with base {}: {}",
            base, repr_str
        ))
    };
    // Pick the digit body, the effective base (explicit base, base 0 =
    // prefix detection, or the default prefix detection / decimal), and
    // whether the body came from auto-detected prefix/octal detection.
    let (body2, eff_base, was_auto_detect, had_prefix): (&str, i64, bool, bool) = match base_obj {
        Some(base_val) => {
            // The base accepts anything indexable (int('101',
            // base=MyIndexable(2)) == 5).
            let base = match to_index(base_val) {
                Ok(n) => match n.to_i64() {
                    Some(n) => n,
                    // A base too large for i64 (e.g. 2**100) is out of range.
                    None => return Err(PyError::value_error("int() base must be >= 2 and <= 36")),
                },
                Err(_) => return Err(PyError::type_error("int() base must be an integer")),
            };
            if !(base == 0 || (2..=36).contains(&base)) {
                return Err(PyError::value_error("int() base must be >= 2 and <= 36"));
            }
            if base == 0 {
                if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
                    (oct, 8, true, true)
                } else if let Some(hex) =
                    body.strip_prefix("0x").or_else(|| body.strip_prefix("0X"))
                {
                    (hex, 16, true, true)
                } else if let Some(bin) =
                    body.strip_prefix("0b").or_else(|| body.strip_prefix("0B"))
                {
                    (bin, 2, true, true)
                } else {
                    (body, 10, true, false)
                }
            } else {
                let stripped = if base == 16 {
                    body.strip_prefix("0x").or_else(|| body.strip_prefix("0X"))
                } else if base == 8 {
                    body.strip_prefix("0o").or_else(|| body.strip_prefix("0O"))
                } else if base == 2 {
                    body.strip_prefix("0b").or_else(|| body.strip_prefix("0B"))
                } else {
                    None
                };
                let had = stripped.is_some();
                (stripped.unwrap_or(body), base, false, had)
            }
        }
        None => {
            // No base given: prefix detection, but NO "old octal" rule —
            // int('010') is decimal 10, only int('010', 0) is an error.
            if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
                (oct, 8, false, true)
            } else if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
                (hex, 16, false, true)
            } else if let Some(bin) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
                (bin, 2, false, true)
            } else {
                (body, 10, false, false)
            }
        }
    };
    // Underscore placement: one underscore may follow a base prefix
    // ('0x_1'), but otherwise underscores must sit between two valid digits
    // (no leading/trailing/double). Validated on the ORIGINAL string, not
    // the underscore-stripped body.
    {
        let orig = s_norm.trim_start_matches(|c: char| c == '+' || c == '-');
        let (orig, had_prefix) = if let Some(r) = orig
            .strip_prefix("0x")
            .or_else(|| orig.strip_prefix("0X"))
            .or_else(|| orig.strip_prefix("0o"))
            .or_else(|| orig.strip_prefix("0O"))
            .or_else(|| orig.strip_prefix("0b"))
            .or_else(|| orig.strip_prefix("0B"))
        {
            (r, true)
        } else {
            (orig, false)
        };
        let orig = if had_prefix {
            orig.strip_prefix('_').unwrap_or(orig)
        } else {
            orig
        };
        if !orig.is_empty() {
            let bytes = orig.as_bytes();
            let mut prev_underscore = false;
            for (i, &c) in bytes.iter().enumerate() {
                if c == b'_' {
                    if prev_underscore {
                        return Err(make_err(eff_base));
                    }
                    let prev_ok =
                        i > 0 && is_base_digit_char(bytes[i - 1] as char, eff_base as u32);
                    let next_ok = i + 1 < bytes.len()
                        && is_base_digit_char(bytes[i + 1] as char, eff_base as u32);
                    if !(prev_ok && next_ok) {
                        return Err(make_err(eff_base));
                    }
                    prev_underscore = true;
                } else {
                    prev_underscore = false;
                }
            }
        }
    }
    let result = parse(body2, eff_base as u32).ok_or_else(|| make_err(eff_base))?;
    // Auto-detected "old octal" (leading 0 with no valid prefix — '0_100',
    // '010'): only valid if the value is zero.
    if was_auto_detect && eff_base == 10 && body.starts_with('0') && body.len() > 1 {
        let is_zero = matches!(&*result.borrow(), PyObject::Int(i) if i.is_zero());
        if !is_zero {
            return Err(make_err(0));
        }
    }
    Ok(result)
}

/// CPython's str->int digit limit: enforced for decimal and non-power-of-2
/// bases (base 3/36 hit it; base 2/4/8/16/32 use a fast binary path that
/// skips it). Counts DIGITS, excluding underscores and the sign.
pub(crate) fn check_int_str_digit_limit(s: &str, base_obj: Option<&PyObjectRef>) -> PyResult<()> {
    #[cfg(not(feature = "no_int_str_limit"))]
    {
        let power_of_two = base_obj
            .as_ref()
            .and_then(|b| to_index(b).ok())
            .and_then(|n| n.to_u64())
            .map(|n| n >= 2 && n & (n - 1) == 0)
            .unwrap_or(false);
        if !power_of_two {
            let limit = INT_MAX_STR_DIGITS.with(|d| d.get());
            if limit > 0 {
                // Count DIGITS, ignoring the sign and surrounding whitespace.
                let digit_len = s
                    .trim()
                    .trim_start_matches(|c: char| c == '+' || c == '-')
                    .chars()
                    .filter(|&c| c != '_')
                    .count();
                if digit_len > limit as usize {
                    return Err(PyError::value_error(format!(
                        "Exceeds the limit ({} digits) for integer string conversion; use sys.set_int_max_str_digits()", limit
                    )));
                }
            }
        }
    }
    Ok(())
}

pub fn builtin_int(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_int(0));
    }
    // Keyword arguments: `x` is positional-only and `base` is the only
    // accepted keyword — int(x=1.2) is "int() got an unexpected keyword
    // argument 'x'", int(base=10) is "int() missing string argument".
    let mut args_vec = args.to_vec();
    let mut base_obj: Option<PyObjectRef> = None;
    {
        let last_is_dict = args
            .last()
            .map(|a| matches!(&*a.borrow(), PyObject::Dict(_)))
            .unwrap_or(false);
        if last_is_dict {
            let last_borrow = args.last().unwrap().borrow();
            let pd = match &*last_borrow {
                PyObject::Dict(d) => d,
                _ => unreachable!(),
            };
            if pd.get(&py_str("x")).ok().flatten().is_some() {
                return Err(PyError::type_error(
                    "int() got an unexpected keyword argument 'x'",
                ));
            }
            base_obj = pd.get(&py_str("base")).ok().flatten();
            if args.len() == 1 {
                // kwargs-only call (int(base=10)): no positional x.
                if base_obj.is_some() {
                    return Err(PyError::type_error("int() missing string argument"));
                }
                // A single dict with neither x nor base is a positional dict
                // — let the match below report "not 'dict'".
            } else {
                args_vec.pop();
            }
        }
    }
    // An explicit base only works with str/bytes/bytearray (and subclasses
    // thereof, whose backing re-enters this function) — int(5.5, 2) /
    // int(memoryview(b'100'), 2) raise "can't convert non-string with
    // explicit base".
    if args_vec.len() > 2 {
        return Err(PyError::type_error("int() takes at most 2 arguments"));
    }
    if base_obj.is_none() && args_vec.len() > 1 {
        base_obj = Some(args_vec[1].clone());
    }
    if base_obj.is_some() {
        let o = args_vec[0].borrow();
        let accepts = matches!(
            &*o,
            PyObject::Str(_)
                | PyObject::Bytes(_)
                | PyObject::ByteArray(_)
                | PyObject::Instance { .. }
        );
        if !accepts {
            return Err(PyError::type_error(
                "int() can't convert non-string with explicit base",
            ));
        }
    }
    let obj = args_vec[0].borrow();
    match &*obj {
        PyObject::Int(_) => Ok(args_vec[0].clone()),
        PyObject::Float(f) => {
            // Exact float->int truncation (a plain `*f as i64` cast would
            // silently saturate at i64::MAX for e.g. 1.797e308 instead of
            // producing the exact 309-digit integer; inf/nan raise).
            f64_to_int_ceil_floor_trunc(*f, 0).map(py_int)
        }
        PyObject::Str(s) => {
            check_int_str_digit_limit(s, base_obj.as_ref())?;
            int_from_digit_string(
                s,
                base_obj.as_ref(),
                &format!("'{}'", crate::object::escape_string(s)),
            )
        }
        PyObject::Bytes(b) => {
            let latin: String = b.iter().map(|&x| x as char).collect();
            check_int_str_digit_limit(&latin, base_obj.as_ref())?;
            int_from_digit_string(
                &latin,
                base_obj.as_ref(),
                &format!("b'{}'", python_bytes_repr(b)),
            )
        }
        PyObject::ByteArray(b) => {
            let latin: String = b.iter().map(|&x| x as char).collect();
            check_int_str_digit_limit(&latin, base_obj.as_ref())?;
            int_from_digit_string(
                &latin,
                base_obj.as_ref(),
                &format!("bytearray(b'{}')", python_bytes_repr(b)),
            )
        }
        PyObject::MemoryView { .. } => {
            // memoryview/array parse base-10 only; an explicit base is
            // rejected ("can't convert non-string with explicit base").
            if base_obj.is_some() {
                return Err(PyError::type_error(
                    "int() can't convert non-string with explicit base",
                ));
            }
            let bytes = crate::object::mv_tobytes(&args_vec[0].clone())?;
            let latin: String = bytes.iter().map(|&x| x as char).collect();
            int_from_digit_string(
                &latin,
                None,
                &format!("'{}'", crate::object::escape_string(&latin)),
            )
        }
        PyObject::Array(arr) => {
            if base_obj.is_some() {
                return Err(PyError::type_error(
                    "int() can't convert non-string with explicit base",
                ));
            }
            if matches!(arr.typecode, 'B' | 'b' | 'u') {
                let bytes: Vec<u8> = arr.data.iter().map(|&v| v as u8).collect();
                let latin: String = bytes.iter().map(|&x| x as char).collect();
                return int_from_digit_string(
                    &latin,
                    None,
                    &format!("'{}'", crate::object::escape_string(&latin)),
                );
            }
            Err(PyError::type_error(format!(
                "int() argument must be a string or number, not '{}'",
                obj.type_name()
            )))
        }
        PyObject::Bool(b) => Ok(py_int(if *b { 1 } else { 0 })),
        PyObject::Instance { typ, .. } => {
            let typ = typ.clone();
            drop(obj);
            // A custom (user-defined) __int__ takes precedence over the
            // native value — int(MyInt(7)) where MyInt.__int__()==42 is 42.
            // A bool return (an int subclass) is deprecated but usable.
            if let Some(int_method) = lookup_dunder_via_mro(&typ, "__int__") {
                let is_native = matches!(
                    &*int_method.borrow(),
                    PyObject::BuiltinFunction { .. } | PyObject::Closure(_)
                );
                let is_native_backed = native_backing_of(&args_vec[0]).is_some();
                // Native-backed int/str/etc. subclasses skip their implicit
                // (BuiltinFunction) `__int__` and use the backing instead —
                // but a genuine object like `Fraction` whose `__int__` is a
                // BuiltinFunction MUST be called.
                if !is_native || !is_native_backed {
                    let result = call_bound_method(int_method, args_vec[0].clone(), vec![])?;
                    if let Some(n) = result.as_i64() {
                        if matches!(result, PyObjectRef::SmallBool(_)) {
                            crate::modules::warnings_emit("__int__ returned non-int (type bool).  The ability to return an instance of a strict subclass of int is deprecated, and may be removed in a future version of Python.", "DeprecationWarning");
                        }
                        return Ok(py_int(n));
                    }
                    if matches!(&*result.borrow(), PyObject::Int(_)) {
                        return Ok(result);
                    }
                    return Err(PyError::type_error(format!(
                        "{}.__int__ returned non-int (type {})",
                        get_type_name_for_instance(&typ),
                        result.borrow().type_name()
                    )));
                }
            }
            // A class transparently subclassing `int` (e.g. IntEnum
            // members) with no custom `__int__` converts via its native
            // backing directly — real Python's `int(x)` for an int
            // subclass instance just IS that underlying int value. A str/
            // bytes/bytearray subclass (CustomStr(b'100')) parses via its
            // backing the same way, carrying an explicit base along.
            if let Some(native) = native_backing_of(&args_vec[0]) {
                if let Some(base) = base_obj {
                    return builtin_int(&[native, base]);
                }
                return builtin_int(&[native]);
            }
            // __index__ fallback (plain objects with __index__, e.g. numpy
            // scalars / custom index types) — a bool result is deprecated.
            if let Some(index_method) = lookup_dunder_via_mro(&typ, "__index__") {
                let result = call_bound_method(index_method, args_vec[0].clone(), vec![])?;
                if let Some(n) = result.as_i64() {
                    if matches!(result, PyObjectRef::SmallBool(_)) {
                        crate::modules::warnings_emit("__index__ returned non-int (type bool).  The ability to return an instance of a strict subclass of int is deprecated, and may be removed in a future version of Python.", "DeprecationWarning");
                    }
                    return Ok(py_int(n));
                }
                if matches!(&*result.borrow(), PyObject::Int(_)) {
                    return Ok(result);
                }
                return Err(PyError::type_error(format!(
                    "{}.__index__ returned non-int (type {})",
                    get_type_name_for_instance(&typ),
                    result.borrow().type_name()
                )));
            }
            Err(PyError::type_error(format!(
                "int() argument must be a string or number, not '{}'",
                get_type_name_for_instance(&typ)
            )))
        }
        _ => Err(PyError::type_error(format!(
            "int() argument must be a string or number, not '{}'",
            obj.type_name()
        ))),
    }
}

/// int.from_bytes(bytes, byteorder, *, signed=False)
pub fn builtin_int_from_bytes(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "int.from_bytes() needs at least 2 arguments",
        ));
    }
    let bytes_val = &args[0];
    let byteorder = &args[1];
    let order_str = byteorder.str();
    let big_endian = order_str == "big";
    let byte_data: Vec<u8> = match &*bytes_val.borrow() {
        PyObject::Bytes(b) => b.clone(),
        PyObject::List(items) => items
            .iter()
            .map(|x| x.as_i64().unwrap_or(0) as u8)
            .collect(),
        _ => {
            let mut v = Vec::new();
            if let Ok(it) = builtin_iter(&[bytes_val.clone()]) {
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(x) => v.push(x.as_i64().unwrap_or(0) as u8),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            v
        }
    };
    let n = if big_endian {
        byte_data.iter().fold(0i64, |acc, &b| (acc << 8) | b as i64)
    } else {
        byte_data
            .iter()
            .rev()
            .fold(0i64, |acc, &b| (acc << 8) | b as i64)
    };
    Ok(py_int(n))
}

