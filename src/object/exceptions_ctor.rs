// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the builtin
// exception constructor functions (one per exception type, generated via
// the `make_exception_func!` macro).
use super::*;

mod utils;
pub use utils::*;
mod deepcopy;
pub use deepcopy::*;

// ---- Exception constructor functions ----

macro_rules! make_exception_func {
    ($name:ident, $typ:expr) => {
        pub fn $name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            // Builtin exception constructors reject keyword arguments
            // (they arrive as a trailing kwargs dict from call_function):
            // `BaseException(a=1)` must raise TypeError, per CPython's
            // test_exceptions::testKeywordArgs.
            if let Some(last) = args.last() {
                if matches!(&*last.borrow(), PyObject::Dict(_)) {
                    return Err(PyError::type_error(format!(
                        "{}() takes no keyword arguments",
                        $typ
                    )));
                }
            }
            Ok(PyObjectRef::new(PyObject::Exception {
                typ: $typ.to_string(),
                args: args.to_vec(),
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }))
        }
    };
}

make_exception_func!(builtin_make_exception_baseexception, "BaseException");
make_exception_func!(builtin_make_exception_exception, "Exception");
make_exception_func!(builtin_make_exception_typeerror, "TypeError");
make_exception_func!(builtin_make_exception_valueerror, "ValueError");
make_exception_func!(
    builtin_make_exception_zerodivisionerror,
    "ZeroDivisionError"
);
make_exception_func!(builtin_make_exception_nameerror, "NameError");
make_exception_func!(
    builtin_make_exception_unboundlocalerror,
    "UnboundLocalError"
);
pub fn builtin_make_exception_attributeerror(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // CPython: `AttributeError('x', name='carry', obj=sentinel)` keeps the
    // positional args in `.args` and stores `name`/`obj` (the attribute name
    // and owning object) as instance attrs. Here keyword args arrive as a
    // trailing dict in `args`; extract and re-store them, then keep the
    // positional message args clean (like real `.args`).
    let (positional, kwargs) = match args.split_last() {
        Some((last, rest)) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
            let mut pos: Vec<PyObjectRef> = Vec::new();
            for a in rest {
                pos.push(a.clone());
            }
            let d = last.borrow();
            if let PyObject::Dict(d) = &*d {
                (pos, Some(d.clone()))
            } else {
                (pos, None)
            }
        }
        _ => (args.to_vec(), None),
    };
    let mut extra = None;
    if let Some(kw) = kwargs {
        let mut m = std::collections::HashMap::new();
        let k_name = py_str("name");
        let k_obj = py_str("obj");
        if let Ok(Some(name)) = kw.get(&k_name) {
            m.insert("name".to_string(), name.clone());
        }
        if let Ok(Some(obj)) = kw.get(&k_obj) {
            m.insert("obj".to_string(), obj.clone());
        }
        if !m.is_empty() {
            extra = Some(m);
        }
    }
    Ok(PyObjectRef::new(PyObject::Exception {
        typ: "AttributeError".to_string(),
        args: positional,
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra,
    }))
}
make_exception_func!(builtin_make_exception_indexerror, "IndexError");
make_exception_func!(builtin_make_exception_keyerror, "KeyError");
make_exception_func!(builtin_make_exception_runtimeerror, "RuntimeError");
make_exception_func!(builtin_make_exception_stopiteration, "StopIteration");
make_exception_func!(builtin_make_exception_assertionerror, "AssertionError");
pub fn builtin_make_exception_oserror(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // CPython: `OSError(errno, strerror, filename, ...)` — the FIRST TWO
    // positional args are the exception's `.args`; a 3rd is `.filename`; a
    // 5th `.filename2` (test_exceptions' testAttributes table). Also expose
    // `.errno`/`.strerror`. Keyword args are rejected like other builtin
    // exceptions.
    if let Some(last) = args.last() {
        if matches!(&*last.borrow(), PyObject::Dict(_)) {
            return Err(PyError::type_error("OSError() takes no keyword arguments"));
        }
    }
    let mut extra = std::collections::HashMap::new();
    let mut clean_args: Vec<PyObjectRef> = Vec::new();
    if let Some(a0) = args.first() {
        clean_args.push(a0.clone());
    }
    if let Some(a1) = args.get(1) {
        // errno/strerror derive only from the 2-arg form and up
        // (`OSError('foo')` keeps errno/strerror None, per CPython).
        if let Some(a0) = args.first() {
            extra.insert("errno".to_string(), a0.clone());
        }
        extra.insert("strerror".to_string(), a1.clone());
        clean_args.push(a1.clone());
    }
    if let Some(a2) = args.get(2) {
        extra.insert("filename".to_string(), a2.clone());
    }
    if let Some(a4) = args.get(4) {
        extra.insert("filename2".to_string(), a4.clone());
    }
    Ok(PyObjectRef::new(PyObject::Exception {
        typ: "OSError".to_string(),
        args: clean_args,
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra: Some(extra),
    }))
}
pub fn builtin_make_exception_importerror(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // CPython: `ImportError('test', name=..., path=...)` stores the message
    // positionally and name/path as attrs; any OTHER keyword is rejected
    // (test_exceptions::ImportErrorTests::test_attributes). Kwargs arrive as
    // a trailing dict from call_function.
    let (positional, kwargs) = match args.split_last() {
        Some((last, rest)) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
            (rest.to_vec(), Some(last.clone()))
        }
        _ => (args.to_vec(), None),
    };
    let mut extra = None;
    if let Some(kw) = kwargs {
        let mut m = std::collections::HashMap::new();
        let mut unexpected = None;
        let d = kw.borrow();
        if let PyObject::Dict(d) = &*d {
            let k_name = py_str("name");
            let k_path = py_str("path");
            for (k, v) in d.iter() {
                let key = match &*k.borrow() {
                    PyObject::Str(s) => s.to_string(),
                    _ => continue,
                };
                if k.is(&k_name) {
                    m.insert("name".to_string(), v.clone());
                } else if k.is(&k_path) {
                    m.insert("path".to_string(), v.clone());
                } else {
                    unexpected = Some(key);
                    break;
                }
            }
        }
        if let Some(key) = unexpected {
            return Err(PyError::type_error(format!(
                "ImportError() got an unexpected keyword argument '{}'",
                key
            )));
        }
        if !m.is_empty() {
            extra = Some(m);
        }
    }
    Ok(PyObjectRef::new(PyObject::Exception {
        typ: "ImportError".to_string(),
        args: positional,
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra,
    }))
}
make_exception_func!(builtin_make_exception_pickleerror, "PickleError");
make_exception_func!(builtin_make_exception_picklingerror, "PicklingError");
make_exception_func!(builtin_make_exception_unpicklingerror, "UnpicklingError");
// Additional exception types for full CPython hierarchy
make_exception_func!(builtin_make_exception_lookuperror, "LookupError");
make_exception_func!(builtin_make_exception_arithmeticerror, "ArithmeticError");
make_exception_func!(
    builtin_make_exception_floatingpointerror,
    "FloatingPointError"
);
make_exception_func!(builtin_make_exception_overflowerror, "OverflowError");
make_exception_func!(builtin_make_exception_environmenterror, "EnvironmentError");
make_exception_func!(builtin_make_exception_ioerror, "IOError");
make_exception_func!(
    builtin_make_exception_filenotfounderror,
    "FileNotFoundError"
);
make_exception_func!(builtin_make_exception_permissionerror, "PermissionError");
make_exception_func!(
    builtin_make_exception_notimplementederror,
    "NotImplementedError"
);
make_exception_func!(builtin_make_exception_recursionerror, "RecursionError");
make_exception_func!(
    builtin_make_exception_keyboardinterrupt,
    "KeyboardInterrupt"
);
make_exception_func!(builtin_make_exception_generatorexit, "GeneratorExit");
make_exception_func!(builtin_make_exception_systemexit, "SystemExit");
make_exception_func!(
    builtin_make_exception_modulenotfounderror,
    "ModuleNotFoundError"
);
make_exception_func!(
    builtin_make_exception_stopasynciteration,
    "StopAsyncIteration"
);
make_exception_func!(builtin_make_exception_eoferror, "EOFError");
pub fn builtin_make_exception_syntaxerror(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // CPython: `SyntaxError(msg, (filename, lineno, offset, text,
    // end_lineno, end_offset))` — the location tuple drives `.msg`/
    // `.filename`/`.lineno`/`.offset`/`.text`/`.end_lineno`/`.end_offset`;
    // the flat `SyntaxError(msg, filename, lineno, offset, text, ...)` form
    // stores those as positional args WITHOUT deriving the location
    // attributes (per test_exceptions' testAttributes table).
    let mut extra = std::collections::HashMap::new();
    let mut clean_args: Vec<PyObjectRef> = Vec::new();
    for a in args {
        clean_args.push(a.clone());
    }
    if let Some((second, rest)) = args.split_first() {
        if rest.len() == 1 {
            if let PyObject::Tuple(t) = &*rest[0].borrow() {
                // `SyntaxError(msg, (filename, lineno, offset, text,
                // end_lineno, end_offset))`
                let mut it = t.iter();
                let filename = it.next();
                let lineno = it.next();
                let offset = it.next();
                let text = it.next();
                let end_lineno = it.next();
                let end_offset = it.next();
                let set = |extra: &mut std::collections::HashMap<String, PyObjectRef>,
                           key: &str,
                           v: Option<&PyObjectRef>| {
                    if let Some(v) = v {
                        extra.insert(key.to_string(), v.clone());
                    }
                };
                set(&mut extra, "filename", filename);
                set(&mut extra, "lineno", lineno);
                set(&mut extra, "offset", offset);
                set(&mut extra, "text", text);
                set(&mut extra, "end_lineno", end_lineno);
                set(&mut extra, "end_offset", end_offset);
            }
        }
    }
    Ok(PyObjectRef::new(PyObject::Exception {
        typ: "SyntaxError".to_string(),
        args: clean_args,
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra: if extra.is_empty() { None } else { Some(extra) },
    }))
}
make_exception_func!(builtin_make_exception_cycleerror, "CycleError");
make_exception_func!(
    builtin_make_exception_incompleteinputerror,
    "_IncompleteInputError"
);
make_exception_func!(builtin_make_exception_decimalexception, "DecimalException");
make_exception_func!(builtin_make_exception_invalidoperation, "InvalidOperation");
make_exception_func!(
    builtin_make_exception_decimaldivisionbyzero,
    "DivisionByZero"
);
make_exception_func!(builtin_make_exception_inexact, "Inexact");
make_exception_func!(builtin_make_exception_rounded, "Rounded");
make_exception_func!(builtin_make_exception_clamped, "Clamped");
make_exception_func!(builtin_make_exception_decimaloverflow, "Overflow");
make_exception_func!(builtin_make_exception_decimalunderflow, "Underflow");
make_exception_func!(builtin_make_exception_floatoperation, "FloatOperation");
make_exception_func!(builtin_make_exception_connectionerror, "ConnectionError");
make_exception_func!(builtin_make_exception_brokenpipeerror, "BrokenPipeError");
make_exception_func!(
    builtin_make_exception_connectionrefusederror,
    "ConnectionRefusedError"
);
make_exception_func!(builtin_make_exception_blockingioerror, "BlockingIOError");
make_exception_func!(
    builtin_make_exception_childprocesserror,
    "ChildProcessError"
);
make_exception_func!(builtin_make_exception_interruptederror, "InterruptedError");
make_exception_func!(builtin_make_exception_timeouterror, "TimeoutError");
make_exception_func!(builtin_make_exception_unicodeerror, "UnicodeError");
make_exception_func!(
    builtin_make_exception_unicodedecodeerror,
    "UnicodeDecodeError"
);
make_exception_func!(
    builtin_make_exception_unicodeencodeerror,
    "UnicodeEncodeError"
);
make_exception_func!(builtin_make_exception_systemerror, "SystemError");
make_exception_func!(builtin_make_exception_warning, "Warning");
make_exception_func!(builtin_make_exception_userwarning, "UserWarning");
make_exception_func!(
    builtin_make_exception_deprecationwarning,
    "DeprecationWarning"
);
make_exception_func!(
    builtin_make_exception_pendingdeprecationwarning,
    "PendingDeprecationWarning"
);
make_exception_func!(builtin_make_exception_syntaxwarning, "SyntaxWarning");
make_exception_func!(builtin_make_exception_runtimewarning, "RuntimeWarning");
make_exception_func!(builtin_make_exception_futurewarning, "FutureWarning");
make_exception_func!(builtin_make_exception_importwarning, "ImportWarning");
make_exception_func!(builtin_make_exception_unicodewarning, "UnicodeWarning");
make_exception_func!(builtin_make_exception_byteswarning, "BytesWarning");
make_exception_func!(builtin_make_exception_resourcewarning, "ResourceWarning");
make_exception_func!(builtin_make_exception_referenceerror, "ReferenceError");
make_exception_func!(builtin_make_exception_buffererror, "BufferError");
make_exception_func!(builtin_make_exception_memoryerror, "MemoryError");
make_exception_func!(
    builtin_make_exception_notadirectoryerror,
    "NotADirectoryError"
);
make_exception_func!(
    builtin_make_exception_isadirectoryerror,
    "IsADirectoryError"
);
make_exception_func!(builtin_make_exception_fileexistserror, "FileExistsError");
make_exception_func!(
    builtin_make_exception_connectionabortederror,
    "ConnectionAbortedError"
);
make_exception_func!(
    builtin_make_exception_connectionreseterror,
    "ConnectionResetError"
);
make_exception_func!(
    builtin_make_exception_processlookuperror,
    "ProcessLookupError"
);
make_exception_func!(
    builtin_make_exception_unicodetranslateerror,
    "UnicodeTranslateError"
);
make_exception_func!(builtin_make_exception_indentationerror, "IndentationError");
make_exception_func!(builtin_make_exception_taberror, "TabError");

