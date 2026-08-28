// Auto-extracted from src/object/attrs/mod.rs lines 2057-2820
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Bytes(_v) => {
                match name {
                    "__mul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__mul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__rmul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__rmul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__rmul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(bytes) = &*args[0].borrow() {
                                Ok(py_int(33 + bytes.len() as i64))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__contains__() takes exactly one argument",
                                ));
                            }
                            py_contains(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__repr__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__repr__".to_string(),
                        func: |args| Ok(py_str(&args[0].repr())),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__str__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__str__".to_string(),
                        func: |args| Ok(py_str(&args[0].str())),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__len__".to_string(),
                        func: |args| {
                            let b = args[0].borrow();
                            if let PyObject::Bytes(bytes) = &*b {
                                Ok(py_int(bytes.len() as i64))
                            } else {
                                Err(PyError::runtime_error("__len__ on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__mod__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mod__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error("__mod__() too few args"));
                            }
                            let fmt = match &*args[1].borrow() {
                                PyObject::Bytes(b) => b.clone(),
                                _ => return Err(PyError::runtime_error("__mod__ on non-bytes")),
                            };
                            let result = bytes_interpolate(&fmt, &args[2]).map_err(|e| {
                                if e.contains("too big") || e.contains("[overflow]") {
                                    PyError::overflow_error(
                                        e.trim_end_matches(" [overflow]").to_string(),
                                    )
                                } else if e.contains("unsupported format character")
                                    || e == "incomplete format"
                                {
                                    PyError::value_error(e)
                                } else {
                                    PyError::type_error(e)
                                }
                            })?;
                            Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "hex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "hex".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(bytes) = &*args[0].borrow() {
                                let hex: String =
                                    bytes.iter().map(|b| format!("{:02x}", b)).collect();
                                Ok(py_str(&hex))
                            } else {
                                Err(PyError::runtime_error("hex on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "decode" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "decode".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(bytes) = &*args[0].borrow() {
                                let encoding = if args.len() > 1 {
                                    args[1].str()
                                } else {
                                    "utf-8".to_string()
                                };
                                let errors = if args.len() > 2 {
                                    args[2].str()
                                } else {
                                    "strict".to_string()
                                };
                                if encoding == "utf-8" || encoding == "utf8" {
                                    match String::from_utf8(bytes.clone()) {
                                        Ok(s) => Ok(py_str(&s)),
                                        Err(e) if errors == "strict" => {
                                            // A real `UnicodeDecodeError` (not a
                                            // bare `ValueError`, its ancestor —
                                            // real code commonly catches the
                                            // specific subclass, e.g. CPython's
                                            // own `test.support.os_helper`
                                            // probing filesystem-encoding
                                            // behavior via `except
                                            // UnicodeDecodeError:`) so real
                                            // CPython-idiomatic error handling
                                            // around `.decode()` actually works.
                                            let pos = e.utf8_error().valid_up_to();
                                            Err(PyError::Exception(
                                                "UnicodeDecodeError".to_string(),
                                                PyObjectRef::new(PyObject::Exception {
                                                    typ: "UnicodeDecodeError".to_string(),
                                                    args: vec![
                                                        py_str(&encoding),
                                                        PyObjectRef::imm(PyObject::Bytes(
                                                            bytes.clone(),
                                                        )),
                                                        py_int(pos as i64),
                                                        py_int(pos as i64 + 1),
                                                        py_str("invalid start byte"),
                                                    ],
                                                    cause: None,
                                                    suppress_context: false,
                                                    context: None,
                                                    traceback: None,
                                                    extra: None,
                                                }),
                                            ))
                                        }
                                        Err(_) => {
                                            // 'ignore'/'replace'/'surrogateescape'/etc:
                                            // this interpreter's `PyObject::Str` is
                                            // backed by a real Rust `String`
                                            // (always valid UTF-8), so it can't
                                            // represent lone surrogates the way
                                            // real `surrogateescape` round-trips
                                            // require — lossy replacement is the
                                            // closest approximation available.
                                            Ok(py_str(&String::from_utf8_lossy(bytes)))
                                        }
                                    }
                                } else {
                                    let norm = encoding.to_ascii_lowercase().replace('-', "_").replace('_', "-");
                                    // latin-1 family
                                    if norm == "latin-1" || norm == "latin1" || norm == "iso-8859-1" || norm == "iso8859-1" || norm == "l1" || norm == "8859" || norm == "cp819" {
                                        let s: String = bytes.iter().map(|&b| b as char).collect();
                                        return Ok(py_str(&s));
                                    }
                                    // ascii
                                    if norm == "ascii" || norm == "us-ascii" || norm == "646" {
                                        let mut out = String::new();
                                        let mut has_error = false;
                                        for (i, &b) in bytes.iter().enumerate() {
                                            if b <= 0x7F {
                                                out.push(b as char);
                                            } else {
                                                match errors.as_str() {
                                                    "strict" => {
                                                        return Err(PyError::Exception(
                                                            "UnicodeDecodeError".to_string(),
                                                            PyObjectRef::new(PyObject::Exception {
                                                                typ: "UnicodeDecodeError".to_string(),
                                                                args: vec![
                                                                    py_str(&encoding),
                                                                    PyObjectRef::imm(PyObject::Bytes(bytes.clone())),
                                                                    py_int(i as i64),
                                                                    py_int((i+1) as i64),
                                                                    py_str("ordinal not in range(128)"),
                                                                ],
                                                                cause: None,
                                                                suppress_context: false,
                                                                context: None,
                                                                traceback: None,
                                                                extra: None,
                                                            }),
                                                        ));
                                                    }
                                                    "ignore" => {},
                                                    "replace" => out.push('\u{FFFD}'),
                                                    "surrogateescape" => {
                                                        // Map bytes 128-255 to code points 128-255 (valid UTF-8, round-trips via latin-1 style)
                                                        // This avoids needing to store lone surrogates which are invalid in Rust's String.
                                                        out.push(b as char);
                                                    }
                                                    "backslashreplace" => {
                                                        out.push_str(&format!("\\x{:02x}", b));
                                                    }
                                                    "xmlcharrefreplace" => {
                                                        out.push_str(&format!("&#{};", b));
                                                    }
                                                    _ => {
                                                        has_error = true;
                                                        out.push('\u{FFFD}');
                                                    }
                                                }
                                            }
                                        }
                                        if !has_error || errors == "surrogateescape" || errors == "ignore" || errors == "replace" || errors == "backslashreplace" || errors == "xmlcharrefreplace" {
                                            return Ok(py_str(&out));
                                        }
                                        // for other handlers, try generic handling below; fallback to lossy if needed
                                    }
                                    // Try generic codec lookup (e.g. testcodec)
                                    if let Some(codec_tuple) = crate::modules::lookup_codec(&encoding) {
                                        let decode_fn = {
                                            let tup = codec_tuple.borrow();
                                            if let PyObject::Tuple(items) = &*tup {
                                                items.get(1).cloned()
                                            } else {
                                                // try attribute
                                                match tup.get_attribute("decode") {
                                                    Ok(v) => Some(v),
                                                    Err(_) => None,
                                                }
                                            }
                                        };
                                        if let Some(df) = decode_fn {
                                            let bytes_obj = PyObjectRef::imm(PyObject::Bytes(bytes.clone()));
                                            match crate::object::call_function_disposable(&df, vec![bytes_obj, py_str(&errors)], vec![]) {
                                                Ok(res) => {
                                                    let s = {
                                                        let br = res.borrow();
                                                        if let PyObject::Tuple(items) = &*br {
                                                            if !items.is_empty() {
                                                                items[0].str()
                                                            } else {
                                                                res.str()
                                                            }
                                                        } else {
                                                            res.str()
                                                        }
                                                    };
                                                    return Ok(py_str(&s));
                                                }
                                                Err(e) => return Err(e),
                                            }
                                        }
                                    }
                                    // fallback: latin1-like lossy
                                    let s = String::from_utf8_lossy(bytes).to_string();
                                    Ok(py_str(&s))
                                }
                            } else {
                                Err(PyError::runtime_error("decode on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removeprefix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removeprefix".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "removeprefix() takes exactly 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let prefix = args[1].borrow();
                                if let PyObject::Bytes(p) = &*prefix {
                                    if b.starts_with(p.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::Bytes(b[p.len()..].to_vec())))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::Bytes(b.clone())))
                                    }
                                } else {
                                    Err(PyError::type_error(
                                        "removeprefix() argument must be bytes",
                                    ))
                                }
                            } else {
                                Err(PyError::runtime_error("removeprefix on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removesuffix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removesuffix".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "removesuffix() takes exactly 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let suffix = args[1].borrow();
                                if let PyObject::Bytes(s) = &*suffix {
                                    if b.ends_with(s.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::Bytes(
                                            b[..b.len() - s.len()].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::Bytes(b.clone())))
                                    }
                                } else {
                                    Err(PyError::type_error(
                                        "removesuffix() argument must be bytes",
                                    ))
                                }
                            } else {
                                Err(PyError::runtime_error("removesuffix on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "startswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "startswith".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "startswith() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let start = opt_i64_arg(args.get(2));
                                let end = opt_i64_arg(args.get(3));
                                let (st, en) = resolve_str_slice_bounds(b.len(), start, end);
                                let sub = &b[st..en];
                                let prefixes = extract_bytes_or_tuple(&args[1]);
                                Ok(py_bool(
                                    prefixes.iter().any(|p| sub.starts_with(p.as_slice())),
                                ))
                            } else {
                                Err(PyError::runtime_error("startswith on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "endswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "endswith".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "endswith() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let start = opt_i64_arg(args.get(2));
                                let end = opt_i64_arg(args.get(3));
                                let (st, en) = resolve_str_slice_bounds(b.len(), start, end);
                                let sub = &b[st..en];
                                let suffixes = extract_bytes_or_tuple(&args[1]);
                                Ok(py_bool(
                                    suffixes.iter().any(|p| sub.ends_with(p.as_slice())),
                                ))
                            } else {
                                Err(PyError::runtime_error("endswith on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "find" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "find".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "find() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let needle = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                let start = opt_i64_arg(args.get(2));
                                let end = opt_i64_arg(args.get(3));
                                Ok(py_int(
                                    bytes_find_impl(b, &needle, start, end, false)
                                        .map(|i| i as i64)
                                        .unwrap_or(-1),
                                ))
                            } else {
                                Err(PyError::runtime_error("find on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rfind" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rfind".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "rfind() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let needle = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                let start = opt_i64_arg(args.get(2));
                                let end = opt_i64_arg(args.get(3));
                                Ok(py_int(
                                    bytes_find_impl(b, &needle, start, end, true)
                                        .map(|i| i as i64)
                                        .unwrap_or(-1),
                                ))
                            } else {
                                Err(PyError::runtime_error("rfind on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "index".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "index() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let needle = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                let start = opt_i64_arg(args.get(2));
                                let end = opt_i64_arg(args.get(3));
                                bytes_find_impl(b, &needle, start, end, false)
                                    .map(|i| py_int(i as i64))
                                    .ok_or_else(|| PyError::value_error("subsection not found"))
                            } else {
                                Err(PyError::runtime_error("index on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rindex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rindex".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "rindex() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let needle = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                let start = opt_i64_arg(args.get(2));
                                let end = opt_i64_arg(args.get(3));
                                bytes_find_impl(b, &needle, start, end, true)
                                    .map(|i| py_int(i as i64))
                                    .ok_or_else(|| PyError::value_error("subsection not found"))
                            } else {
                                Err(PyError::runtime_error("rindex on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "count() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let needle = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                let start = opt_i64_arg(args.get(2));
                                let end = opt_i64_arg(args.get(3));
                                let (st, en) = resolve_str_slice_bounds(b.len(), start, end);
                                let sub = &b[st..en];
                                let c = if needle.is_empty() {
                                    sub.len() + 1
                                } else {
                                    sub.windows(needle.len())
                                        .filter(|w| *w == needle.as_slice())
                                        .count()
                                };
                                Ok(py_int(c as i64))
                            } else {
                                Err(PyError::runtime_error("count on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "replace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "replace".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "replace() takes at least 2 arguments",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let old = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                let new = arg_bytes(&args[2]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                let maxcount = if args.len() > 3 {
                                    args[3].as_i64().unwrap_or(-1)
                                } else {
                                    -1
                                };
                                if old.is_empty() {
                                    let mut result = new.clone();
                                    for (i, byte) in b.iter().enumerate() {
                                        if maxcount >= 0 && (i as i64) >= maxcount {
                                            result.extend_from_slice(&b[i..]);
                                            return Ok(PyObjectRef::imm(PyObject::Bytes(result)));
                                        }
                                        result.push(*byte);
                                        result.extend_from_slice(&new);
                                    }
                                    return Ok(PyObjectRef::imm(PyObject::Bytes(result)));
                                }
                                let mut result = Vec::new();
                                let mut rest = &b[..];
                                let mut count = 0i64;
                                loop {
                                    if maxcount >= 0 && count >= maxcount {
                                        break;
                                    }
                                    match rest.windows(old.len()).position(|w| w == old.as_slice())
                                    {
                                        Some(idx) => {
                                            result.extend_from_slice(&rest[..idx]);
                                            result.extend_from_slice(&new);
                                            rest = &rest[idx + old.len()..];
                                            count += 1;
                                        }
                                        None => break,
                                    }
                                }
                                result.extend_from_slice(rest);
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("replace on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "split" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "split".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let sep = if args.len() > 1
                                    && !matches!(&*args[1].borrow(), PyObject::None)
                                {
                                    Some(arg_bytes(&args[1]).ok_or_else(|| {
                                        PyError::type_error(
                                            "argument should be a bytes-like object",
                                        )
                                    })?)
                                } else {
                                    None
                                };
                                let maxsplit = if args.len() > 2 {
                                    args[2].as_i64().unwrap_or(-1)
                                } else {
                                    -1
                                };
                                let parts: Vec<Vec<u8>> = match &sep {
                                    Some(sep) => {
                                        if sep.is_empty() {
                                            return Err(PyError::value_error("empty separator"));
                                        }
                                        let mut parts = Vec::new();
                                        let mut rest = &b[..];
                                        let mut count = 0i64;
                                        loop {
                                            if maxsplit >= 0 && count >= maxsplit {
                                                break;
                                            }
                                            match rest
                                                .windows(sep.len())
                                                .position(|w| w == sep.as_slice())
                                            {
                                                Some(idx) => {
                                                    parts.push(rest[..idx].to_vec());
                                                    rest = &rest[idx + sep.len()..];
                                                    count += 1;
                                                }
                                                None => break,
                                            }
                                        }
                                        parts.push(rest.to_vec());
                                        parts
                                    }
                                    None => {
                                        let mut parts: Vec<Vec<u8>> = Vec::new();
                                        let mut rest = &b[..];
                                        loop {
                                            if maxsplit >= 0 && parts.len() >= maxsplit as usize {
                                                break;
                                            }
                                            let ws_start = rest
                                                .iter()
                                                .position(|c| !c.is_ascii_whitespace())
                                                .unwrap_or(rest.len());
                                            rest = &rest[ws_start..];
                                            if rest.is_empty() {
                                                break;
                                            }
                                            let idx = rest
                                                .iter()
                                                .position(|c| c.is_ascii_whitespace())
                                                .unwrap_or(rest.len());
                                            parts.push(rest[..idx].to_vec());
                                            rest = &rest[idx..];
                                        }
                                        let tail_start = rest
                                            .iter()
                                            .position(|c| !c.is_ascii_whitespace())
                                            .unwrap_or(rest.len());
                                        let tail_end = rest
                                            .iter()
                                            .rposition(|c| !c.is_ascii_whitespace())
                                            .map(|i| i + 1)
                                            .unwrap_or(tail_start);
                                        if tail_start < tail_end {
                                            parts.push(rest[tail_start..tail_end].to_vec());
                                        }
                                        parts
                                    }
                                };
                                Ok(py_list(
                                    parts
                                        .into_iter()
                                        .map(|v| PyObjectRef::imm(PyObject::Bytes(v)))
                                        .collect(),
                                ))
                            } else {
                                Err(PyError::runtime_error("split on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rsplit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rsplit".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let sep = if args.len() > 1
                                    && !matches!(&*args[1].borrow(), PyObject::None)
                                {
                                    Some(arg_bytes(&args[1]).ok_or_else(|| {
                                        PyError::type_error(
                                            "argument should be a bytes-like object",
                                        )
                                    })?)
                                } else {
                                    None
                                };
                                let maxsplit = if args.len() > 2 {
                                    args[2].as_i64().unwrap_or(-1)
                                } else {
                                    -1
                                };
                                let parts: Vec<Vec<u8>> = match &sep {
                                    Some(sep) => {
                                        if sep.is_empty() {
                                            return Err(PyError::value_error("empty separator"));
                                        }
                                        let mut parts = Vec::new();
                                        let mut rest = &b[..];
                                        let mut count = 0i64;
                                        loop {
                                            if maxsplit >= 0 && count >= maxsplit {
                                                break;
                                            }
                                            match rest
                                                .windows(sep.len())
                                                .rposition(|w| w == sep.as_slice())
                                            {
                                                Some(idx) => {
                                                    parts.push(rest[idx + sep.len()..].to_vec());
                                                    rest = &rest[..idx];
                                                    count += 1;
                                                }
                                                None => break,
                                            }
                                        }
                                        parts.push(rest.to_vec());
                                        parts.reverse();
                                        parts
                                    }
                                    None => {
                                        let mut parts: Vec<Vec<u8>> = Vec::new();
                                        let mut rest = &b[..];
                                        loop {
                                            if maxsplit >= 0 && parts.len() >= maxsplit as usize {
                                                break;
                                            }
                                            let ws_end = rest
                                                .iter()
                                                .rposition(|c| !c.is_ascii_whitespace())
                                                .map(|i| i + 1)
                                                .unwrap_or(0);
                                            rest = &rest[..ws_end];
                                            if rest.is_empty() {
                                                break;
                                            }
                                            let idx = rest
                                                .iter()
                                                .rposition(|c| c.is_ascii_whitespace())
                                                .map(|i| i + 1)
                                                .unwrap_or(0);
                                            parts.push(rest[idx..].to_vec());
                                            rest = &rest[..idx];
                                        }
                                        let head_start = rest
                                            .iter()
                                            .position(|c| !c.is_ascii_whitespace())
                                            .unwrap_or(rest.len());
                                        let head_end = rest
                                            .iter()
                                            .rposition(|c| !c.is_ascii_whitespace())
                                            .map(|i| i + 1)
                                            .unwrap_or(head_start);
                                        if head_start < head_end {
                                            parts.push(rest[head_start..head_end].to_vec());
                                        }
                                        parts.reverse();
                                        parts
                                    }
                                };
                                Ok(py_list(
                                    parts
                                        .into_iter()
                                        .map(|v| PyObjectRef::imm(PyObject::Bytes(v)))
                                        .collect(),
                                ))
                            } else {
                                Err(PyError::runtime_error("rsplit on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => return super::bytes2::get(o, name),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
