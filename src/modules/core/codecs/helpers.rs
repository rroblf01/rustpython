use crate::object::*;
#[allow(unused_imports)]
use num_traits::{Signed, ToPrimitive};
use std::collections::HashMap;
#[allow(unused_imports)]
use std::rc::Rc;
#[allow(unused_imports)]
use std::sync::atomic::AtomicI64;

/// Encode a string as UTF-8/ASCII/Latin-1 (used by codecs.lookup() results).
pub(crate) fn _codecs_encode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("encode() requires at least 1 argument"));
    }
    let s = args[0].str();
    let len = s.len();
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        PyObjectRef::imm(PyObject::Bytes(s.into_bytes())),
        py_int(len as i64),
    ])))
}

/// Decode bytes as UTF-8 (used by codecs.lookup() results).
pub(crate) fn _codecs_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("decode() requires at least 1 argument"));
    }
    let data = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Bytes(b) => b.clone(),
            _ => return Err(PyError::type_error("decode() argument must be bytes")),
        }
    };
    let s = String::from_utf8(data)
        .map_err(|e| PyError::value_error(format!("decode error: {}", e)))?;
    let len = s.len();
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        py_str(&s),
        py_int(len as i64),
    ])))
}

pub(crate) fn _codecs_reader(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::value_error("stream reader not implemented"))
}

pub(crate) fn _codecs_writer(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::value_error("stream writer not implemented"))
}

pub(crate) fn _codecs_register_error(name: &str, handler: PyObjectRef) {
    super::CODEC_ERROR_HANDLERS.with(|h| {
        h.borrow_mut().insert(name.to_lowercase(), handler);
    });
}

pub(crate) fn _codecs_lookup_error(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error(
            "lookup_error() requires at least 1 argument",
        ));
    }
    let name = args[0].str().to_lowercase();
    let found = super::CODEC_ERROR_HANDLERS.with(|h| h.borrow().get(&name).cloned());
    match found {
        Some(h) => Ok(h),
        None => Err(PyError::Exception(
            "LookupError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "LookupError".to_string(),
                args: vec![py_str(&format!("unknown error handler: '{}'", name))],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )),
    }
}

pub(crate) fn _codecs_lookup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("lookup() requires at least 1 argument"));
    }
    let encoding = args[0].str().to_lowercase().replace('-', "_");
    match encoding.as_str() {
        "utf_8" | "ascii" | "latin_1" | "utf8" => Ok(PyObjectRef::new(PyObject::Tuple(vec![
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "encode".to_string(),
                func: _codecs_encode,
            }),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "decode".to_string(),
                func: _codecs_decode,
            }),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "stream_reader".to_string(),
                func: _codecs_reader,
            }),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "stream_writer".to_string(),
                func: _codecs_writer,
            }),
            py_str(&encoding),
        ]))),
        _ => {
            let result = super::CODEC_SEARCH_FUNCTIONS.with(|fns| {
                for f in fns.borrow().iter() {
                    match crate::object::builtin_call(f, &[py_str(&args[0].str())]) {
                        Ok(res) if !matches!(&*res.borrow(), PyObject::None) => return Some(res),
                        _ => continue,
                    }
                }
                None
            });
            // Borrow released above; fold in re-entrant registrations.
            super::drain_pending_search_functions();
            if let Some(entry) = result {
                return Ok(entry);
            }
            Err(PyError::value_error(format!(
                "unknown encoding: {}",
                encoding
            )))
        }
    }
}

pub(crate) fn _codecs_encode_func(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "encode() requires at least 2 arguments",
        ));
    }
    let s = args[0].str();
    let encoding = args[1].str().to_lowercase().replace('-', "_");
    match encoding.as_str() {
        "utf_8" | "ascii" | "latin_1" | "utf8" => {
            let len = s.len();
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                PyObjectRef::imm(PyObject::Bytes(s.into_bytes())),
                py_int(len as i64),
            ])))
        }
        _ => Err(PyError::value_error(format!(
            "unknown encoding: {}",
            encoding
        ))),
    }
}