// ExceptionGroup and BaseExceptionGroup factory functions (PEP 654)
pub fn builtin_make_exception_exceptiongroup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _message = if !args.is_empty() {
        args[0].str()
    } else {
        "".to_string()
    };
    let exceptions = if args.len() > 1 {
        match &*args[1].borrow() {
            PyObject::List(items) => items.clone(),
            PyObject::Tuple(items) => items.clone(),
            _ => vec![],
        }
    } else {
        vec![]
    };
    Ok(PyObjectRef::new(PyObject::ExceptionGroup {
        typ: "ExceptionGroup".to_string(),
        args: args.to_vec(),
        exceptions,
    }))
}

pub fn builtin_make_exception_baseexceptiongroup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _message = if !args.is_empty() {
        args[0].str()
    } else {
        "".to_string()
    };
    let exceptions = if args.len() > 1 {
        match &*args[1].borrow() {
            PyObject::List(items) => items.clone(),
            PyObject::Tuple(items) => items.clone(),
            _ => vec![],
        }
    } else {
        vec![]
    };
    Ok(PyObjectRef::new(PyObject::ExceptionGroup {
        typ: "BaseExceptionGroup".to_string(),
        args: args.to_vec(),
        exceptions,
    }))
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn json_decode(s: &str) -> PyResult<PyObjectRef> {
    let s = s.trim();
    let mut chars = s.chars().peekable();
    json_parse_value(&mut chars)
}

