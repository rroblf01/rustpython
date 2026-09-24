use crate::object::*;
use num_traits::{Signed, ToPrimitive};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicI64;

thread_local! {
    pub(crate) static CODEC_SEARCH_FUNCTIONS: std::cell::RefCell<Vec<crate::object::PyObjectRef>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    // Pending search-function registrations deferred from re-entrant
    // `codecs.register()` calls made WHILE a lookup is iterating the
    // list (borrow held) — e.g. a search function re-executing
    // `encodings/__init__.py` via `__import__`, which calls
    // `codecs.register()` again. Pushing directly would need
    // `borrow_mut()` and panic ("RefCell already borrowed"); cloning
    // the list instead caused unexplained segfaults (test_dis), so
    // deferral it is. Drained into the main list after each lookup.
    pub(crate) static PENDING_SEARCH_FUNCTIONS: std::cell::RefCell<Vec<crate::object::PyObjectRef>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Move deferred registrations into the main search list. Call after
/// lookup iteration ends (borrow released).
pub(crate) fn drain_pending_search_functions() {
    let pending: Vec<crate::object::PyObjectRef> =
        PENDING_SEARCH_FUNCTIONS.with(|p| p.borrow_mut().drain(..).collect());
    if pending.is_empty() {
        return;
    }
    CODEC_SEARCH_FUNCTIONS.with(|fns| {
        if let Ok(mut g) = fns.try_borrow_mut() {
            for f in pending {
                g.push(f);
            }
        } else {
            // Still borrowed (nested lookup); put them back for later.
            PENDING_SEARCH_FUNCTIONS.with(|p| {
                for f in pending {
                    p.borrow_mut().push(f);
                }
            });
        }
    });
}

thread_local! {
    // Shared codec error-handler registry (`codecs.register_error` /
    // `codecs.lookup_error` / `_codecs._unregister_error` all operate on
    // this) — real CPython keeps it in `_codecs`; this interpreter's
    // Lib/codecs.py delegates to these natives.
    pub(crate) static CODEC_ERROR_HANDLERS: std::cell::RefCell<std::collections::HashMap<String, PyObjectRef>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

mod helpers;
mod registry;
pub use registry::lookup_codec;

// ── charmap helpers ───────────────────────────────────────────────────────

fn _codecs_charmap_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "charmap_decode() requires at least 1 argument",
        ));
    }
    let data: Vec<u8> = match &*args[0].borrow() {
        PyObject::Bytes(b) | PyObject::ByteArray(b) => b.clone(),
        _ => {
            return Err(PyError::type_error(
                "charmap_decode() argument 1 must be bytes",
            ))
        }
    };
    let errors = if args.len() > 1 && !matches!(&*args[1].borrow(), PyObject::None) {
        args[1].str()
    } else {
        "strict".to_string()
    };
    let mapping_opt: Option<PyObjectRef> = if args.len() > 2 {
        let m = &args[2];
        if matches!(&*m.borrow(), PyObject::None) {
            None
        } else {
            Some(m.clone())
        }
    } else {
        None
    };
    // None => latin-1
    if mapping_opt.is_none() {
        let s: String = data.iter().map(|&b| b as char).collect();
        let len = data.len();
        return Ok(PyObjectRef::new(PyObject::Tuple(vec![
            py_str(&s),
            py_int(len as i64),
        ])));
    }
    let mapping = mapping_opt.unwrap();
    let is_str_mapping = matches!(&*mapping.borrow(), PyObject::Str(_));
    // fast path for string mapping
    let mapping_str_chars: Option<Vec<char>> = if is_str_mapping {
        Some(mapping.str().chars().collect())
    } else {
        None
    };
    let mut out = String::new();
    let mut i = 0usize;
    while i < data.len() {
        let byte = data[i];
        let lookup_result: Option<PyObjectRef> = if let Some(ref chars) = mapping_str_chars {
            if (byte as usize) < chars.len() {
                let ch = chars[byte as usize];
                if ch == '\u{FFFE}' {
                    None
                } else {
                    // single char string
                    Some(py_str(&ch.to_string()))
                }
            } else {
                None
            }
        } else if matches!(&*mapping.borrow(), PyObject::Dict(_)) {
            let dict_borrow = mapping.borrow();
            if let PyObject::Dict(d) = &*dict_borrow {
                let key = py_int(byte as i64);
                match d.get(&key) {
                    Ok(Some(v)) => {
                        let vt = v.borrow();
                        match &*vt {
                            PyObject::None => None,
                            PyObject::Int(n) => {
                                let val = n.to_i64().unwrap_or(-1);
                                if val == 0xFFFE || val < 0 || val > 0x10FFFF {
                                    None
                                } else {
                                    drop(vt);
                                    Some(v.clone())
                                }
                            }
                            PyObject::Str(s) => {
                                if s.chars().count() == 1
                                    && s.chars().next().unwrap() as u32 == 0xFFFE
                                {
                                    None
                                } else {
                                    drop(vt);
                                    Some(v.clone())
                                }
                            }
                            _ => {
                                drop(vt);
                                return Err(PyError::type_error(
                                    "character mapping must return integer, None or str",
                                ));
                            }
                        }
                    }
                    Ok(None) => None,
                    Err(e) => return Err(e),
                }
            } else {
                None
            }
        } else {
            // generic mapping via __getitem__
            let key = py_int(byte as i64);
            match crate::object::py_getitem(&mapping, &key) {
                Ok(v) => {
                    let vt = v.borrow();
                    match &*vt {
                        PyObject::None => None,
                        PyObject::Int(n) => {
                            let val = n.to_i64().unwrap_or(-1);
                            if val == 0xFFFE || val < 0 || val > 0x10FFFF {
                                None
                            } else {
                                drop(vt);
                                Some(v.clone())
                            }
                        }
                        PyObject::Str(s) => {
                            if s.chars().count() == 1 && s.chars().next().unwrap() as u32 == 0xFFFE
                            {
                                None
                            } else {
                                drop(vt);
                                Some(v.clone())
                            }
                        }
                        _ => {
                            drop(vt);
                            return Err(PyError::type_error(
                                "character mapping must return integer, None or str",
                            ));
                        }
                    }
                }
                Err(_) => None,
            }
        };
        if let Some(v) = lookup_result {
            let b = v.borrow();
            match &*b {
                PyObject::Str(s) => {
                    // empty string means deletion, multi-char is inserted
                    out.push_str(s);
                }
                PyObject::Int(n) => {
                    let cp = n.to_u64().unwrap_or(0) as u32;
                    if let Some(ch) = char::from_u32(cp) {
                        out.push(ch);
                    } else {
                        return Err(PyError::type_error(
                            "character mapping must be in range(0x110000)",
                        ));
                    }
                }
                _ => unreachable!(),
            }
            drop(b);
            i += 1;
            continue;
        }
        // undefined mapping
        match errors.as_str() {
            "strict" => {
                return Err(PyError::Exception(
                    "UnicodeDecodeError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "UnicodeDecodeError".to_string(),
                        args: vec![
                            py_str("charmap"),
                            PyObjectRef::imm(PyObject::Bytes(data.clone())),
                            py_int(i as i64),
                            py_int((i + 1) as i64),
                            py_str("character maps to <undefined>"),
                        ],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    }),
                ));
            }
            "ignore" => {
                i += 1;
                continue;
            }
            "replace" => {
                out.push('\u{FFFD}');
                i += 1;
                continue;
            }
            "backslashreplace" => {
                out.push_str(&format!("\\x{:02x}", byte));
                i += 1;
                continue;
            }
            other => {
                // try to delegate to registered error handler
                let handler_opt = CODEC_ERROR_HANDLERS.with(|h| h.borrow().get(other).cloned());
                if let Some(handler) = handler_opt {
                    let exc = PyObjectRef::new(PyObject::Exception {
                        typ: "UnicodeDecodeError".to_string(),
                        args: vec![
                            py_str("charmap"),
                            PyObjectRef::imm(PyObject::Bytes(data.clone())),
                            py_int(i as i64),
                            py_int((i + 1) as i64),
                            py_str("character maps to <undefined>"),
                        ],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    });
                    match crate::object::call_function_disposable(&handler, vec![exc], vec![]) {
                        Ok(res) => {
                            // res is (replacement, newpos)
                            let (rep, newpos) = {
                                let br = res.borrow();
                                if let PyObject::Tuple(t) = &*br {
                                    if t.len() >= 2 {
                                        (
                                            t[0].str(),
                                            t[1].as_i64().unwrap_or((i + 1) as i64) as usize,
                                        )
                                    } else {
                                        (String::new(), i + 1)
                                    }
                                } else {
                                    (res.str(), i + 1)
                                }
                            };
                            out.push_str(&rep);
                            i = newpos;
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                } else {
                    return Err(PyError::value_error(format!(
                        "unknown error handler: {}",
                        other
                    )));
                }
            }
        }
    }
    let len = data.len();
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        py_str(&out),
        py_int(len as i64),
    ])))
}

