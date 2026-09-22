use super::*;

/// Parses a real CPython-style complex literal string (`complex("1+2j")`,
/// `complex("-3-4j")`, `complex("2j")`, `complex("(1+2j)")`) — finds the
/// LAST top-level `+`/`-` before the trailing `j`/`J` (skipping one right
/// after `e`/`E`, which is an exponent sign, not the real/imag separator).
/// Supports digit-separator underscores (`complex("1_000+2_000j")`), which
/// must sit between two digits (CPython's numeric-literal rule).
fn parse_complex_str(s: &str) -> PyResult<(f64, f64)> {
    let malformed = || PyError::value_error(format!("complex() arg is a malformed string"));
    // Parse a float allowing `_` digit separators (validated: each `_` must be
    // between two digits), matching CPython's numeric-literal underscore rule.
    let parse_num = |t: &str| -> Result<f64, ()> {
        let chars: Vec<char> = t.chars().collect();
        let mut out = String::with_capacity(t.len());
        for (i, &c) in chars.iter().enumerate() {
            if c == '_' {
                let prev = if i > 0 { chars[i - 1] } else { '\0' };
                let next = if i + 1 < chars.len() { chars[i + 1] } else { '\0' };
                if !prev.is_ascii_digit() || !next.is_ascii_digit() {
                    return Err(());
                }
                continue; // strip the underscore
            }
            out.push(c);
        }
        out.parse::<f64>().map_err(|_| ())
    };
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
                let re: f64 = parse_num(real_str).map_err(|_| malformed())?;
                let im: f64 = match imag_str {
                    "+" => 1.0,
                    "-" => -1.0,
                    _ => parse_num(imag_str).map_err(|_| malformed())?,
                };
                Ok((re, im))
            }
            None => {
                let im: f64 = match stripped {
                    "" | "+" => 1.0,
                    "-" => -1.0,
                    _ => parse_num(stripped).map_err(|_| malformed())?,
                };
                Ok((0.0, im))
            }
        }
    } else {
        let re: f64 = parse_num(inner).map_err(|_| malformed())?;
        Ok((re, 0.0))
    }
}