fn json_parse_value<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
) -> PyResult<PyObjectRef> {
    json_skip_ws(chars);
    match chars.peek() {
        None => Err(PyError::ValueError(
            "Unexpected end of JSON input".to_string(),
        )),
        Some('"') => json_parse_string(chars),
        Some('t') | Some('f') => json_parse_bool(chars),
        Some('n') => json_parse_null(chars),
        Some('[') => json_parse_array(chars),
        Some('{') => json_parse_object(chars),
        Some(c) if c.is_ascii_digit() || *c == '-' => json_parse_number(chars),
        Some(c) => Err(PyError::ValueError(format!("Unexpected character '{}'", c))),
    }
}

fn json_skip_ws<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) {
    while let Some(&c) = chars.peek() {
        if c.is_ascii_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

fn json_parse_string<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
) -> PyResult<PyObjectRef> {
    chars.next();
    let mut s = String::new();
    loop {
        match chars.next() {
            None => return Err(PyError::ValueError("Unterminated JSON string".to_string())),
            Some('"') => return Ok(py_str(&s)),
            Some('\\') => match chars.next() {
                None => {
                    return Err(PyError::ValueError(
                        "Unexpected end of JSON string".to_string(),
                    ))
                }
                Some('"') => s.push('"'),
                Some('\\') => s.push('\\'),
                Some('/') => s.push('/'),
                Some('n') => s.push('\n'),
                Some('r') => s.push('\r'),
                Some('t') => s.push('\t'),
                Some('b') => s.push('\x08'),
                Some('f') => s.push('\x0c'),
                Some('u') => {
                    let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                    if hex.len() < 4 {
                        return Err(PyError::ValueError("Invalid Unicode escape".to_string()));
                    }
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(code) {
                            s.push(c);
                        }
                    }
                }
                Some(c) => s.push(c),
            },
            Some(c) => s.push(c),
        }
    }
}

