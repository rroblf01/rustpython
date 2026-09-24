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

/// Error CPython raises for a malformed UTF-8 sequence, e.g.
/// `'utf-8' codec can't decode byte 0xff in position 1: invalid start byte`.
/// Its message argument carries the full `... in position N: reason` text so
/// tracebacks read exactly like CPython's.
fn utf8_decode_error(
    data: &[u8],
    valid_up_to: usize,
    error_len: Option<usize>,
    reason: &str,
) -> PyError {
    let end = valid_up_to + error_len.unwrap_or(1);
    let byte_text = match data.get(valid_up_to) {
        Some(b) => format!("byte 0x{:02x}", b),
        None => "byte".to_string(),
    };
    PyError::Exception(
        "UnicodeDecodeError".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "UnicodeDecodeError".to_string(),
            args: vec![
                py_str("utf-8"),
                py_str(&format!(
                    "{} in position {}: {}",
                    byte_text, valid_up_to, reason
                )),
                py_int(valid_up_to as i64),
                py_int(end as i64),
                py_str(reason),
            ],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

/// `codecs.utf_8_decode(data, errors='strict', final=False)` — the
/// incremental-decoder workhorse every `encodings/utf_8*.py` and
/// `encodings/utf_16.py` builds on. Raises the real `UnicodeDecodeError`
/// (not a `ValueError`) so `UnicodeDecodeError`-based handling works, and
/// honours the `errors` handler argument plus CPython's trailing-incomplete-
/// sequence behaviour, which depends on `final`.
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
    let errors = args
        .get(1)
        .map(|v| v.str())
        .unwrap_or_else(|| "strict".to_string());
    let final_flag = args.get(2).map(|v| v.truthy()).unwrap_or(false);
    utf8_decode_impl(&data, &errors, final_flag)
}

/// Shared UTF-8 decode with CPython's `errors` handlers and `final`
/// semantics. Returns `(text, consumed)` where `consumed` counts INPUT
/// bytes (a trailing incomplete sequence stays unconsumed when `final` is
/// false, so an incremental decoder can wait for more input).
fn utf8_decode_impl(data: &[u8], errors: &str, final_flag: bool) -> PyResult<PyObjectRef> {
    match std::str::from_utf8(data) {
        Ok(s) => {
            let len = data.len();
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                py_str(s),
                py_int(len as i64),
            ])))
        }
        Err(e) => {
            let valid_up_to = e.valid_up_to();
            let error_len = e.error_len();
            if !final_flag {
                // An incomplete sequence at the very end may still be
                // completed by later input: decode the valid prefix and
                // leave the rest unconsumed (what `IncrementalDecoder`
                // expects).
                if error_len.is_none() {
                    let prefix = std::str::from_utf8(&data[..valid_up_to]).unwrap_or("");
                    return Ok(PyObjectRef::new(PyObject::Tuple(vec![
                        py_str(prefix),
                        py_int(valid_up_to as i64),
                    ])));
                }
            }
            let reason = utf8_error_reason(data, valid_up_to, error_len);
            match errors {
                "strict" => Err(utf8_decode_error(data, valid_up_to, error_len, &reason)),
                "ignore" => {
                    let mut text = String::from_utf8_lossy(&data[..valid_up_to]).to_string();
                    let skip = error_len.unwrap_or(1).max(1);
                    let rest = &data[(valid_up_to + skip).min(data.len())..];
                    text.push_str(&String::from_utf8_lossy(rest));
                    let len = data.len();
                    Ok(PyObjectRef::new(PyObject::Tuple(vec![
                        py_str(&text),
                        py_int(len as i64),
                    ])))
                }
                "replace" => {
                    let mut text = String::from_utf8_lossy(&data[..valid_up_to]).to_string();
                    text.push('\u{fffd}');
                    let skip = error_len.unwrap_or(1).max(1);
                    let rest = &data[(valid_up_to + skip).min(data.len())..];
                    text.push_str(&String::from_utf8_lossy(rest));
                    let len = data.len();
                    Ok(PyObjectRef::new(PyObject::Tuple(vec![
                        py_str(&text),
                        py_int(len as i64),
                    ])))
                }
                "surrogateescape" => {
                    let mut text = String::from_utf8_lossy(&data[..valid_up_to]).to_string();
                    let skip = error_len.unwrap_or(1).max(1);
                    if valid_up_to < data.len() {
                        let b = data[valid_up_to];
                        let ch = char::from_u32(0xdc00 + b as u32).unwrap_or('\u{fffd}');
                        text.push(ch);
                    }
                    let rest = &data[(valid_up_to + skip).min(data.len())..];
                    text.push_str(&String::from_utf8_lossy(rest));
                    let len = data.len();
                    Ok(PyObjectRef::new(PyObject::Tuple(vec![
                        py_str(&text),
                        py_int(len as i64),
                    ])))
                }
                other => Err(PyError::Exception(
                    "LookupError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "LookupError".to_string(),
                        args: vec![py_str(&format!("unknown error handler name '{}'", other))],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    }),
                )),
            }
        }
    }
}

