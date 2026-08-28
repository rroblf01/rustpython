// Extracted from ctors.rs — string interpolation (% operator) for str.
use super::*;
use crate::object::*;

pub(crate) fn string_interpolate(fmt: &str, arg: &PyObjectRef) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = fmt.chars();
    let mut converted = 0usize;

    // Handle tuple arguments: consume one element per format spec. Real
    // CPython's `%` operator does this ONLY for a tuple RHS — a list (or
    // any other single non-mapping value) is always used as-is for a
    // single conversion, never unpacked positionally. This used to also
    // treat `PyObject::List` the same as a tuple, so `"%r" % some_list`
    // silently dropped every element past the first `%`-spec instead of
    // formatting the whole list — confirmed via a general, non-Django repro
    // (`"%r" % self._tests` inside a `__repr__`, `self._tests` a 3-element
    // list, printing only the first element's repr instead of the whole
    // list's) that also masked an unrelated real bug during Django/unittest
    // bisecting (looked like a lost/dropped list element, was actually this
    // formatting bug misrepresenting a correctly-3-element list as 1).
    let mut arg_iter: Option<Box<dyn Iterator<Item = PyObjectRef>>> = None;
    let arg0 = arg.clone();
    {
        let obj = arg0.borrow();
        match &*obj {
            PyObject::Tuple(items) => {
                let vec = items.clone();
                let iter = vec.into_iter();
                arg_iter = Some(Box::new(iter));
            }
            // namedtuple instances are tuple subclasses in real CPython, so
            // `'%d %d %r' % a_namedtuple` unpacks positionally like a tuple
            // (unittest's `_Mismatch` in assertCountEqual relies on it).
            PyObject::Instance { dict, .. } if dict.get_str("_fields").is_some() => {
                let mut vals: Vec<PyObjectRef> = Vec::new();
                if let Some(fields) = dict.get_str("_fields") {
                    match &*fields.borrow() {
                        PyObject::List(fields) => {
                            for f in fields {
                                if let Some(v) = dict.get(&f.str()) {
                                    vals.push(v.clone());
                                }
                            }
                        }
                        PyObject::Tuple(fields) => {
                            for f in fields {
                                if let Some(v) = dict.get(&f.str()) {
                                    vals.push(v.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                arg_iter = Some(Box::new(vals.into_iter()));
            }
            _ => {}
        }
    }
    // Helper: get next arg (consume from tuple iterator, or always use the single arg)
    let mut get_arg = || -> PyObjectRef {
        if let Some(ref mut it) = arg_iter {
            it.next().unwrap_or_else(|| py_str(""))
        } else {
            arg.clone()
        }
    };

    while let Some(ch) = chars.next() {
        if ch == '%' {
            // Parse an optional mapping key: %(name)s pulls "name" out of a
            // dict argument instead of consuming positionally.
            let mut mapping_key: Option<String> = None;
            if chars.clone().next() == Some('(') {
                chars.next(); // consume '('
                let mut key = String::new();
                loop {
                    match chars.next() {
                        Some(')') => break,
                        Some(c) => key.push(c),
                        None => return Err("incomplete format key".to_string()),
                    }
                }
                mapping_key = Some(key);
            }
            // Parse optional width specifier (e.g., %03o, %02d, %3s, %4d)
            let mut width: Option<usize> = None;
            // Parse flags (`0`, `-`, `+`, space, `#`) — previously only `0`.
            let mut flags = String::new();
            loop {
                let mut peek = chars.clone();
                match peek.next() {
                    Some(c) if c == '0' || c == '-' || c == '+' || c == ' ' || c == '#' => {
                        flags.push(c);
                        chars.next();
                    }
                    _ => break,
                }
            }
            // Parse width (digits or `*` for dynamic width from arg)
            let mut width_str = String::new();
            if chars.clone().next() == Some('*') {
                chars.next();
                let w_arg = get_arg();
                let w = w_arg.as_i64().unwrap_or(0).max(0) as usize;
                if w > 1000 {
                    return Err("width too big".to_string());
                }
                width = Some(w);
            } else {
                loop {
                    let mut peek2 = chars.clone();
                    match peek2.next() {
                        Some(c) if c.is_ascii_digit() => {
                            width_str.push(c);
                            chars.next();
                        }
                        _ => break,
                    }
                }
                if !width_str.is_empty() {
                    let w = width_str
                        .parse::<usize>()
                        .map_err(|_| "invalid width".to_string())?;
                    if w > 1000 {
                        return Err("width too big".to_string());
                    }
                    width = Some(w);
                }
            }
            // Parse optional `.precision` (e.g. `%.3f`, `%6.2f`) — this was
            // entirely unparsed before, so any precision-qualified float
            // format (an extremely common idiom for printing durations/
            // percentages/rounded numbers — real trigger: CPython 3.14's
            // own `unittest/runner.py`, `"%.3fs" % elapsed`) hit the literal
            // `.` as if it were an (unsupported) conversion character.
            let mut precision: Option<usize> = None;
            if chars.clone().next() == Some('.') {
                chars.next();
                // Dynamic precision `.*` — the value comes from the next arg.
                if chars.clone().next() == Some('*') {
                    chars.next();
                    let p = get_arg();
                    precision = Some(p.as_i64().unwrap_or(6).max(0) as usize);
                } else {
                    let mut prec_str = String::new();
                    loop {
                        let mut peek3 = chars.clone();
                        match peek3.next() {
                            Some(c) if c.is_ascii_digit() => {
                                prec_str.push(c);
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                    precision = Some(if prec_str.is_empty() {
                        0
                    } else {
                        let p = prec_str
                            .parse::<usize>()
                            .map_err(|_| "invalid precision".to_string())?;
                        // See the matching `width` cap above — a precision this
                        // large parses fine but panics Rust's own `format!`
                        // machinery when actually used (`test_str.py::
                        // test_formatting_huge_precision`: `"%.{}f" %
                        // (sys.maxsize + 1)`).
                        if p > 1000 {
                            return Err("precision too big".to_string());
                        }
                        p
                    });
                }
            }

            let had_spec = !flags.is_empty()
                || width.is_some()
                || precision.is_some()
                || mapping_key.is_some();
            match chars.next() {
                None => return Err("incomplete format".to_string()),
                // `%%` is only an escape when the % follows immediately — a
                // % after a flag/width/precision is an unsupported character
                // ('% %s' reports the second % at index 2).
                Some('%') if had_spec => {
                    return Err(format!(
                        "unsupported format character '%' (0x25) at index {}",
                        byte_index_in(fmt, chars.as_str())
                    ));
                }
                Some('%') => result.push('%'),
                Some(conv @ 's') | Some(conv @ 'r') | Some(conv @ 'f') | Some(conv @ 'd')
                | Some(conv @ 'i') | Some(conv @ 'o') | Some(conv @ 'x') | Some(conv @ 'X')
                | Some(conv @ 'c') | Some(conv @ 'e') | Some(conv @ 'E') | Some(conv @ 'g')
                | Some(conv @ 'G') | Some(conv @ 'u') | Some(conv @ 'F') | Some(conv @ 'a') => {
                    converted += 1;
                    let raw = if let Some(ref key) = mapping_key {
                        let obj = arg.borrow();
                        match &*obj {
                            PyObject::Dict(d) => d
                                .get(&py_str(key))
                                .ok()
                                .flatten()
                                .ok_or_else(|| format!("'{}'", key))?,
                            _ => return Err("format requires a mapping".to_string()),
                        }
                    } else {
                        get_arg()
                    };

                    if matches!(conv, 'f' | 'F' | 'e' | 'E' | 'g' | 'G')
                        && !matches!(
                            &*raw.borrow(),
                            PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)
                        )
                    {
                        return Err(format!(
                            "must be real number, not {}",
                            raw.borrow().type_name()
                        ));
                    }
                    let formatted = match conv {
                        's' => raw.str(),
                        'r' => raw.repr(),
                        'a' => {
                            // ascii() repr: repr with ALL non-ASCII escaped
                            let r = raw.repr();
                            let mut out = String::new();
                            for c in r.chars() {
                                if c.is_ascii() {
                                    out.push(c);
                                } else if (c as u32) <= 0xFFFF {
                                    out.push_str(&format!("\\u{:04x}", c as u32));
                                } else {
                                    out.push_str(&format!("\\U{:08x}", c as u32));
                                }
                            }
                            out
                        }
                        'f' => {
                            if !matches!(
                                &*raw.borrow(),
                                PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)
                            ) {
                                return Err(format!(
                                    "must be real number, not {}",
                                    raw.borrow().type_name()
                                ));
                            }
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            // CPython's test_format exercises %12.*f with
                            // precision 123456 (must work); sys.maxsize must
                            // still overflow. Rust's own format! panics at
                            // large precisions, so format at <=100 decimals
                            // (exact for any f64) and pad the rest with
                            // zeros.
                            if prec > 200000 {
                                return Err("precision too big".to_string());
                            }
                            let mut s = format_fixed_padded(f, prec);
                            // # keeps the decimal point for integral values,
                            // but never for inf/nan ('%#f' % inf == 'inf').
                            if flags.contains('#')
                                && !s.contains('.')
                                && !f.is_nan()
                                && !f.is_infinite()
                            {
                                s.push('.');
                            }
                            apply_sign_flag(s, &flags)
                        }
                        'F' => {
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            if prec > 200000 {
                                return Err("precision too big".to_string());
                            }
                            // %F upper-cases nan/inf (INF/NAN), like %G.
                            let mut s = format_fixed_padded(f, prec).to_uppercase();
                            if flags.contains('#') && !s.contains('.') {
                                s.push('.');
                            }
                            apply_sign_flag(s, &flags)
                        }
                        'e' | 'E' => {
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            apply_sign_flag(
                                format_percent_e(f, prec, flags.contains('#'), conv == 'E'),
                                &flags,
                            )
                        }
                        'g' | 'G' => {
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            apply_sign_flag(
                                format_percent_g(f, prec, flags.contains('#'), false),
                                &flags,
                            )
                        }
                        'd' | 'i' | 'u' => {
                            // A non-numeric arg must raise ("%d format: a
                            // real number is required, not str").
                            if !matches!(
                                &*raw.borrow(),
                                PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)
                            ) {
                                return Err(format!(
                                    "%{} format: a real number is required, not {}",
                                    conv,
                                    raw.borrow().type_name()
                                ));
                            }
                            // Handle big ints that overflow i64, and float
                            // whole numbers ('%d' % -1.2e29) — stringify via
                            // BigInt (test_common_format).
                            let mut s = bigint_of(&raw).to_string();
                            if !s.starts_with('-') {
                                if flags.contains('+') {
                                    s = format!("+{}", s);
                                } else if flags.contains(' ') {
                                    s = format!(" {}", s);
                                }
                            }
                            // `.precision` zero-pads %d/%i (`%.100d` of 1
                            // -> 99 zeros then 1). A huge precision
                            // (e.g. sys.maxsize from `%.*d`) must raise,
                            // not allocate an astronomical string.
                            if let Some(p) = precision {
                                if p > 1000 {
                                    return Err("precision too big".to_string());
                                }
                                if p > s.len() {
                                    format!("{}{}", "0".repeat(p - s.len()), s)
                                } else {
                                    s
                                }
                            } else {
                                s
                            }
                        }
                        'o' => {
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                                return Err(format!(
                                    "%{} format: an integer is required, not {}",
                                    conv,
                                    raw.borrow().type_name()
                                ));
                            }
                            let bi = bigint_of(&raw);
                            let neg = bi.sign() == num_bigint::Sign::Minus;
                            let mut s = if flags.contains('#') {
                                if neg {
                                    format!("-0o{:o}", bi.abs())
                                } else {
                                    format!("0o{:o}", bi)
                                }
                            } else {
                                format!("{:o}", bi)
                            };
                            if !s.starts_with('-') && flags.contains('+') {
                                s = format!("+{}", s);
                            } else if !s.starts_with('-') && flags.contains(' ') {
                                s = format!(" {}", s);
                            }
                            if let Some(p) = precision {
                                s = zero_pad_precision(s, p, flags.contains('#'))
                            }
                            s
                        }
                        'x' => {
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                                return Err(format!(
                                    "%{} format: an integer is required, not {}",
                                    conv,
                                    raw.borrow().type_name()
                                ));
                            }
                            let bi = bigint_of(&raw);
                            let neg = bi.sign() == num_bigint::Sign::Minus;
                            let mut s = if flags.contains('#') {
                                if neg {
                                    format!("-0x{:x}", bi.abs())
                                } else {
                                    format!("0x{:x}", bi)
                                }
                            } else {
                                format!("{:x}", bi)
                            };
                            if !s.starts_with('-') && flags.contains('+') {
                                s = format!("+{}", s);
                            } else if !s.starts_with('-') && flags.contains(' ') {
                                s = format!(" {}", s);
                            }
                            if let Some(p) = precision {
                                s = zero_pad_precision(s, p, flags.contains('#'))
                            }
                            s
                        }
                        'X' => {
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                                return Err(format!(
                                    "%{} format: an integer is required, not {}",
                                    conv,
                                    raw.borrow().type_name()
                                ));
                            }
                            let bi = bigint_of(&raw);
                            let neg = bi.sign() == num_bigint::Sign::Minus;
                            let mut s = if flags.contains('#') {
                                if neg {
                                    format!("-0X{:X}", bi.abs())
                                } else {
                                    format!("0X{:X}", bi)
                                }
                            } else {
                                format!("{:X}", bi)
                            };
                            if !s.starts_with('-') && flags.contains('+') {
                                s = format!("+{}", s);
                            } else if !s.starts_with('-') && flags.contains(' ') {
                                s = format!(" {}", s);
                            }
                            if let Some(p) = precision {
                                s = zero_pad_precision(s, p, flags.contains('#'))
                            }
                            s
                        }
                        'c' => {
                            if let Some(i) = raw.as_i64() {
                                // %c of an int must be a valid Unicode scalar
                                // (0..0x110000) — out of range raises
                                // OverflowError ("%c arg not in range(0x110000)").
                                match char::from_u32(i as u32) {
                                    Some(c) => c.to_string(),
                                    None => {
                                        return Err(
                                            "%c arg not in range(0x110000) [overflow]".to_string()
                                        )
                                    }
                                }
                            } else if matches!(&*raw.borrow(), PyObject::Str(_)) {
                                let s = raw.str();
                                if s.chars().count() != 1 {
                                    return Err(format!(
                                        "%c requires an int or a unicode character, not a string of length {}",
                                        s.chars().count()
                                    ));
                                }
                                s.chars().next().map(|c| c.to_string()).unwrap_or_default()
                            } else {
                                return Err(format!(
                                    "%c requires an int or a unicode character, not {}",
                                    raw.borrow().type_name()
                                ));
                            }
                        }
                        _ => unreachable!(),
                    };

                    // Apply width (respecting - left-justify, 0 zero-pad)
                    let padded = if let Some(w) = width {
                        if flags.contains('-') {
                            format!("{:<width$}", formatted, width = w)
                        } else if flags.contains('0') {
                            // Zero-pad AFTER the sign and any 0x/0o prefix
                            // (`%032d` of -big -> '-000...digits';
                            // `%#027x` -> '0x00001234567890abcdef12345').
                            let (sign, rest) = if let Some(r) = formatted.strip_prefix('-') {
                                ("-", r)
                            } else if let Some(r) = formatted.strip_prefix('+') {
                                ("+", r)
                            } else if let Some(r) = formatted.strip_prefix(' ') {
                                (" ", r)
                            } else {
                                ("", formatted.as_str())
                            };
                            let (prefix, body) = if let Some(r) = rest.strip_prefix("0x") {
                                ("0x", r)
                            } else if let Some(r) = rest.strip_prefix("0X") {
                                ("0X", r)
                            } else if let Some(r) = rest.strip_prefix("0o") {
                                ("0o", r)
                            } else {
                                ("", rest)
                            };
                            let padded = format!(
                                "{:0>width$}",
                                body,
                                width = w.saturating_sub(sign.len() + prefix.len())
                            );
                            format!("{}{}{}", sign, prefix, padded)
                        } else {
                            format!("{:>width$}", formatted, width = w)
                        }
                    } else {
                        formatted
                    };
                    result.push_str(&padded);
                }
                Some(c) => {
                    return Err(format!(
                        "unsupported format character '{}' (0x{:02x}) at index {}",
                        c,
                        c as u32,
                        byte_index_in(fmt, chars.as_str())
                    ))
                }
            }
        } else {
            result.push(ch);
        }
    }

    // Real CPython raises if a single non-mapping arg is provided but no
    // conversion consumed it, or a tuple has MORE elements than specs.
    let arg_is_dict = matches!(&*arg0.borrow(), PyObject::Dict(_));
    if arg_iter.is_none() && !arg_is_dict && converted == 0 {
        return Err("not all arguments converted during string formatting".to_string());
    }
    if let Some(it) = arg_iter {
        let mut it = it;
        if it.next().is_some() {
            return Err("not all arguments converted during string formatting".to_string());
        }
    }
    Ok(result)
}