fn json_parse_bool<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
) -> PyResult<PyObjectRef> {
    let s: String = chars.by_ref().take(5).collect();
    if s.starts_with("true") {
        Ok(py_bool(true))
    } else if s.starts_with("false") {
        Ok(py_bool(false))
    } else {
        Err(PyError::ValueError(format!("Unexpected token '{}'", s)))
    }
}

fn json_parse_null<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
) -> PyResult<PyObjectRef> {
    let s: String = chars.by_ref().take(4).collect();
    if s.starts_with("null") {
        Ok(py_none())
    } else {
        Err(PyError::ValueError(format!("Unexpected token '{}'", s)))
    }
}

fn json_parse_number<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
) -> PyResult<PyObjectRef> {
    let mut num = String::new();
    if let Some(&'-') = chars.peek() {
        num.push(chars.next().unwrap());
    }
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            num.push(chars.next().unwrap());
        } else {
            break;
        }
    }
    if let Some(&'.') = chars.peek() {
        num.push(chars.next().unwrap());
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                num.push(chars.next().unwrap());
            } else {
                break;
            }
        }
        let peek_lower = chars.peek().map(|c| c.to_ascii_lowercase());
        if peek_lower == Some('e') {
            num.push(chars.next().unwrap());
            if let Some(&'+') | Some(&'-') = chars.peek() {
                num.push(chars.next().unwrap());
            }
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    num.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
        }
        Ok(py_float(num.parse::<f64>().map_err(|_| {
            PyError::ValueError(format!("Invalid number: {}", num))
        })?))
    } else {
        Ok(py_int(num.parse::<i64>().map_err(|_| {
            PyError::ValueError(format!("Invalid integer: {}", num))
        })?))
    }
}