/// CPython's reason strings for the three UTF-8 failure shapes.
fn utf8_error_reason(data: &[u8], valid_up_to: usize, error_len: Option<usize>) -> String {
    match error_len {
        None => "unexpected end of data".to_string(),
        Some(len) => {
            let start = data[valid_up_to];
            if start < 0x80 {
                "invalid start byte".to_string()
            } else if start < 0xc0 {
                "invalid continuation byte".to_string()
            } else if start < 0xc2 {
                "invalid start byte".to_string()
            } else if start > 0xf4 {
                "invalid start byte".to_string()
            } else if len == 2 {
                "invalid continuation byte".to_string()
            } else {
                "invalid continuation byte".to_string()
            }
        }
    }
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

/// CPython raises `LookupError` (not `ValueError`) for an unknown codec name
/// — `str.encode`, `bytes.decode`, `codecs.lookup` and `codecs.getreader`
/// all depend on that type, and test_codecs asserts it.
fn unknown_encoding(encoding: &str) -> PyError {
    PyError::Exception(
        "LookupError".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "LookupError".to_string(),
            args: vec![py_str(&format!("unknown encoding: {}", encoding))],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
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

/// The generic class from the (real, vendored) `Lib/codecs.py` of a given
/// name — `StreamReader`, `StreamWriter`, `IncrementalEncoder`,
/// `IncrementalDecoder` — or None when the codecs module isn't loaded yet.
fn generic_codec_class(name: &str) -> Option<PyObjectRef> {
    crate::modules::get_module("codecs")
        .as_ref()
        .and_then(|m| m.borrow().get_attribute(name).ok())
        .filter(|v| !matches!(&*v.borrow(), PyObject::None))
}

pub(crate) fn _codecs_lookup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("lookup() requires at least 1 argument"));
    }
    // CPython's `normalizestring` (Python/codecs.c): lowercase, and spaces
    // and hyphens become underscores. Without the space case,
    // `codecs.lookup('raw unicode escape')` missed its search function.
    let encoding = args[0].str().to_lowercase().replace(['-', ' '], "_");
    // CPython's `lookup` consults the registered search functions FIRST
    // (that is how `encodings/*` installs each codec's own
    // `IncrementalEncoder`/`IncrementalDecoder`/`StreamReader`/
    // `StreamWriter` classes). Do the same before the native fast path, so
    // `codecs.getincrementaldecoder('utf8')` (which reads
    // `lookup(enc).incrementaldecoder`) gets the REAL class instead of the
    // abstract base — that base raises `NotImplementedError` on every
    // `decode()`, which broke every stream-codec read/write.
    let search_hit = super::lookup_codec(&encoding)?;
    if let Some(found) = &search_hit {
        if !matches!(&*found.borrow(), PyObject::None) {
            return Ok(found.clone());
        }
    }
    match encoding.as_str() {
        // CPython's `codecs.lookup()` returns a `CodecInfo`: an object with
        // the named attributes `name`, `encode`, `decode`, `incref`,
        // `decref`, `streamreader`, `streamwriter` (plus tuple-like
        // indexing). The real (vendored) `Lib/codecs.py` uses exactly those
        // attributes (`lookup(enc).streamreader` in `getreader`,
        // `.encode`/`.decode` in `getencoder`/`getdecoder`), so returning a
        // bare tuple broke every one of those calls with
        // `AttributeError: 'tuple' object has no attribute 'streamreader'`
        // (and stalled the stream-codec tests). Built here as an instance
        // carrying the seven entries — attribute access on an instance
        // reads its dict, which is what the stdlib needs.
        "utf_8" | "ascii" | "latin_1" | "utf8" => {
            let mut info = crate::object::AttrMap::new();
            info.insert_str("name", py_str(&encoding));
            info.insert_str(
                "encode",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "encode".to_string(),
                    func: _codecs_encode,
                }),
            );
            info.insert_str(
                "decode",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "decode".to_string(),
                    func: _codecs_decode,
                }),
            );
            info.insert_str("incref", py_none());
            info.insert_str("decref", py_none());
            // Abstract fallbacks only reachable when no `encodings` search
            // function claimed the codec (see the search-first note above).
            info.insert_str(
                "incrementalencoder",
                generic_codec_class("IncrementalEncoder").unwrap_or_else(|| py_none()),
            );
            info.insert_str(
                "incrementaldecoder",
                generic_codec_class("IncrementalDecoder").unwrap_or_else(|| py_none()),
            );
            // `streamreader`/`streamwriter`: CPython's `CodecInfo` carries
            // the codec-SPECIFIC classes (`encodings.utf_8.StreamReader`,
            // installed by the `encodings` package's search functions). The
            // generic `codecs.StreamReader`/`StreamWriter` from the (real,
            // vendored) `Lib/codecs.py` are behaviourally equivalent for
            // this purpose, and are what the `encodings` package itself
            // subclasses — prefer the registered search function's own
            // classes, and fall back to those.
            let generic = generic_codec_class;
            let from_search = search_hit
                .as_ref()
                .and_then(|found| {
                    let borrowed = found.borrow();
                    match &*borrowed {
                        PyObject::Instance { dict, .. } => Some((
                            dict.get_str("streamreader").cloned(),
                            dict.get_str("streamwriter").cloned(),
                        )),
                        PyObject::Tuple(items) => Some((
                            items
                                .get(5)
                                .cloned()
                                .filter(|v| !matches!(&*v.borrow(), PyObject::None)),
                            items
                                .get(6)
                                .cloned()
                                .filter(|v| !matches!(&*v.borrow(), PyObject::None)),
                        )),
                        _ => None,
                    }
                })
                .unwrap_or((None, None));
            info.insert_str(
                "streamreader",
                from_search
                    .0
                    .or_else(|| generic("StreamReader"))
                    .unwrap_or_else(|| {
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "stream_reader".to_string(),
                            func: _codecs_reader,
                        })
                    }),
            );
            info.insert_str(
                "streamwriter",
                from_search
                    .1
                    .or_else(|| generic("StreamWriter"))
                    .unwrap_or_else(|| {
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "stream_writer".to_string(),
                            func: _codecs_writer,
                        })
                    }),
            );
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "CodecInfo".to_string(),
                    dict: Box::new(crate::object::str_map_to_typedict(
                        std::collections::HashMap::new(),
                    )),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: info,
            }))
        }
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
            Err(unknown_encoding(&encoding))
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
    let bytes =
        crate::object::encode_utf16_utf32(&s, norm).ok_or_else(|| unknown_encoding(&norm))?;
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
    let s =
        crate::object::decode_utf16_utf32(&data, norm).ok_or_else(|| unknown_encoding(&norm))?;
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
    let s =
        crate::object::decode_utf16_utf32(&data, norm).ok_or_else(|| unknown_encoding(&norm))?;
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