pub fn builtin_complex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Normalize positional + keyword args (`real=`/`imag=` arrive as a
    // trailing dict). Build effective (real, imag) argument slots.
    let mut positional: Vec<PyObjectRef> = args.to_vec();
    let mut kw_real: Option<PyObjectRef> = None;
    let mut kw_imag: Option<PyObjectRef> = None;
    if let Some(last) = args.last() {
        if let PyObject::Dict(d) = &*last.borrow() {
            let has_kw = d.get(&py_str("real")).ok().flatten().is_some()
                || d.get(&py_str("imag")).ok().flatten().is_some();
            if has_kw {
                kw_real = d.get(&py_str("real")).ok().flatten();
                kw_imag = d.get(&py_str("imag")).ok().flatten();
                positional.pop(); // drop the trailing kwargs dict
            }
        }
    }
    let eff_real: Option<PyObjectRef> = positional.first().cloned().or_else(|| kw_real.clone());
    let eff_imag: Option<PyObjectRef> = positional.get(1).cloned().or(kw_imag);
    // Track whether `real` was supplied via the `real=` keyword — CPython
    // warns for `complex(real=<complex>)` even with no imag, but NOT for a
    // plain positional `complex(<complex>)`.
    let real_from_kw = positional.is_empty() && kw_real.is_some();
    // Rebuild an equivalent positional-style args list for the logic below:
    // [real] or [real, imag].
    let args: Vec<PyObjectRef> = match (&eff_real, &eff_imag) {
        (None, None) => Vec::new(),
        (Some(r), None) => vec![r.clone()],
        (None, Some(i)) => vec![py_float(0.0), i.clone()],
        (Some(r), Some(i)) => vec![r.clone(), i.clone()],
    };
    if args.is_empty() {
        return Ok(PyObjectRef::imm(PyObject::Complex(0.0, 0.0)));
    }
    // Error message for an invalid FIRST (real) argument: with a second arg
    // present CPython reports it as "argument 'real' must be a real number…",
    // otherwise the generic "argument must be a string or a number…" — both
    // WITHOUT the "complex() " prefix and WITHOUT quoting the type name
    // (test_constructor's assertRaisesRegex expects e.g. "not dict").
    let arg0_err = |tn: &str| -> String {
        if args.len() > 1 {
            format!("argument 'real' must be a real number, not {}", tn)
        } else {
            format!("argument must be a string or a number, not {}", tn)
        }
    };
    let (re, im, first_was_complex) = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Complex(re, im) => (*re, *im, true),
            PyObject::Int(i) => (i.to_f64().unwrap_or(0.0), 0.0, false),
            PyObject::Float(f) => (*f, 0.0, false),
            PyObject::Bool(b) => (if *b { 1.0 } else { 0.0 }, 0.0, false),
            PyObject::Str(s) => {
                if args.len() > 1 {
                    return Err(PyError::type_error(arg0_err("str")));
                }
                let (re, im) = parse_complex_str(s)?;
                (re, im, false)
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
                    // Direct complex result, OR a complex-subclass instance
                    // (e.g. `complex1.__complex__` returns `self`) — take its
                    // value from the native backing.
                    let direct = {
                        let rb = result.borrow();
                        match &*rb {
                            PyObject::Complex(re, im) => Some((*re, *im)),
                            _ => None,
                        }
                    };
                    if let Some((re, im)) = direct {
                        (re, im, true)
                    } else {
                        match native_backing_of(&result) {
                            Some(nb) => match &*nb.borrow() {
                                PyObject::Complex(re, im) => {
                                    // `__complex__` returned a non-plain-complex
                                    // instance (e.g. `complex1.__complex__` returns
                                    // `self`) — CPython DeprecationWarnings here
                                    // (test_constructor's complex1 case).
                                    let rtype = result.borrow().type_name().to_string();
                                    crate::modules::warnings_emit(
                                        &format!(
                                            "argument 'real' must be a real number, not {}",
                                            rtype
                                        ),
                                        "DeprecationWarning",
                                    );
                                    (*re, *im, true)
                                }
                                _ => {
                                    return Err(PyError::type_error(
                                        "__complex__ returned non-complex",
                                    ))
                                }
                            },
                            None => {
                                return Err(PyError::type_error(
                                    "__complex__ returned non-complex",
                                ))
                            }
                        }
                    }
                }
                // A real `class ComplexSubclass(complex): ...` instance
                // (complex is one of the native types migrated to real
                // `PyObject::Type` subclassing) has no `__complex__` dunder
                // of its own to find via MRO - its actual value lives in
                // its native backing instead. Missing this made
                // `complex(some_complex_subclass_instance)` raise
                // TypeError even though `.real`/`.imag` worked fine on the
                // very same instance (test_complex.py's
                // test_constructor_special_numbers).
                None => match native_backing_of(&args[0]) {
                    Some(native) => match &*native.borrow() {
                        PyObject::Complex(re, im) => (*re, *im, true),
                        _ => {
                            return Err(PyError::type_error(arg0_err(
                                &get_type_name_for_instance(typ),
                            )))
                        }
                    },
                    None => {
                        // Compute the type name BEFORE `drop(obj)` (typ is a
                        // borrow into obj; using it after the drop is the
                        // borrow error). No `__complex__`, no complex backing
                        // — try `__float__` then `__index__`
                        // (test_constructor: `complex(WithFloat(4.25))` works
                        // via `__float__`, `complex(WithIndex(42))` via
                        // `__index__`; a non-float/non-int return, or a huge
                        // `__index__` value, must TypeError/OverflowError).
                        let tn = get_type_name_for_instance(typ);
                        if let Some(ffn) = lookup_dunder_via_mro(typ, "__float__") {
                            let ffn = ffn.clone();
                            let self_obj = args[0].clone();
                            drop(obj);
                            let result = call_bound_method(ffn, self_obj, vec![])?;
                            let rb = result.borrow();
                            match &*rb {
                                PyObject::Float(fl) => (*fl, 0.0, false),
                                _ => return Err(PyError::type_error(arg0_err(&tn))),
                            }
                        } else if let Some(ifn) = lookup_dunder_via_mro(typ, "__index__") {
                            let ifn = ifn.clone();
                            let self_obj = args[0].clone();
                            drop(obj);
                            let result = call_bound_method(ifn, self_obj, vec![])?;
                            if let Some(i) = result.as_i64() {
                                (i as f64, 0.0, false)
                            } else {
                                let rb = result.borrow();
                                match &*rb {
                                    PyObject::Int(n) => match n.to_f64() {
                                        Some(f) if f.is_finite() => (f, 0.0, false),
                                        _ => {
                                            return Err(PyError::overflow_error(
                                                "int too large to convert to float",
                                            ))
                                        }
                                    },
                                    _ => return Err(PyError::type_error(arg0_err(&tn))),
                                }
                            }
                        } else {
                            return Err(PyError::type_error(arg0_err(&tn)));
                        }
                    }
                },
            },
            _ => {
                return Err(PyError::type_error(arg0_err(&obj.type_name())))
            }
        }
    };
    // DeprecationWarning when the `real` (first) argument is complex — fires
    // when an imag is supplied OR when real came via the `real=` keyword
    // (CPython: `complex(real=<cx>)` and `complex(<cx>, <imag>)` both warn,
    // but a plain positional `complex(<cx>)` does not; test_constructor).
    if first_was_complex && (args.len() > 1 || real_from_kw) {
        let type_name = {
            let borrowed = args[0].borrow();
            match &*borrowed {
                PyObject::Instance { typ, .. } => get_type_name_for_instance(typ),
                other => other.type_name().to_string(),
            }
        };
        crate::modules::warnings_emit(
            &format!("argument 'real' must be a real number, not {}", type_name),
            "DeprecationWarning",
        );
    }
    if args.len() > 1 {
        // DeprecationWarning when the `imag` (second) argument is complex
        // (`complex(0, 4.25+0j)` → "argument 'imag' must be a real number").
        {
            let direct_cx = {
                let b = args[1].borrow();
                matches!(&*b, PyObject::Complex(..))
            };
            let second_is_cx = direct_cx
                || native_backing_of(&args[1])
                    .map(|n| matches!(&*n.borrow(), PyObject::Complex(..)))
                    .unwrap_or(false);
            if second_is_cx {
                let type_name = {
                    let borrowed = args[1].borrow();
                    match &*borrowed {
                        PyObject::Instance { typ, .. } => get_type_name_for_instance(typ),
                        other => other.type_name().to_string(),
                    }
                };
                crate::modules::warnings_emit(
                    &format!("argument 'imag' must be a real number, not {}", type_name),
                    "DeprecationWarning",
                );
            }
        }
        // (extra_re, extra_im): the second argument's own real/imaginary
        // parts. A plain number's "imaginary part" is 0.0 — real CPython
        // still supports (with a deprecation warning) the second arg
        // itself being complex (`complex(1, 2j)` -> `(-1+0j)`,
        // `complex(3+4j, 1+2j)` -> `(1+5j)`), combining via
        // `real = a.real - b.imag`, `imag = a.imag + b.real`.
        let (extra_re, extra_im): (f64, f64) = {
            let obj = args[1].borrow();
            match &*obj {
                PyObject::Complex(re, im) => (*re, *im),
                PyObject::Int(i) => (i.to_f64().unwrap_or(0.0), 0.0),
                PyObject::Float(f) => (*f, 0.0),
                PyObject::Bool(b) => (if *b { 1.0 } else { 0.0 }, 0.0),
                PyObject::Instance { typ, .. } => {
                    // The `imag` (second) argument must be a real number or
                    // a complex value (native backing). Objects that only
                    // provide `__complex__` (e.g. `WithComplex`) are rejected
                    // with TypeError (test_constructor asserts this) — unlike
                    // the `real` (first) argument, which does use `__complex__`.
                    match native_backing_of(&args[1]) {
                        Some(native) => match &*native.borrow() {
                            PyObject::Complex(re, im) => (*re, *im),
                            _ => {
                                return Err(PyError::type_error(format!(
                                    "argument 'imag' must be a real number, not {}",
                                    get_type_name_for_instance(typ)
                                )))
                            }
                        },
                        None => {
                            // No complex backing — try `__float__` (must return a
                            // float) then `__index__` (returns an int; huge →
                            // OverflowError). test_constructor: `complex(1.5,
                            // WithFloat(4.25))`/`complex(1.5, WithIndex(42))`
                            // work, but `complex(1.5, WithFloat(42))` — its
                            // `__float__` returns an INT — must TypeError.
                            let tn = get_type_name_for_instance(typ);
                            if let Some(ffn) = lookup_dunder_via_mro(typ, "__float__") {
                                let ffn = ffn.clone();
                                let self_obj = args[1].clone();
                                drop(obj);
                                let result = call_bound_method(ffn, self_obj, vec![])?;
                                let rb = result.borrow();
                                match &*rb {
                                    PyObject::Float(fl) => (*fl, 0.0),
                                    _ => {
                                        return Err(PyError::type_error(format!(
                                            "argument 'imag' must be a real number, not {}",
                                            tn
                                        )))
                                    }
                                }
                            } else if let Some(ifn) = lookup_dunder_via_mro(typ, "__index__") {
                                let ifn = ifn.clone();
                                let self_obj = args[1].clone();
                                drop(obj);
                                let result = call_bound_method(ifn, self_obj, vec![])?;
                                if let Some(i) = result.as_i64() {
                                    (i as f64, 0.0)
                                } else {
                                    let rb = result.borrow();
                                    match &*rb {
                                        PyObject::Int(n) => match n.to_f64() {
                                            Some(f) if f.is_finite() => (f, 0.0),
                                            _ => {
                                                return Err(PyError::overflow_error(
                                                    "int too large to convert to float",
                                                ))
                                            }
                                        },
                                        _ => {
                                            return Err(PyError::type_error(format!(
                                                "argument 'imag' must be a real number, not {}",
                                                tn
                                            )))
                                        }
                                    }
                                }
                            } else {
                                return Err(PyError::type_error(format!(
                                    "argument 'imag' must be a real number, not {}",
                                    tn
                                )));
                            }
                        }
                    }
                },
                _ => {
                    return Err(PyError::type_error(format!(
                        "argument 'imag' must be a real number, not {}",
                        obj.type_name()
                    )))
                }
            }
        };
        // Only actually ADD when the first argument was itself complex
        // (or the second argument's own imaginary/real parts are
        // genuinely nonzero) - `im`/`extra_im`/`extra_re` are exactly
        // 0.0 in the overwhelmingly common `complex(x, y)` case with two
        // plain reals, and IEEE-754 `0.0 + (-0.0)` rounds to `+0.0`,
        // silently flushing a negative-zero `imag` argument's sign
        // (test_complex.py's test_constructor_special_numbers, exercised
        // via a `complex` subclass so it can't just be a display-only
        // quirk). Computing the final real part the same way (only
        // subtracting when there's something nonzero to subtract) keeps
        // a lone `-0.0` real part intact too.
        let new_im = if first_was_complex || extra_re != 0.0 {
            im + extra_re
        } else {
            extra_re
        };
        let new_re = if extra_im != 0.0 { re - extra_im } else { re };
        return Ok(PyObjectRef::imm(PyObject::Complex(new_re, new_im)));
    }
    // Single plain-complex argument → return the SAME object (complex is
    // immutable; test_constructor asserts `complex(c) is c`). Reached for
    // both the direct `complex(c)` call and a subclass's `CS(c)` backing
    // synthesis — returning the identical object is safe: the subclass path
    // stores it as NATIVE_BACKING, it is never re-wrapped/re-entered.
    if args.len() == 1 {
        if let PyObject::Complex(..) = &*args[0].borrow() {
            return Ok(args[0].clone());
        }
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
                let ord: i64 = if let Some(i) = k.as_i64() {
                    i
                } else {
                    let ks = k.str();
                    if ks.chars().count() != 1 {
                        return Err(PyError::value_error(
                            "string keys in translate table must be of length 1",
                        ));
                    }
                    ks.chars().next().unwrap() as i64
                };
                table.set(py_int(ord), v)?;
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
                table.set(py_int(*a as i64), py_str(&b.to_string()))?;
            }
            if args.len() == 3 {
                for c in args[2].str().chars() {
                    table.set(py_int(c as i64), py_none())?;
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
    // Handle kwargs packed as trailing dict (object/encoding/errors)
    let (positional, kwargs) = {
        if let Some(last) = args.last() {
            if let PyObject::Dict(d) = &*last.borrow() {
                let is_kwargs = d.contains(&py_str("object")).unwrap_or(false)
                    || d.contains(&py_str("encoding")).unwrap_or(false)
                    || d.contains(&py_str("errors")).unwrap_or(false);
                if is_kwargs {
                    (&args[..args.len() - 1], Some(last.clone()))
                } else {
                    (args, None)
                }
            } else {
                (args, None)
            }
        } else {
            (args, None)
        }
    };
    let get_kw = |k: &str| -> Option<PyObjectRef> {
        kwargs.as_ref().and_then(|d| {
            if let PyObject::Dict(dd) = &*d.borrow() {
                dd.get(&py_str(k)).ok().flatten()
            } else {
                None
            }
        })
    };
    let obj_arg: Option<PyObjectRef> = if let Some(v) = get_kw("object") {
        Some(v)
    } else {
        positional.get(0).cloned()
    };
    let enc_arg: Option<PyObjectRef> = if let Some(v) = get_kw("encoding") {
        Some(v)
    } else {
        positional.get(1).cloned()
    };
    let err_arg: Option<PyObjectRef> = if let Some(v) = get_kw("errors") {
        Some(v)
    } else {
        positional.get(2).cloned()
    };

    if obj_arg.is_none() && enc_arg.is_none() && err_arg.is_none() {
        return Ok(py_str(""));
    }
    if enc_arg.is_some() || err_arg.is_some() {
        let target = obj_arg.clone().unwrap_or_else(|| py_str(""));
        if matches!(
            &*target.borrow(),
            PyObject::Bytes(_) | PyObject::ByteArray(_)
        ) {
            let decode = target.borrow().get_attribute("decode")?;
            let enc = enc_arg.clone().unwrap_or_else(|| py_str("utf-8"));
            let err = err_arg.clone();
            if let PyObject::BuiltinMethod { func: f, .. } = &*decode.borrow() {
                let mut call_args = vec![target.clone()];
                call_args.push(enc);
                if let Some(e) = err {
                    call_args.push(e);
                }
                return f(&call_args);
            }
            let mut call_args = vec![enc];
            if let Some(e) = err {
                call_args.push(e);
            }
            return call_bound_method(decode, target.clone(), call_args);
        }
        if obj_arg.is_some() {
            return Err(PyError::type_error(
                "decoding to str: need a bytes-like object, found type object",
            ));
        }
        if enc_arg.is_some() || err_arg.is_some() {
            return Ok(py_str(""));
        }
    }
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
        let target_obj = obj_arg.clone().unwrap_or_else(|| py_str(""));

        // WeakProxy: str(proxy) forwards to str(target) if alive, else ReferenceError.
        if let PyObject::WeakProxy { target, .. } = &*target_obj.borrow() {
            if let Some(rc) = target.upgrade() {
                let target_ref = PyObjectRef::Mut(rc);
                return builtin_str(&[target_ref]);
            } else {
                return Err(PyError::reference_error(
                    "weakly-referenced object no longer exists",
                ));
            }
        }
        let f = {
            let obj_borrowed = target_obj.borrow();
            if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                lookup_dunder_via_mro(typ, "__str__")
            } else {
                None
            }
        };
        if let Some(f) = f {
            return call_bound_method(f, target_obj.clone(), vec![]);
        }
        let is_exc = if let PyObject::Instance { typ, .. } = &*target_obj.borrow() {
            is_exception_type(typ)
        } else {
            false
        };
        if is_exc {
            return Ok(py_str(&exception_instance_str(&target_obj)));
        }
        // int->str digit limit (str(10**10000) raises ValueError), also for
        // int subclass instances (whose native backing is the int).
        if let Some(i) = int_value_or_backing(&target_obj) {
            check_int_to_str_limit(&i)?;
        }
        Ok(py_str(&target_obj.str()))
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
    // Clear any stale flag left over from an unrelated earlier `.repr()`
    // call elsewhere (e.g. inside `print()`/`str()` formatting a different,
    // deeply-nested object) — only a truncation caused by THIS call should
    // be reported as a `RecursionError` here.
    crate::object::take_repr_recursion_overflow();
    let s = args[0].repr();
    if crate::object::take_repr_recursion_overflow() {
        return Err(PyError::recursion_error(
            "maximum recursion depth exceeded while getting the repr of an object",
        ));
    }
    Ok(py_str(&s))
}