/// Shared body for the `_codecs.utf_16*`/`utf_32*` encode natives that
/// `Lib/encodings/utf_16.py`/`utf_32.py` (and their `_be`/`_le` siblings)
/// look up as plain module attributes (`codecs.utf_16_encode`, etc.) —
/// these previously didn't exist at all in `_codecs`, so importing any of
/// those `encodings/` modules (which `codecs.lookup("utf_16")` etc. does
/// via the search-function path) raised `AttributeError` before the
/// `ValueError: unknown encoding` a caller would otherwise see, e.g.
/// `test_codecs.py`. `str.encode(...)` already had working UTF-16/32 logic
/// (`crate::object::encode_utf16_utf32`) wired only into the fast path for
/// `str.encode`/`bytes.decode`, not exposed as callable natives — reuse it.
fn utf16_32_encode_named(args: &[PyObjectRef], norm: &str) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("encode() requires at least 1 argument"));
    }
    let s = args[0].str();
    let bytes = crate::object::encode_utf16_utf32(&s, norm)
        .ok_or_else(|| PyError::value_error(format!("unknown encoding: {}", norm)))?;
    let len = s.chars().count();
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        PyObjectRef::imm(PyObject::Bytes(bytes)),
        py_int(len as i64),
    ])))
}

fn utf16_32_decode_named(args: &[PyObjectRef], norm: &str) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("decode() requires at least 1 argument"));
    }
    let data = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Bytes(b) | PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("decode() argument must be bytes-like")),
        }
    };
    let consumed = data.len();
    let s = crate::object::decode_utf16_utf32(&data, norm)
        .ok_or_else(|| PyError::value_error(format!("unknown encoding: {}", norm)))?;
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        py_str(&s),
        py_int(consumed as i64),
    ])))
}

macro_rules! utf_codec_fn {
    ($enc_name:ident, $dec_name:ident, $norm:expr) => {
        pub(crate) fn $enc_name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            utf16_32_encode_named(args, $norm)
        }
        pub(crate) fn $dec_name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            utf16_32_decode_named(args, $norm)
        }
    };
}

utf_codec_fn!(_codecs_utf_16_encode, _codecs_utf_16_decode, "utf-16");
utf_codec_fn!(
    _codecs_utf_16_le_encode,
    _codecs_utf_16_le_decode,
    "utf-16-le"
);
utf_codec_fn!(
    _codecs_utf_16_be_encode,
    _codecs_utf_16_be_decode,
    "utf-16-be"
);
utf_codec_fn!(_codecs_utf_32_encode, _codecs_utf_32_decode, "utf-32");
utf_codec_fn!(
    _codecs_utf_32_le_encode,
    _codecs_utf_32_le_decode,
    "utf-32-le"
);
utf_codec_fn!(
    _codecs_utf_32_be_encode,
    _codecs_utf_32_be_decode,
    "utf-32-be"
);

/// `codecs.utf_16_ex_decode(input, errors, byteorder, final)` — like
/// `utf_16_decode` but also reports the detected byte order (`-1` = BE,
/// `1` = LE, `0` = unknown/no BOM), which `encodings/utf_16.py`'s
/// `IncrementalDecoder`/`StreamReader` use to pick which raw decoder to
/// keep using after the first chunk.
fn utf16_32_ex_decode_named(args: &[PyObjectRef], is32: bool) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("decode() requires at least 1 argument"));
    }
    let data = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Bytes(b) | PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("decode() argument must be bytes-like")),
        }
    };
    let bom_len = if is32 { 4 } else { 2 };
    let byteorder = if !is32 && data.starts_with(&[0xFF, 0xFE]) {
        -1i64
    } else if !is32 && data.starts_with(&[0xFE, 0xFF]) {
        1i64
    } else if is32 && data.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        -1i64
    } else if is32 && data.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
        1i64
    } else {
        0i64
    };
    let _ = bom_len;
    let norm = if is32 { "utf-32" } else { "utf-16" };
    let consumed = data.len();
    let s = crate::object::decode_utf16_utf32(&data, norm)
        .ok_or_else(|| PyError::value_error(format!("unknown encoding: {}", norm)))?;
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        py_str(&s),
        py_int(consumed as i64),
        py_int(byteorder),
    ])))
}

pub(crate) fn _codecs_utf_16_ex_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    utf16_32_ex_decode_named(args, false)
}

pub(crate) fn _codecs_utf_32_ex_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    utf16_32_ex_decode_named(args, true)
}

pub(crate) fn _codecs_decode_func(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "decode() requires at least 2 arguments",
        ));
    }
    let data = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Bytes(b) => b.clone(),
            _ => return Err(PyError::type_error("decode() argument must be bytes")),
        }
    };
    let encoding = args[1].str().to_lowercase().replace('-', "_");
    match encoding.as_str() {
        "utf_8" | "ascii" | "latin_1" | "utf8" => {
            let s = String::from_utf8(data)
                .map_err(|e| PyError::value_error(format!("decode error: {}", e)))?;
            let len = s.len();
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                py_str(&s),
                py_int(len as i64),
            ])))
        }
        _ => Err(PyError::value_error(format!(
            "unknown encoding: {}",
            encoding
        ))),
    }
}