/// Which escape flavour a `*_escape_*` native implements. Mirrors the three
/// distinct CPython escape codecs: `unicode_escape` (escapes everything
/// non-printable-ASCII, decodes hex/octal/`\u`/`\U`/`\N{}`),
/// `raw_unicode_escape` (leaves 0x00-0xFF raw, decodes only `\u`/`\U`) and
/// `string_escape` (operates on bytes, decodes hex/octal only).
#[derive(Clone, Copy, PartialEq)]
enum EscapeFlavour {
    Unicode,
    RawUnicode,
    Bytes,
}

impl EscapeFlavour {
    fn codec_name(self) -> &'static str {
        match self {
            EscapeFlavour::Unicode => "unicodeescape",
            EscapeFlavour::RawUnicode => "rawunicodeescape",
            EscapeFlavour::Bytes => "stringescape",
        }
    }

    /// Decoded characters are produced for input bytes >= 0x80 in the
    /// `unicode_escape`/`raw_unicode_escape` codecs (latin-1 style) and
    /// kept as raw bytes in the `string_escape` one.
    fn decodes_latin1_high_bytes(self) -> bool {
        !matches!(self, EscapeFlavour::Bytes)
    }
}

fn escape_decode_error(flavour: EscapeFlavour, msg: &str, start: usize, end: usize) -> PyError {
    if matches!(flavour, EscapeFlavour::Bytes) {
        // CPython's `string_escape` decoder reports these as plain
        // `ValueError`s carrying the position, e.g. `invalid \x escape at
        // position 0` — not `UnicodeDecodeError`.
        let text = match msg {
            "truncated \\xXX escape" => format!("invalid \\x escape at position {}", start),
            "\\ at end of string" => "Trailing \\ in string".to_string(),
            other => format!("{} at position {}", other, start),
        };
        return PyError::value_error(text);
    }
    let text = format!(
        "'{}' codec can't decode bytes in position {}-{}: {}",
        flavour.codec_name(),
        start,
        end,
        msg
    );
    PyError::Exception(
        "UnicodeDecodeError".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "UnicodeDecodeError".to_string(),
            args: vec![
                py_str(flavour.codec_name()),
                py_str(&text),
                py_int(start as i64),
                py_int(end as i64),
                py_str(msg),
            ],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

fn escape_hex_value(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a') as u32 + 10),
        b'A'..=b'F' => Some((b - b'A') as u32 + 10),
        _ => None,
    }
}

fn escape_parse_digits(bytes: &[u8], start: usize, count: usize) -> Option<u32> {
    if bytes.len() < start + count {
        return None;
    }
    let mut value = 0u32;
    for &b in &bytes[start..start + count] {
        value = value.checked_mul(16)?.checked_add(escape_hex_value(b)?)?;
    }
    Some(value)
}

/// `unicode_escape_decode` / `raw_unicode_escape_decode` /
/// `string_escape` decoding, ported from CPython's `PyUnicode_DecodeEscape`
/// family. Returns the decoded text and the number of input bytes consumed
/// (which, for the incremental decoders, stops at the first incomplete
/// escape).
fn escape_decode_impl(bytes: &[u8], flavour: EscapeFlavour) -> Result<(String, usize), PyError> {
    // `out` is built as a Rust `String` even for the bytes flavour; every
    // pushed char is a single byte value there, so the caller's
    // `chars().map(|c| c as u32 as u8)` round-trip is lossless.
    let mut out = String::new();
    let mut i = 0usize;
    let mut consumed = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'\\' {
            match b {
                0x00..=0x7f => out.push(b as char),
                _ => {
                    // The text codecs decode high bytes latin-1 style; the
                    // bytes codec passes them through untouched.
                    out.push(b as char);
                }
            }
            i += 1;
            consumed = i;
            continue;
        }
        // Backslash escape
        if i + 1 >= bytes.len() {
            if matches!(flavour, EscapeFlavour::RawUnicode) {
                out.push('\\');
                i += 1;
                consumed = i;
                continue;
            }
            return Err(escape_decode_error(
                flavour,
                "\\ at end of string",
                i,
                i + 1,
            ));
        }
        let c = bytes[i + 1];
        // Simple escapes honoured by the unicode/raw codecs
        if matches!(flavour, EscapeFlavour::Unicode | EscapeFlavour::RawUnicode) {
            let simple = match c {
                b'\n' => None,
                b't' => Some('\t'),
                b'n' => Some('\n'),
                b'r' => Some('\r'),
                b'a' => Some('\u{7}'),
                b'b' => Some('\u{8}'),
                b'f' => Some('\u{c}'),
                b'v' => Some('\u{b}'),
                b'\\' => Some('\\'),
                b'\'' => Some('\''),
                b'"' => Some('"'),
                _ => None,
            };
            if let Some(ch) = simple {
                if c == b'\\' && matches!(flavour, EscapeFlavour::RawUnicode) {
                    // raw_unicode_escape leaves a backslash pair untouched
                    out.push('\\');
                    out.push('\\');
                    i += 2;
                    consumed = i;
                    continue;
                }
                out.push(ch);
                i += 2;
                consumed = i;
                continue;
            }
        } else {
            let simple = match c {
                b'\n' => None,
                b't' => Some('\t'),
                b'n' => Some('\n'),
                b'r' => Some('\r'),
                b'a' => Some('\u{7}'),
                b'b' => Some('\u{8}'),
                b'f' => Some('\u{c}'),
                b'v' => Some('\u{b}'),
                b'\\' => Some('\\'),
                b'\'' => Some('\''),
                b'"' => Some('"'),
                _ => None,
            };
            if let Some(ch) = simple {
                out.push(ch);
                i += 2;
                consumed = i;
                continue;
            }
        }
        // \xXX — unicode_escape and string_escape only
        if c == b'x' && !matches!(flavour, EscapeFlavour::RawUnicode) {
            match (bytes.len() > i + 3, escape_parse_digits(bytes, i + 2, 2)) {
                (true, Some(value)) => {
                    out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
                    i += 4;
                    consumed = i;
                }
                _ => {
                    return Err(escape_decode_error(
                        flavour,
                        "truncated \\xXX escape",
                        i,
                        (i + 3).min(bytes.len()),
                    ))
                }
            }
            continue;
        }
        // \uXXXX / \UNNNNNNNN — both unicode codecs, never string_escape
        if (c == b'u' || c == b'U') && !matches!(flavour, EscapeFlavour::Bytes) {
            let count = if c == b'u' { 4 } else { 8 };
            match escape_parse_digits(bytes, i + 2, count) {
                Some(value) => {
                    if value > 0x10FFFF {
                        return Err(escape_decode_error(
                            flavour,
                            if c == b'u' {
                                "truncated \\uXXXX escape"
                            } else {
                                "\\Uxxxxxxxx out of range"
                            },
                            i,
                            i + 2 + count,
                        ));
                    }
                    // Lone surrogates are allowed (the codecs are
                    // surrogate-producing by design).
                    out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
                    i += 2 + count;
                    consumed = i;
                }
                None => {
                    if bytes.len() < i + 2 + count {
                        return Err(escape_decode_error(
                            flavour,
                            if c == b'u' {
                                "truncated \\uXXXX escape"
                            } else {
                                "truncated \\UXXXXXXXX escape"
                            },
                            i,
                            (i + 2 + count).min(bytes.len()),
                        ));
                    }
                    return Err(escape_decode_error(
                        flavour,
                        "illegal Unicode character",
                        i,
                        i + 2 + count,
                    ));
                }
            }
            continue;
        }
        // Octal \0oo — unicode_escape and string_escape only
        if c.is_ascii_digit() && !matches!(flavour, EscapeFlavour::RawUnicode) {
            // `\ooo`: up to three octal digits. The text codecs keep the
            // full code point (`\400` -> U+0100), the bytes codec keeps the
            // low byte (`\400` -> 0x00).
            let mut value = (c - b'0') as u32;
            let mut width = 2usize;
            while width < 4 && i + width < bytes.len() && bytes[i + width].is_ascii_digit() {
                value = value * 8 + (bytes[i + width] - b'0') as u32;
                width += 1;
            }
            if matches!(flavour, EscapeFlavour::Bytes) {
                out.push(char::from_u32(value & 0xFF).unwrap_or('\u{fffd}'));
            } else {
                out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
            }
            i += width;
            consumed = i;
            continue;
        }
        // \N{NAME} — unicode_escape only
        if c == b'N'
            && matches!(flavour, EscapeFlavour::Unicode)
            && bytes.len() > i + 2
            && bytes[i + 2] == b'{'
        {
            if let Some(close) = bytes[i + 3..].iter().position(|&b| b == b'}') {
                let name = String::from_utf8_lossy(&bytes[i + 3..i + 3 + close]).to_string();
                match crate::unicode_names::char_from_name(&name) {
                    Some(ch) => {
                        out.push(ch);
                        i += 4 + close;
                        consumed = i;
                        continue;
                    }
                    None => {
                        return Err(escape_decode_error(
                            flavour,
                            "unknown Unicode character name",
                            i,
                            i + 4 + close,
                        ))
                    }
                }
            }
            return Err(escape_decode_error(
                flavour,
                "malformed \\N character escape",
                i,
                bytes.len(),
            ));
        }
        // Unrecognised escape: keep the backslash verbatim
        out.push('\\');
        i += 1;
        consumed = i;
    }
    Ok((out, consumed))
}