fn json_parse_array<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
) -> PyResult<PyObjectRef> {
    chars.next();
    let mut items = Vec::new();
    loop {
        json_skip_ws(chars);
        match chars.peek() {
            None => return Err(PyError::ValueError("Unterminated JSON array".to_string())),
            Some(&']') => {
                chars.next();
                return Ok(py_list(items));
            }
            Some(&',') => {
                chars.next();
                continue;
            }
            _ => {
                items.push(json_parse_value(chars)?);
            }
        }
    }
}

fn json_parse_object<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
) -> PyResult<PyObjectRef> {
    chars.next();
    let dict = py_dict();
    loop {
        json_skip_ws(chars);
        match chars.peek() {
            None => return Err(PyError::ValueError("Unterminated JSON object".to_string())),
            Some(&'}') => {
                chars.next();
                return Ok(dict);
            }
            Some(&',') => {
                chars.next();
                continue;
            }
            Some(&'"') => {
                let key = json_parse_string(chars)?;
                json_skip_ws(chars);
                match chars.next() {
                    Some(':') => {}
                    Some(c) => {
                        return Err(PyError::ValueError(format!("Expected ':' got '{}'", c)))
                    }
                    None => {
                        return Err(PyError::ValueError(
                            "Unexpected end of JSON object".to_string(),
                        ))
                    }
                }
                let val = json_parse_value(chars)?;
                if let PyObject::Dict(d) = &mut *dict.borrow_mut() {
                    d.set(key, val)?;
                }
            }
            Some(c) => {
                return Err(PyError::ValueError(format!(
                    "Unexpected token '{}' in object",
                    c
                )))
            }
        }
    }
}