fn _codecs_charmap_encode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "charmap_encode() requires at least 1 argument",
        ));
    }
    let s = args[0].str();
    let errors = if args.len() > 1 && !matches!(&*args[1].borrow(), PyObject::None) {
        args[1].str()
    } else {
        "strict".to_string()
    };
    let mapping_opt: Option<PyObjectRef> = if args.len() > 2 {
        let m = &args[2];
        if matches!(&*m.borrow(), PyObject::None) {
            None
        } else {
            Some(m.clone())
        }
    } else {
        None
    };
    if mapping_opt.is_none() {
        // default latin1
        let mut out = Vec::with_capacity(s.len());
        for (idx, ch) in s.chars().enumerate() {
            let cp = ch as u32;
            if cp > 0xFF {
                match errors.as_str() {
                    "strict" => {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str("charmap"),
                                    py_str(&s),
                                    py_int(idx as i64),
                                    py_int((idx + 1) as i64),
                                    py_str("character maps to <undefined>"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    }
                    "ignore" => continue,
                    "replace" => out.push(b'?'),
                    "backslashreplace" => {
                        let esc = if cp < 0x100 {
                            format!("\\x{:02x}", cp)
                        } else if cp < 0x10000 {
                            format!("\\u{:04x}", cp)
                        } else {
                            format!("\\U{:08x}", cp)
                        };
                        out.extend_from_slice(esc.as_bytes());
                    }
                    _ => {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str("charmap"),
                                    py_str(&s),
                                    py_int(idx as i64),
                                    py_int((idx + 1) as i64),
                                    py_str("character maps to <undefined>"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    }
                }
            } else {
                out.push(cp as u8);
            }
        }
        let len = s.chars().count();
        return Ok(PyObjectRef::new(PyObject::Tuple(vec![
            PyObjectRef::imm(PyObject::Bytes(out)),
            py_int(len as i64),
        ])));
    }
    let mapping = mapping_opt.unwrap();
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<u8> = Vec::new();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        let cp = ch as u32;
        // lookup mapping: key is int(cp)
        let key = py_int(cp as i64);
        let lookup: Option<PyObjectRef> = if matches!(&*mapping.borrow(), PyObject::Dict(_)) {
            let dict_borrow = mapping.borrow();
            if let PyObject::Dict(d) = &*dict_borrow {
                match d.get(&key) {
                    Ok(Some(v)) => Some(v.clone()),
                    Ok(None) => None,
                    Err(e) => return Err(e),
                }
            } else {
                None
            }
        } else {
            match crate::object::py_getitem(&mapping, &key) {
                Ok(v) => Some(v),
                Err(_) => None,
            }
        };
        if let Some(v) = lookup {
            let vb = v.borrow();
            match &*vb {
                PyObject::None => {
                    // undefined
                    drop(vb);
                }
                PyObject::Int(n) => {
                    let val = n.to_i64().unwrap_or(-1);
                    if val < 0 || val > 255 {
                        drop(vb);
                        return Err(PyError::type_error(
                            "character mapping must be in range(256)",
                        ));
                    }
                    out.push(val as u8);
                    drop(vb);
                    idx += 1;
                    continue;
                }
                PyObject::Bytes(b) => {
                    let cloned = b.clone();
                    drop(vb);
                    out.extend_from_slice(&cloned);
                    idx += 1;
                    continue;
                }
                PyObject::ByteArray(b) => {
                    let cloned = b.clone();
                    drop(vb);
                    out.extend_from_slice(&cloned);
                    idx += 1;
                    continue;
                }
                _ => {
                    drop(vb);
                    return Err(PyError::type_error(
                        "character mapping must return integer, bytes or None, not str",
                    ));
                }
            }
            // fell through => undefined (None)
        }
        // undefined mapping
        match errors.as_str() {
            "strict" => {
                return Err(PyError::Exception(
                    "UnicodeEncodeError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "UnicodeEncodeError".to_string(),
                        args: vec![
                            py_str("charmap"),
                            py_str(&s),
                            py_int(idx as i64),
                            py_int((idx + 1) as i64),
                            py_str("character maps to <undefined>"),
                        ],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    }),
                ));
            }
            "ignore" => {
                idx += 1;
                continue;
            }
            "replace" => {
                // try to lookup '?' mapping
                let qkey = py_int('?' as i64);
                let rep_opt: Option<PyObjectRef> =
                    if matches!(&*mapping.borrow(), PyObject::Dict(_)) {
                        let db = mapping.borrow();
                        if let PyObject::Dict(d) = &*db {
                            d.get(&qkey).ok().flatten()
                        } else {
                            None
                        }
                    } else {
                        crate::object::py_getitem(&mapping, &qkey).ok()
                    };
                if let Some(rv) = rep_opt {
                    let rbb = rv.borrow();
                    match &*rbb {
                        PyObject::Int(n) => {
                            let val = n.to_i64().unwrap_or(-1);
                            if val >= 0 && val <= 255 {
                                out.push(val as u8);
                            } else {
                                out.push(b'?');
                            }
                        }
                        PyObject::Bytes(b) => out.extend_from_slice(b),
                        _ => out.push(b'?'),
                    }
                } else {
                    out.push(b'?');
                }
                idx += 1;
                continue;
            }
            "backslashreplace" | "xmlcharrefreplace" => {
                let esc = if cp < 0x100 {
                    format!("\\x{:02x}", cp)
                } else if cp < 0x10000 {
                    format!("\\u{:04x}", cp)
                } else {
                    format!("\\U{:08x}", cp)
                };
                out.extend_from_slice(esc.as_bytes());
                idx += 1;
                continue;
            }
            other => {
                let handler_opt = CODEC_ERROR_HANDLERS.with(|h| h.borrow().get(other).cloned());
                if let Some(handler) = handler_opt {
                    let exc = PyObjectRef::new(PyObject::Exception {
                        typ: "UnicodeEncodeError".to_string(),
                        args: vec![
                            py_str("charmap"),
                            py_str(&s),
                            py_int(idx as i64),
                            py_int((idx + 1) as i64),
                            py_str("character maps to <undefined>"),
                        ],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    });
                    match crate::object::call_function_disposable(&handler, vec![exc], vec![]) {
                        Ok(res) => {
                            let (rep_str, newpos) = {
                                let br = res.borrow();
                                if let PyObject::Tuple(t) = &*br {
                                    if t.len() >= 2 {
                                        (
                                            t[0].str(),
                                            t[1].as_i64().unwrap_or((idx + 1) as i64) as usize,
                                        )
                                    } else {
                                        (res.str(), idx + 1)
                                    }
                                } else {
                                    (res.str(), idx + 1)
                                }
                            };
                            out.extend_from_slice(rep_str.as_bytes());
                            idx = newpos;
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                } else {
                    return Err(PyError::value_error(format!(
                        "unknown error handler: {}",
                        other
                    )));
                }
            }
        }
    }
    let len = chars.len();
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        PyObjectRef::imm(PyObject::Bytes(out)),
        py_int(len as i64),
    ])))
}