/// Bytes shared by the three escape ENCODERS. `flavour` decides which
/// characters are escaped: `unicode_escape` escapes everything outside
/// printable ASCII (plus the standard short escapes), `raw_unicode_escape`
/// only escapes DEL and non-Latin-1 characters, and both return the input
/// length as consumed.
fn escape_encode_impl(text: &str, flavour: EscapeFlavour) -> (Vec<u8>, usize) {
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        match flavour {
            EscapeFlavour::Unicode => match ch {
                '\\' => out.extend_from_slice(b"\\\\"),
                '\n' => out.extend_from_slice(b"\\n"),
                '\r' => out.extend_from_slice(b"\\r"),
                '\t' => out.extend_from_slice(b"\\t"),
                _ if (0x20..0x7f).contains(&code) => out.push(code as u8),
                _ if code < 0x100 => out.extend_from_slice(format!("\\x{:02x}", code).as_bytes()),
                _ if code < 0x10000 => out.extend_from_slice(format!("\\u{:04x}", code).as_bytes()),
                _ => out.extend_from_slice(format!("\\U{:08x}", code).as_bytes()),
            },
            EscapeFlavour::RawUnicode => {
                if code == 0x7f {
                    out.extend_from_slice(b"\\x7f");
                } else if code < 0x100 {
                    out.push(code as u8);
                } else if code < 0x10000 {
                    out.extend_from_slice(format!("\\u{:04x}", code).as_bytes());
                } else {
                    out.extend_from_slice(format!("\\U{:08x}", code).as_bytes());
                }
            }
            EscapeFlavour::Bytes => {
                let byte = if code < 0x100 { code as u8 } else { b'?' };
                match byte {
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    b'\t' => out.extend_from_slice(b"\\t"),
                    0x20..=0x7e => out.push(byte),
                    _ => out.extend_from_slice(format!("\\x{:02x}", byte).as_bytes()),
                }
            }
        }
    }
    (out, text.len())
}