pub fn json_encode_full(
    val: &PyObjectRef,
    indent: Option<usize>,
    sort_keys: bool,
    level: usize,
) -> PyResult<PyObjectRef> {
    match &*val.borrow() {
        PyObject::None => Ok(py_str("null")),
        PyObject::Bool(b) => Ok(py_str(if *b { "true" } else { "false" })),
        PyObject::Int(i) => Ok(py_str(&i.to_string())),
        PyObject::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                return Err(PyError::ValueError(
                    "Out of range float values are not JSON compliant".to_string(),
                ));
            }
            Ok(py_str(&f.to_string()))
        }
        PyObject::Str(s) => Ok(py_str(&json_escape_string(s))),
        PyObject::List(items) | PyObject::Tuple(items) => {
            if indent.is_some() {
                let inner_indent = indent.unwrap_or(4);
                let pad = " ".repeat(inner_indent * (level + 1));
                let close_pad = " ".repeat(inner_indent * level);
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    let encoded = json_encode_full(item, indent, sort_keys, level + 1)?;
                    parts.push(format!("\n{}{}", pad, encoded.str()));
                }
                if parts.is_empty() {
                    Ok(py_str("[]"))
                } else {
                    Ok(py_str(&format!("[{}\n{}]", parts.join(","), close_pad)))
                }
            } else {
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    let encoded = json_encode_full(item, indent, sort_keys, level + 1)?;
                    parts.push(encoded.str());
                }
                Ok(py_str(&format!("[{}]", parts.join(", "))))
            }
        }
        PyObject::Dict(d) => {
            let pairs: Vec<(String, String)> = if sort_keys {
                let mut sorted: Vec<(String, String)> = d
                    .items()
                    .iter()
                    .map(|(k, v)| {
                        let key_obj = json_encode_full(k, indent, sort_keys, level + 1)
                            .unwrap_or_else(|_| py_str("\"?\""));
                        let val_obj = json_encode_full(v, indent, sort_keys, level + 1)
                            .unwrap_or_else(|_| py_str("null"));
                        (k.str(), format!("{}: {}", key_obj.str(), val_obj.str()))
                    })
                    .collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                sorted
            } else {
                d.items()
                    .iter()
                    .map(|(k, v)| {
                        let key_obj = json_encode_full(k, indent, sort_keys, level + 1)
                            .unwrap_or_else(|_| py_str("\"?\""));
                        let val_obj = json_encode_full(v, indent, sort_keys, level + 1)
                            .unwrap_or_else(|_| py_str("null"));
                        (
                            String::new(),
                            format!("{}: {}", key_obj.str(), val_obj.str()),
                        )
                    })
                    .collect()
            };
            if indent.is_some() {
                let inner_indent = indent.unwrap_or(4);
                let pad = " ".repeat(inner_indent * (level + 1));
                let close_pad = " ".repeat(inner_indent * level);
                let items: Vec<String> = pairs
                    .iter()
                    .map(|(_, v)| format!("\n{}{}", pad, v))
                    .collect();
                if items.is_empty() {
                    Ok(py_str("{}"))
                } else {
                    Ok(py_str(&format!("{{{}\n{}}}", items.join(","), close_pad)))
                }
            } else {
                let items: Vec<String> = pairs.iter().map(|(_, v)| v.clone()).collect();
                Ok(py_str(&format!("{{{}}}", items.join(", "))))
            }
        }
        _ => Err(PyError::type_error(format!(
            "Object of type '{}' is not JSON serializable",
            val.borrow().type_name()
        ))),
    }
}

// Late additions — PythonFinalizationError (RuntimeError subclass) and
// EncodingWarning (Warning subclass) were registered in CPython but missing
// here entirely, so `issubclass(EncodingWarning, Warning)`/`issubclass(
// PythonFinalizationError, RuntimeError)` (test_baseexception's
// exception-hierarchy audit) found nothing to resolve.
make_exception_func!(
    builtin_make_exception_pythonfinalizationerror,
    "PythonFinalizationError"
);
make_exception_func!(builtin_make_exception_encodingwarning, "EncodingWarning");