pub fn builtin_bool(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Keyword rejection (`bool(x=10)`) lives in the VM's call_function,
    // where `keywords` is known before it gets flattened into an
    // indistinguishable trailing positional dict — see the list/tuple
    // handling there for why (a positional dict, e.g. `bool({})`, is
    // legal here and must not be confused with real keywords).
    if args.len() > 1 {
        return Err(PyError::type_error("bool() takes at most 1 argument"));
    }
    if args.is_empty() {
        return Ok(py_bool(false));
    }
    if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
        if target.upgrade().is_none() {
            return Err(PyError::reference_error(
                "weakly-referenced object no longer exists",
            ));
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
        // `format(x)` is `format(x, '')`, which still dispatches
        // `type(x).__format__` (mirrors CPython: instance `__format__`
        // attributes are bypassed, the type's is used).
        1 => {
            let result = crate::vm::format_with_spec(&args[0], "")
                .map_err(|e| PyError::value_error(format!("Format spec: {}", e)))?;
            Ok(py_str(&result))
        }
        2 => {
            if matches!(&*args[1].borrow(), PyObject::None) {
                return Err(PyError::type_error(
                    "format() argument 2 must be str, not NoneType",
                ));
            }
            let spec = args[1].str();
            // Use the comprehensive format_with_spec from vm.rs
            let result = crate::vm::format_with_spec(&args[0], &spec)
                .map_err(|e| PyError::value_error(format!("Format spec: {}", e)))?;
            Ok(py_str(&result))
        }
        _ => Err(PyError::type_error("format() takes at most 2 arguments")),
    }
}