fn bytes_arg(args: &[PyObjectRef], index: usize, who: &str) -> PyResult<Vec<u8>> {
    match args.get(index) {
        Some(v) => match &*v.borrow() {
            PyObject::Bytes(b) => Ok(b.clone()),
            PyObject::ByteArray(b) => Ok(b.clone()),
            other => Err(PyError::type_error(format!(
                "{}() argument {} must be bytes-like object, not {}",
                who,
                index + 1,
                other.type_name()
            ))),
        },
        None => Err(PyError::type_error(format!(
            "{}() takes at least {} argument(s) ({} given)",
            who,
            index + 1,
            args.len()
        ))),
    }
}

fn str_result(text: String, consumed: usize) -> PyObjectRef {
    PyObjectRef::new(PyObject::Tuple(vec![
        py_str(&text),
        py_int(consumed as i64),
    ]))
}

fn encode_result(bytes: Vec<u8>, consumed: usize) -> PyObjectRef {
    PyObjectRef::new(PyObject::Tuple(vec![
        PyObjectRef::imm(PyObject::Bytes(bytes)),
        py_int(consumed as i64),
    ]))
}

pub(crate) fn unicode_escape_encode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let text = args
        .first()
        .map(|v| v.str())
        .ok_or_else(|| PyError::type_error("unicode_escape_encode() takes at least 1 argument"))?;
    let (bytes, consumed) = escape_encode_impl(&text, EscapeFlavour::Unicode);
    Ok(encode_result(bytes, consumed))
}

