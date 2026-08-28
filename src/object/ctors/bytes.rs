// Extracted from ctors.rs — bytes interpolation (% operator) for bytes/bytearray.
use super::*;
use crate::object::*;

/// printf-style byte-string interpolation (`bytes`/`bytearray`'s `%` operator, PEP 461).
/// Deliberately covers the common conversions (`%s`/`%b`/`%d`/`%i`/`%o`/`%x`/`%X`/`%c`/`%%`
/// plus width) rather than CPython's full adversarial surface (`%a`'s exact unicode-escaping
/// repr, the `-` left-justify flag) — those are not exercised outside `test_format.py`'s own
/// exhaustive self-test.
pub(crate) fn bytes_interpolate(fmt: &[u8], arg: &PyObjectRef) -> Result<Vec<u8>, String> {
    let mut result: Vec<u8> = Vec::new();
    let mut i = 0;

    let mut arg_iter: Option<Box<dyn Iterator<Item = PyObjectRef>>> = None;
    {
        let obj = arg.borrow();
        if let PyObject::Tuple(items) = &*obj {
            arg_iter = Some(Box::new(items.clone().into_iter()));
        }
    }
    let arg0 = arg.clone();
    let mut get_arg = || -> PyObjectRef {
        if let Some(ref mut it) = arg_iter {
            it.next().unwrap_or_else(py_none)
        } else {
            arg0.clone()
        }
    };

    // Bytes-like: bytes, bytearray, or memoryview-over-bytes — used by %s/%b.
    let as_bytes_like = |v: &PyObjectRef| -> Option<Vec<u8>> {
        match &*v.borrow() {
            PyObject::Bytes(b) => Some(b.clone()),
            PyObject::ByteArray(b) => Some(b.clone()),
            PyObject::MemoryView { .. } => crate::object::mv_tobytes(v).ok(),
            _ => None,
        }
    };

    let mut converted = 0usize;
    while i < fmt.len() {
        let ch = fmt[i];
        i += 1;
        if ch != b'%' {
            result.push(ch);
            continue;
        }
        let mut flags_zero = false;
        let mut flags_alt = false;
        let mut flags_minus = false;
        let mut flags_plus = false;
        let mut flags_space = false;
        loop {
            if i >= fmt.len() {
                break;
            }
            match fmt[i] {
                b'0' => {
                    flags_zero = true;
                    i += 1;
                }
                b'#' => {
                    flags_alt = true;
                    i += 1;
                }
                b'-' => {
                    flags_minus = true;
                    i += 1;
                }
                b'+' => {
                    flags_plus = true;
                    i += 1;
                }
                b' ' => {
                    flags_space = true;
                    i += 1;
                }
                _ => break,
            }
        }
        let mut width_str = String::new();
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            width_str.push(fmt[i] as char);
            i += 1;
        }
        let width: Option<usize> = if width_str.is_empty() {
            None
        } else {
            Some(width_str.parse().map_err(|_| "invalid width".to_string())?)
        };
        // Parse optional `.precision` (e.g. b"%.2f", b"%.0d") and dynamic `.*`.
        let mut precision: Option<usize> = None;
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            if i < fmt.len() && fmt[i] == b'*' {
                i += 1;
                let p = get_arg();
                precision = Some(p.as_i64().unwrap_or(6).max(0) as usize);
            } else {
                let mut prec_str = String::new();
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    prec_str.push(fmt[i] as char);
                    i += 1;
                }
                precision = Some(if prec_str.is_empty() {
                    0
                } else {
                    prec_str
                        .parse()
                        .map_err(|_| "invalid precision".to_string())?
                });
            }
        }
        if i >= fmt.len() {
            return Err("incomplete format".to_string());
        }
        let conv = fmt[i];
        i += 1;
        // `%%` is only an escape when the % follows immediately — after a
        // flag it is an unsupported character ('% %s' -> the second %).
        let had_spec = flags_zero
            || flags_alt
            || flags_minus
            || flags_plus
            || flags_space
            || width.is_some()
            || precision.is_some();
        if conv == b'%' && had_spec {
            return Err(format!(
                "unsupported format character '%' (0x25) at index {}",
                i - 1
            ));
        }
        if conv != b'%' {
            converted += 1;
        }
        let formatted: Vec<u8> = match conv {
            b'%' => vec![b'%'],
            b's' | b'b' => {
                let raw = get_arg();
                if let Some(b) = as_bytes_like(&raw) {
                    b
                } else if let Some(f) = raw.borrow().get_attribute("__bytes__").ok() {
                    crate::object::call_bound_method(f, raw.clone(), vec![])
                        .and_then(|r| {
                            let rb = r.borrow();
                            if let PyObject::Bytes(b) = &*rb { Ok(b.clone()) } else { Err(crate::object::PyError::type_error("__bytes__ returned non-bytes")) }
                        })
                        .map_err(|_| format!(
                            "%{} requires a bytes-like object, or an object that implements __bytes__, not '{}'",
                            conv as char, raw.borrow().type_name()
                        ))?
                } else {
                    return Err(format!(
                        "%{} requires a bytes-like object, or an object that implements __bytes__, not '{}'",
                        conv as char, raw.borrow().type_name()
                    ));
                }
            }
            b'r' => {
                let raw = get_arg();
                let s = raw.repr();
                // Escape non-ASCII like %a (CPython: b'%r' % 'Մ' ==
                // b"'\\u0544'").
                let mut bytes: Vec<u8> = Vec::new();
                for ch in s.chars() {
                    if ch.is_ascii() {
                        bytes.push(ch as u8);
                    } else if (ch as u32) <= 0xFFFF {
                        bytes.extend_from_slice(format!("\\u{:04x}", ch as u32).as_bytes());
                    } else {
                        bytes.extend_from_slice(format!("\\U{:08x}", ch as u32).as_bytes());
                    }
                }
                bytes
            }
            b'a' => {
                // ascii() representation: ASCII chars verbatim, non-ASCII
                // as \uXXXX / \UXXXXXXXX escapes (CPython: b'%a' % 'Մ'
                // == b"'\\u0544'").
                let raw = get_arg();
                let s = raw.repr();
                let mut bytes: Vec<u8> = Vec::new();
                for ch in s.chars() {
                    if ch.is_ascii() {
                        bytes.push(ch as u8);
                    } else if (ch as u32) <= 0xFFFF {
                        bytes.extend_from_slice(format!("\\u{:04x}", ch as u32).as_bytes());
                    } else {
                        bytes.extend_from_slice(format!("\\U{:08x}", ch as u32).as_bytes());
                    }
                }
                bytes
            }
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
                let raw = get_arg();
                let is_real = matches!(conv, b'd' | b'i' | b'u');
                let allowed = if is_real {
                    matches!(
                        &*raw.borrow(),
                        PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)
                    )
                } else {
                    // %x/%X/%o reject even whole floats (an integer is required)
                    matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_))
                };
                if !allowed {
                    return Err(format!(
                        "%{} format: {} is required, not {}",
                        conv as char,
                        if is_real {
                            "a real number"
                        } else {
                            "an integer"
                        },
                        raw.borrow().type_name()
                    ));
                }
                let bi = bigint_of(&raw);
                let mut s = match conv {
                    b'd' | b'i' | b'u' => {
                        if let Some(p) = precision {
                            // zero-pad to precision digits (`%.100d` of 1)
                            if p > 1000 {
                                return Err("precision too big".to_string());
                            }
                            let s2 = bi.to_string();
                            if p > s2.len() {
                                format!("{}{}", "0".repeat(p - s2.len()), s2)
                            } else {
                                s2
                            }
                        } else {
                            bi.to_string()
                        }
                    }
                    b'o' => {
                        if flags_alt {
                            if bi.sign() == num_bigint::Sign::Minus {
                                format!("-0o{:o}", bi.abs())
                            } else {
                                format!("0o{:o}", bi)
                            }
                        } else {
                            format!("{:o}", bi)
                        }
                    }
                    b'x' => {
                        if flags_alt {
                            if bi.sign() == num_bigint::Sign::Minus {
                                format!("-0x{:x}", bi.abs())
                            } else {
                                format!("0x{:x}", bi)
                            }
                        } else {
                            format!("{:x}", bi)
                        }
                    }
                    b'X' => {
                        if flags_alt {
                            if bi.sign() == num_bigint::Sign::Minus {
                                format!("-0X{:X}", bi.abs())
                            } else {
                                format!("0X{:X}", bi)
                            }
                        } else {
                            format!("{:X}", bi)
                        }
                    }
                    _ => unreachable!(),
                };
                if !s.starts_with('-') {
                    if flags_plus {
                        s = format!("+{}", s);
                    } else if flags_space {
                        s = format!(" {}", s);
                    }
                }
                if let Some(p) = precision {
                    s = zero_pad_precision(s, p, flags_alt);
                }
                if let Some(w) = width {
                    s = if flags_minus {
                        format!("{:<width$}", s, width = w)
                    } else if flags_zero {
                        // zero-pad after the sign and 0x/0o prefix
                        let (sign, rest) = if let Some(r) = s.strip_prefix('-') {
                            ("-", r)
                        } else if let Some(r) = s.strip_prefix('+') {
                            ("+", r)
                        } else if let Some(r) = s.strip_prefix(' ') {
                            (" ", r)
                        } else {
                            ("", s.as_str())
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
                        format!("{:>width$}", s, width = w)
                    };
                }
                s.into_bytes()
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                let raw = get_arg();
                // Must be a real number — a str arg raises TypeError
                // ("float argument required, not str"), not silently 0.0.
                let is_num = raw.as_f64().is_some()
                    || matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Float(_));
                if !is_num {
                    return Err(format!(
                        "float argument required, not '{}'",
                        raw.borrow().type_name()
                    ));
                }
                let f = raw.as_f64().unwrap_or(0.0);
                let p = precision.unwrap_or(6);
                if p > 200000 {
                    return Err("precision too big".to_string());
                }
                let s = match conv {
                    b'e' | b'E' => crate::object::format_percent_e(f, p, flags_alt, conv == b'E'),
                    b'g' | b'G' => {
                        let mut s = crate::object::format_percent_g(f, p, flags_alt, false);
                        if conv == b'G' {
                            s = s.to_uppercase();
                        }
                        s
                    }
                    _ => {
                        // b'f' | b'F' — %F upper-cases nan/inf (INF/NAN).
                        let mut s = format_fixed_padded(f, p);
                        if conv == b'F' {
                            s = s.to_uppercase();
                        }
                        if flags_alt && !s.contains('.') {
                            s.push('.');
                        }
                        s
                    }
                };
                let mut s = s.into_bytes();
                if let Some(w) = width {
                    let s_str = String::from_utf8_lossy(&s).to_string();
                    let padded = if flags_minus {
                        format!("{:<width$}", s_str, width = w)
                    } else if flags_zero {
                        format!("{:0>width$}", s_str, width = w)
                    } else {
                        format!("{:>width$}", s_str, width = w)
                    };
                    s = padded.into_bytes();
                }
                s
            }
            b'c' => {
                let raw = get_arg();
                let out = if let Some(n) = raw.as_i64() {
                    if n < 0 || n > 255 {
                        return Err("%c arg not in range(256) [overflow]".to_string());
                    }
                    vec![n as u8]
                } else if matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                    // A big int (e.g. 2**128) is out of range(256).
                    return Err("%c arg not in range(256) [overflow]".to_string());
                } else if let Some(b) = as_bytes_like(&raw) {
                    if b.len() != 1 {
                        return Err("%c requires an integer in range(256) or a single byte, not a bytes object of length {}".to_string());
                    }
                    b
                } else if matches!(&*raw.borrow(), PyObject::Str(_)) {
                    return Err(
                        "%c requires an integer in range(256) or a single byte, not str"
                            .to_string(),
                    );
                } else {
                    return Err("%c requires an integer in range(256) or a single byte".to_string());
                };
                if let Some(w) = width {
                    if out.len() < w {
                        let pad = w - out.len();
                        if flags_minus {
                            let mut v = out.clone();
                            v.extend(std::iter::repeat(b' ').take(pad));
                            v
                        } else {
                            let mut v = std::iter::repeat(b' ').take(pad).collect::<Vec<u8>>();
                            v.extend(out);
                            v
                        }
                    } else {
                        out
                    }
                } else {
                    out
                }
            }
            c => {
                return Err(format!(
                    "unsupported format character '{}' (0x{:02x}) at index {}",
                    c as char,
                    c as u32,
                    i - 1
                ))
            }
        };
        let padded = if let Some(w) = width {
            if formatted.len() >= w {
                formatted
            } else {
                let pad_len = w - formatted.len();
                let pad_byte = if flags_zero { b'0' } else { b' ' };
                let mut v = vec![pad_byte; pad_len];
                v.extend(formatted);
                v
            }
        } else {
            formatted
        };
        result.extend(padded);
    }

    // Real CPython raises if a single non-mapping arg is provided but no
    // conversion consumed it, or a tuple has MORE elements than specs.
    let arg_is_dict = matches!(&*arg0.borrow(), PyObject::Dict(_));
    if arg_iter.is_none() && converted == 0 && !arg_is_dict {
        return Err("not all arguments converted during bytes formatting".to_string());
    }
    if let Some(it) = arg_iter {
        let mut it = it;
        if it.next().is_some() {
            return Err("not all arguments converted during bytes formatting".to_string());
        }
    }
    Ok(result)
}