/// Create the `_codecs` module dictionary.
pub fn create_codecs_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // CPython's `PyCodec_Init` registers the standard error handlers in its
    // module init; do the same here, BEFORE the dict is handed out (the
    // vendored `Lib/codecs.py` reads them back at import time).
    register_standard_error_handlers();
    d.insert_str(
        "lookup_error",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "lookup_error".to_string(),
            func: helpers::_codecs_lookup_error,
        }),
    );
    d.insert_str(
        "_register_error",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_register_error".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "_register_error() requires at least 2 arguments",
                    ));
                }
                helpers::_codecs_register_error(&args[0].str(), args[1].clone());
                Ok(py_none())
            },
        }),
    );
    d.insert_str(
        "_unregister_error",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_unregister_error".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 1 {
                    return Err(PyError::type_error(
                        "_unregister_error() requires at least 1 argument",
                    ));
                }
                let name = args[0].str().to_lowercase();
                if matches!(
                    name.as_str(),
                    "strict"
                        | "ignore"
                        | "replace"
                        | "backslashreplace"
                        | "namereplace"
                        | "xmlcharrefreplace"
                        | "surrogateescape"
                        | "surrogatepass"
                ) {
                    return Err(PyError::value_error(format!(
                        "cannot unregister builtin error handler '{}'",
                        name
                    )));
                }
                let removed = CODEC_ERROR_HANDLERS.with(|h| h.borrow_mut().remove(&name));
                Ok(py_bool(removed.is_some()))
            },
        }),
    );
    d.insert_str(
        "lookup",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "lookup".to_string(),
            func: helpers::_codecs_lookup,
        }),
    );
    d.insert_str(
        "encode",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "encode".to_string(),
            func: helpers::_codecs_encode_func,
        }),
    );
    d.insert_str(
        "decode",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "decode".to_string(),
            func: helpers::_codecs_decode_func,
        }),
    );
    d.insert_str(
        "register",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "register".to_string(),
            func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if args.len() < 1 {
                    return Err(PyError::type_error(
                        "register() requires at least 1 argument",
                    ));
                }
                // Never panic on re-entrant registration (search function
                // calling register during lookup iteration): defer to the
                // pending list, drained after the lookup ends.
                CODEC_SEARCH_FUNCTIONS.with(|fns| match fns.try_borrow_mut() {
                    Ok(mut g) => {
                        g.push(args[0].clone());
                    }
                    Err(_) => {
                        PENDING_SEARCH_FUNCTIONS.with(|p| {
                            p.borrow_mut().push(args[0].clone());
                        });
                    }
                });
                Ok(py_none())
            },
        }),
    );
    d.insert_str(
        "unregister",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "unregister".to_string(),
            func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if args.len() < 1 {
                    return Err(PyError::type_error(
                        "unregister() requires at least 1 argument",
                    ));
                }
                CODEC_SEARCH_FUNCTIONS.with(|fns| {
                    fns.borrow_mut().retain(|f| !f.is(&args[0]));
                });
                Ok(py_none())
            },
        }),
    );
    macro_rules! insert_codec_fn {
        ($name:literal, $func:path) => {
            d.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    insert_codec_fn!("utf_7_encode", helpers::utf_7_encode);
    insert_codec_fn!("utf_7_decode", helpers::utf_7_decode);
    insert_codec_fn!("unicode_escape_encode", helpers::unicode_escape_encode);
    insert_codec_fn!("unicode_escape_decode", helpers::unicode_escape_decode);
    insert_codec_fn!(
        "raw_unicode_escape_encode",
        helpers::raw_unicode_escape_encode
    );
    insert_codec_fn!(
        "raw_unicode_escape_decode",
        helpers::raw_unicode_escape_decode
    );
    insert_codec_fn!("escape_encode", helpers::escape_encode);
    insert_codec_fn!("escape_decode", helpers::escape_decode);
    insert_codec_fn!("utf_8_encode", helpers::_codecs_encode);
    insert_codec_fn!("utf_8_decode", helpers::_codecs_decode);
    insert_codec_fn!("ascii_encode", helpers::_codecs_encode);
    insert_codec_fn!("ascii_decode", helpers::_codecs_decode);
    insert_codec_fn!("latin_1_encode", helpers::_codecs_encode);
    insert_codec_fn!("latin_1_decode", helpers::_codecs_decode);
    insert_codec_fn!("utf_16_encode", helpers::_codecs_utf_16_encode);
    insert_codec_fn!("utf_16_decode", helpers::_codecs_utf_16_decode);
    insert_codec_fn!("utf_16_le_encode", helpers::_codecs_utf_16_le_encode);
    insert_codec_fn!("utf_16_le_decode", helpers::_codecs_utf_16_le_decode);
    insert_codec_fn!("utf_16_be_encode", helpers::_codecs_utf_16_be_encode);
    insert_codec_fn!("utf_16_be_decode", helpers::_codecs_utf_16_be_decode);
    insert_codec_fn!("utf_16_ex_decode", helpers::_codecs_utf_16_ex_decode);
    insert_codec_fn!("utf_32_encode", helpers::_codecs_utf_32_encode);
    insert_codec_fn!("utf_32_decode", helpers::_codecs_utf_32_decode);
    insert_codec_fn!("utf_32_le_encode", helpers::_codecs_utf_32_le_encode);
    insert_codec_fn!("utf_32_le_decode", helpers::_codecs_utf_32_le_decode);
    insert_codec_fn!("utf_32_be_encode", helpers::_codecs_utf_32_be_encode);
    insert_codec_fn!("utf_32_be_decode", helpers::_codecs_utf_32_be_decode);
    insert_codec_fn!("utf_32_ex_decode", helpers::_codecs_utf_32_ex_decode);
    d.insert_str(
        "charmap_encode",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "charmap_encode".to_string(),
            func: _codecs_charmap_encode,
        }),
    );
    d.insert_str(
        "charmap_decode",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "charmap_decode".to_string(),
            func: _codecs_charmap_decode,
        }),
    );
    d.insert_str(
        "charmap_build",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "charmap_build".to_string(),
            func: registry::_codecs_charmap_build,
        }),
    );

    fn err_bounds(exc: &PyObjectRef) -> (usize, usize, Option<PyObjectRef>) {
        let getattr = |name: &str| -> Option<PyObjectRef> { exc.borrow().get_attribute(name).ok() };
        let end = getattr("end").and_then(|e| e.as_i64()).unwrap_or(0) as usize;
        let obj = getattr("object");
        let start = getattr("start").and_then(|e| e.as_i64()).unwrap_or(0) as usize;
        (start, end, obj)
    }
    fn err_object_str(obj: &Option<PyObjectRef>) -> String {
        obj.as_ref().map(|o| o.str()).unwrap_or_default()
    }
    fn backslashreplace_impl(exc: &PyObjectRef) -> PyResult<PyObjectRef> {
        let (start, end, obj) = err_bounds(exc);
        let s = err_object_str(&obj);
        let chars: Vec<char> = s.chars().collect();
        let slice = &chars[start.min(chars.len())..end.min(chars.len())];
        let mut out = String::new();
        for &ch in slice {
            let cp = ch as u32;
            if cp < 0x100 {
                out.push_str(&format!("\\x{:02x}", cp));
            } else if cp < 0x10000 {
                out.push_str(&format!("\\u{:04x}", cp));
            } else {
                out.push_str(&format!("\\U{:08x}", cp));
            }
        }
        Ok(py_tuple(vec![py_str(&out), py_int(end as i64)]))
    }
    fn xmlcharrefreplace_impl(exc: &PyObjectRef) -> PyResult<PyObjectRef> {
        let (start, end, obj) = err_bounds(exc);
        let s = err_object_str(&obj);
        let chars: Vec<char> = s.chars().collect();
        let slice = &chars[start.min(chars.len())..end.min(chars.len())];
        let mut out = String::new();
        for &ch in slice {
            let cp = ch as u32;
            if cp < 0x100 {
                out.push_str(&format!("&#{};", cp));
            } else {
                out.push_str(&format!("&#x{:x};", cp));
            }
        }
        Ok(py_tuple(vec![py_str(&out), py_int(end as i64)]))
    }
    fn surrogateescape_impl(exc: &PyObjectRef) -> PyResult<PyObjectRef> {
        let (start, end, obj) = err_bounds(exc);
        let raw = obj
            .as_ref()
            .map(|o| {
                let b = o.borrow();
                if let PyObject::Bytes(v) = &*b {
                    v.clone()
                } else {
                    vec![]
                }
            })
            .unwrap_or_default();
        let mut out: Vec<u8> = Vec::new();
        for byte in &raw[start.min(raw.len())..end.min(raw.len())] {
            let ch = 0xDC00u32 | (*byte as u32);
            out.extend_from_slice(&ch.to_string().into_bytes());
        }
        Ok(py_tuple(vec![
            py_str(&String::from_utf8_lossy(&out)),
            py_int(end as i64),
        ]))
    }
    fn surrogatepass_impl(exc: &PyObjectRef) -> PyResult<PyObjectRef> {
        let (start, end, obj) = err_bounds(exc);
        let s = err_object_str(&obj);
        let chars: Vec<char> = s.chars().collect();
        let slice: String = chars[start.min(chars.len())..end.min(chars.len())]
            .iter()
            .collect();
        Ok(py_tuple(vec![py_str(&slice), py_int(end as i64)]))
    }
    d.insert_str(
        "backslashreplace_errors",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "backslashreplace_errors".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "backslashreplace_errors() missing argument",
                    ));
                }
                backslashreplace_impl(&args[0])
            },
        }),
    );
    d.insert_str(
        "xmlcharrefreplace_errors",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "xmlcharrefreplace_errors".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "xmlcharrefreplace_errors() missing argument",
                    ));
                }
                xmlcharrefreplace_impl(&args[0])
            },
        }),
    );
    d.insert_str(
        "surrogateescape_errors",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "surrogateescape_errors".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "surrogateescape_errors() missing argument",
                    ));
                }
                surrogateescape_impl(&args[0])
            },
        }),
    );
    d.insert_str(
        "surrogatepass_errors",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "surrogatepass_errors".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "surrogatepass_errors() missing argument",
                    ));
                }
                surrogatepass_impl(&args[0])
            },
        }),
    );
    d
}