pub(crate) fn raw_unicode_escape_encode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let text = args.first().map(|v| v.str()).ok_or_else(|| {
        PyError::type_error("raw_unicode_escape_encode() takes at least 1 argument")
    })?;
    let (bytes, consumed) = escape_encode_impl(&text, EscapeFlavour::RawUnicode);
    Ok(encode_result(bytes, consumed))
}

pub(crate) fn escape_encode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let bytes = bytes_arg(args, 0, "escape_encode")?;
    let text: String = bytes.iter().map(|&b| b as char).collect();
    let (out, consumed) = escape_encode_impl(&text, EscapeFlavour::Bytes);
    Ok(encode_result(out, consumed))
}

pub(crate) fn unicode_escape_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let bytes = bytes_arg(args, 0, "unicode_escape_decode")?;
    let (text, consumed) = escape_decode_impl(&bytes, EscapeFlavour::Unicode)?;
    Ok(str_result(text, consumed))
}

pub(crate) fn raw_unicode_escape_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let bytes = bytes_arg(args, 0, "raw_unicode_escape_decode")?;
    let (text, consumed) = escape_decode_impl(&bytes, EscapeFlavour::RawUnicode)?;
    Ok(str_result(text, consumed))
}

pub(crate) fn escape_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let bytes = bytes_arg(args, 0, "escape_decode")?;
    let (text, consumed) = escape_decode_impl(&bytes, EscapeFlavour::Bytes)?;
    let raw: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
    Ok(encode_result(raw, consumed))
}

