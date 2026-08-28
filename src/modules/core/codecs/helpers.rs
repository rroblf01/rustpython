use crate::object::*;
use std::collections::HashMap;
#[allow(unused_imports)]
use std::sync::atomic::AtomicI64;
#[allow(unused_imports)]
use num_traits::{Signed, ToPrimitive};
#[allow(unused_imports)]
use std::rc::Rc;

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