/// The standard codec error handlers, registered natively exactly like
/// CPython's `PyCodec_Init` does (`Python/codecs.c`): the vendored
/// `Lib/codecs.py` only *reads* them back
/// (`strict_errors = lookup_error("strict")`), so without this registration
/// at startup EVERY import that touches `codecs`' error-handler section
/// raises `LookupError: unknown error handler: 'strict'` — which took down
/// `traceback`, `tracemalloc`, `ftplib` and more the moment their real
/// CPython sources were vendored in.
///
/// Each handler receives the `UnicodeEncodeError`/`UnicodeDecodeError`
/// object and returns `(replacement, new_position)`, per PEP 383 /
/// `codecs.register_error`'s contract.
fn register_standard_error_handlers() {
    for name in [
        "strict",
        "ignore",
        "replace",
        "xmlcharrefreplace",
        "backslashreplace",
        "namereplace",
        "surrogateescape",
        "surrogatepass",
    ] {
        let kind = name.to_string();
        let handler: PyObjectRef = PyObjectRef::new(PyObject::Closure(Rc::new(
            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                codec_error_handler(&kind, args)
            },
        )));
        helpers::_codecs_register_error(name, handler);
    }
}

/// Read `(object, start, end)` out of a Unicode(Encode|Decode)Error instance
/// (its `args` tuple is `(encoding, object, start, end, reason)`).
fn codec_error_parts(exc: &PyObjectRef) -> (PyObjectRef, i64, i64) {
    let args = match &*exc.borrow() {
        PyObject::Exception { args, .. } => args.clone(),
        PyObject::Tuple(items) => items.clone(),
        _ => vec![],
    };
    let object = args.get(1).cloned().unwrap_or_else(|| py_str(""));
    let start = args.get(2).and_then(|v| v.as_i64()).unwrap_or(0);
    let end = args.get(3).and_then(|v| v.as_i64()).unwrap_or(start + 1);
    (object, start, end)
}