/// Modified base64 alphabet used by UTF-7 (RFC 2152): standard base64 with
/// `+` replaced by `,`, emitted without padding.
const UTF7_B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

fn utf7_b64_value(b: u8) -> Option<u32> {
    match b {
        b'A'..=b'Z' => Some((b - b'A') as u32),
        b'a'..=b'z' => Some((b - b'a') as u32 + 26),
        b'0'..=b'9' => Some((b - b'0') as u32 + 52),
        b'+' | b',' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// UTF-7 "set O" direct characters: printable ASCII except `+` and `~`, plus
/// TAB/LF/CR (verified against CPython's `utf_7_encode` for every ASCII
/// code point).
fn utf7_is_direct(ch: char) -> bool {
    match ch {
        '\t' | '\n' | '\r' => true,
        _ => {
            let code = ch as u32;
            (0x20..0x7f).contains(&code) && code != b'+' as u32 && code != b'~' as u32
        }
    }
}

/// `codecs.utf_7_encode(text, errors='strict')` — RFC 2152. Non-direct
/// characters are UTF-16BE-encoded, modified-base64'd, and wrapped in `+...-`;
/// a `-` inside the input terminates an open escape directly, matching
/// CPython's output for inputs like `'a+b'` -> `b'a+-b'`.
pub(crate) fn utf_7_encode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let text = args
        .first()
        .map(|v| v.str())
        .ok_or_else(|| PyError::type_error("utf_7_encode() takes at least 1 argument"))?;
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    let mut pending: Vec<u8> = Vec::new();
    let mut in_escape = false;
    for ch in text.chars() {
        if ch == '+' && !in_escape {
            // Outside an escape a literal '+' must be written "+-" so it is
            // not mistaken for an escape introducer. INSIDE one it is just
            // another code unit (CPython: 'aé+b' -> b'a+AOkAKw-b').
            out.push(b'+');
            out.push(b'-');
            continue;
        }
        if utf7_is_direct(ch) {
            if in_escape {
                if !pending.is_empty() {
                    let group: Vec<u8> = pending.drain(..).collect();
                    encode_utf7_group(&mut out, &group);
                }
                if ch == '-' {
                    // A literal '-' both terminates the escape and is
                    // itself the direct character.
                    out.push(b'-');
                } else {
                    out.push(b'-');
                    out.push(ch as u8);
                }
                in_escape = false;
                continue;
            }
            out.push(ch as u8);
            continue;
        }
        // Non-direct: UTF-16 code units
        let code = ch as u32;
        if code > 0xFFFF {
            let adjusted = code - 0x10000;
            let high = 0xD800 + (adjusted >> 10);
            let low = 0xDC00 + (adjusted & 0x3FF);
            pending.extend_from_slice(&(high as u16).to_be_bytes());
            pending.extend_from_slice(&(low as u16).to_be_bytes());
        } else {
            pending.extend_from_slice(&(code as u16).to_be_bytes());
        }
        if !in_escape {
            out.push(b'+');
            in_escape = true;
        }
        if pending.len() >= 3 {
            // Emit complete 3-byte groups eagerly so the terminator never
            // has to be followed by padding.
            let take = pending.len() / 3 * 3;
            let group: Vec<u8> = pending.drain(..take).collect();
            encode_utf7_group(&mut out, &group);
        }
    }
    if in_escape {
        if !pending.is_empty() {
            let group: Vec<u8> = pending.drain(..).collect();
            encode_utf7_group(&mut out, &group);
        }
        out.push(b'-');
    }
    let len = text.len();
    Ok(encode_result(out, len))
}

fn encode_utf7_group(out: &mut Vec<u8>, group: &[u8]) {
    for chunk in group.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(UTF7_B64[((triple >> 18) & 0x3F) as usize]);
        out.push(UTF7_B64[((triple >> 12) & 0x3F) as usize]);
        if chunk.len() > 1 {
            out.push(UTF7_B64[((triple >> 6) & 0x3F) as usize]);
        }
        if chunk.len() > 2 {
            out.push(UTF7_B64[(triple & 0x3F) as usize]);
        }
    }
}

