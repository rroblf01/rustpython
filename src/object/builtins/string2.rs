use super::*;

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
    if args.len() > 1 {
        if let Some(last) = args.last() {
            if matches!(&*last.borrow(), PyObject::Dict(_)) {
                return Err(PyError::type_error("bool() takes no keyword arguments"));
            }
        }
    }
    if args.len() > 1 {
        return Err(PyError::type_error("bool() takes at most 1 argument"));
    }
    if args.is_empty() {
        return Ok(py_bool(false));
    }
    if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
        if target.upgrade().is_none() {
            return Err(PyError::reference_error("weakly-referenced object no longer exists"));
        }
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