fn codec_error_handler(kind: &str, args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "codec error handler requires 1 argument",
        ));
    }
    let exc = args[0].clone();
    let (object, start, end) = codec_error_parts(&exc);
    let exc_type = match &*exc.borrow() {
        PyObject::Exception { typ, .. } => typ.clone(),
        _ => "UnicodeDecodeError".to_string(),
    };
    let is_encode = exc_type == "UnicodeEncodeError";
    let count = (end - start).max(0) as usize;
    match kind {
        // `strict` re-raises the very exception it was handed (CPython's
        // `codec_error_strict`).
        "strict" => Err(PyError::Exception(exc_type, exc)),
        "ignore" => Ok(py_tuple(vec![py_str(""), py_int(end)])),
        "replace" => {
            let rep = if is_encode {
                "?".repeat(count)
            } else {
                "\u{fffd}".to_string()
            };
            Ok(py_tuple(vec![py_str(&rep), py_int(end)]))
        }
        "xmlcharrefreplace" => {
            let mut rep = String::new();
            if is_encode {
                for ch in object.str().chars().skip(start as usize).take(count) {
                    rep.push_str(&format!("&#{};", ch as u32));
                }
            } else {
                let bytes = match &*object.borrow() {
                    PyObject::Bytes(b) => b.clone(),
                    _ => Vec::new(),
                };
                for byte in bytes.iter().skip(start as usize).take(count) {
                    rep.push_str(&format!("&#{};", byte));
                }
            }
            Ok(py_tuple(vec![py_str(&rep), py_int(end)]))
        }
        "backslashreplace" => {
            let mut rep = String::new();
            if is_encode {
                // Encoding direction: the failing characters become
                // `\xNN` escapes of their UTF-8 bytes.
                for ch in object.str().chars().skip(start as usize).take(count) {
                    let mut buf = [0u8; 4];
                    for byte in ch.encode_utf8(&mut buf).as_bytes() {
                        rep.push_str(&format!("\\x{:02x}", byte));
                    }
                }
            } else {
                let bytes = match &*object.borrow() {
                    PyObject::Bytes(b) => b.clone(),
                    _ => Vec::new(),
                };
                for byte in bytes.iter().skip(start as usize).take(count) {
                    match byte {
                        0x0c => rep.push_str("\\f"),
                        0x0a => rep.push_str("\\n"),
                        0x0d => rep.push_str("\\r"),
                        0x09 => rep.push_str("\\t"),
                        0x0b => rep.push_str("\\v"),
                        0x08 => rep.push_str("\\b"),
                        0x07 => rep.push_str("\\a"),
                        b if *b < 0x20 || *b >= 0x7f => rep.push_str(&format!("\\x{:02x}", b)),
                        b => rep.push(*b as char),
                    }
                }
            }
            Ok(py_tuple(vec![py_str(&rep), py_int(end)]))
        }
        "namereplace" => {
            let mut rep = String::new();
            for ch in object.str().chars().skip(start as usize).take(count) {
                match crate::unicode_names::unicode_name_of(ch) {
                    Some(name) => rep.push_str(&format!("\\N{{{}}}", name)),
                    None => rep.push_str(&format!("\\u{:04x}", ch as u32)),
                }
            }
            Ok(py_tuple(vec![py_str(&rep), py_int(end)]))
        }
        // PEP 383: an undecodable byte 0x80-0xFF round-trips as U+DC80..U+DCFF
        // (and back); other positions are left alone.
        "surrogateescape" => {
            let bytes = match &*object.borrow() {
                PyObject::Bytes(b) => b.clone(),
                _ => Vec::new(),
            };
            let mut rep = String::new();
            for byte in bytes.iter().skip(start as usize).take(count) {
                if (0x80..=0xff).contains(byte) {
                    rep.push(char::from_u32(0xdc00 + (*byte as u32 - 0x80)).unwrap_or('\u{fffd}'));
                } else {
                    return Err(PyError::Exception(exc_type, exc));
                }
            }
            Ok(py_tuple(vec![py_str(&rep), py_int(end)]))
        }
        "surrogatepass" => Ok(py_tuple(vec![
            object.clone(),
            py_int(object.borrow().str().chars().count() as i64),
        ])),
        other => Err(PyError::value_error(format!(
            "unknown error handler: {}",
            other
        ))),
    }
}