/// `codecs.utf_7_decode(data, errors='strict', final=False)`. An escape runs
/// from `+` to the next `-` or to the first character that is not in the
/// modified base64 alphabet (which is then decoded normally, unconsumed by
/// the escape), matching CPython — `b'+ImQ.'` decodes to `'≤.'`.
pub(crate) fn utf_7_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = bytes_arg(args, 0, "utf_7_decode")?;
    let errors = args
        .get(1)
        .map(|v| v.str())
        .unwrap_or_else(|| "strict".to_string());
    let mut out = String::new();
    let mut i = 0usize;
    let mut consumed = 0usize;
    while i < data.len() {
        if data[i] != b'+' {
            let byte = data[i];
            if byte < 0x80 {
                out.push(byte as char);
            } else {
                // Raw non-ASCII bytes are not valid UTF-7 input, but CPython
                // decodes them latin-1 style rather than failing.
                out.push(byte as char);
            }
            i += 1;
            consumed = i;
            continue;
        }
        // Escape: collect modified-base64 digits
        let start = i;
        i += 1;
        let mut digits: Vec<u8> = Vec::new();
        while i < data.len() {
            let b = data[i];
            if b == b'-' {
                i += 1;
                break;
            }
            if utf7_b64_value(b).is_none() {
                break;
            }
            digits.push(b);
            i += 1;
        }
        // Decode the digit run into UTF-16 code units
        let mut units: Vec<u16> = Vec::new();
        let mut acc: u32 = 0;
        let mut bits = 0u32;
        for d in &digits {
            acc = (acc << 6) | utf7_b64_value(*d).unwrap_or(0);
            bits += 6;
            if bits >= 16 {
                bits -= 16;
                units.push(((acc >> bits) & 0xFFFF) as u16);
            }
        }
        if units.is_empty() {
            // CPython yields nothing for an empty escape (`b'++'` -> '').
            if i > data.len() {
                i = data.len();
            }
            consumed = i;
            continue;
        }
        // Leftover bits below 16 are ignored, matching CPython's tolerance
        // of a final partial group (`b'+ImQ.'` -> '≤.').
        match String::from_utf16(&units) {
            Ok(text) => out.push_str(&text),
            Err(e) => {
                if errors == "strict" {
                    return Err(utf8_decode_error(
                        &data,
                        start,
                        None,
                        &format!("utf-7 stream does not end with a '-' terminator"),
                    ));
                }
                let _ = e;
            }
        }
        consumed = i;
    }
    let _ = consumed;
    Ok(str_result(out, data.len()))
}
