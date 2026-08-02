// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the builtin
// exception constructor functions (one per exception type, generated via
// the `make_exception_func!` macro).
use super::*;

// ---- Exception constructor functions ----

macro_rules! make_exception_func {
    ($name:ident, $typ:expr) => {
        pub fn $name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            Ok(PyObjectRef::new(PyObject::Exception { typ: $typ.to_string(), args: args.to_vec(), cause: None }))
        }
    };
}

make_exception_func!(builtin_make_exception_baseexception, "BaseException");
make_exception_func!(builtin_make_exception_exception, "Exception");
make_exception_func!(builtin_make_exception_typeerror, "TypeError");
make_exception_func!(builtin_make_exception_valueerror, "ValueError");
make_exception_func!(builtin_make_exception_zerodivisionerror, "ZeroDivisionError");
make_exception_func!(builtin_make_exception_nameerror, "NameError");
make_exception_func!(builtin_make_exception_unboundlocalerror, "UnboundLocalError");
make_exception_func!(builtin_make_exception_attributeerror, "AttributeError");
make_exception_func!(builtin_make_exception_indexerror, "IndexError");
make_exception_func!(builtin_make_exception_keyerror, "KeyError");
make_exception_func!(builtin_make_exception_runtimeerror, "RuntimeError");
make_exception_func!(builtin_make_exception_stopiteration, "StopIteration");
make_exception_func!(builtin_make_exception_assertionerror, "AssertionError");
make_exception_func!(builtin_make_exception_oserror, "OSError");
make_exception_func!(builtin_make_exception_importerror, "ImportError");
make_exception_func!(builtin_make_exception_pickleerror, "PickleError");
make_exception_func!(builtin_make_exception_picklingerror, "PicklingError");
make_exception_func!(builtin_make_exception_unpicklingerror, "UnpicklingError");
// Additional exception types for full CPython hierarchy
make_exception_func!(builtin_make_exception_lookuperror, "LookupError");
make_exception_func!(builtin_make_exception_arithmeticerror, "ArithmeticError");
make_exception_func!(builtin_make_exception_floatingpointerror, "FloatingPointError");
make_exception_func!(builtin_make_exception_overflowerror, "OverflowError");
make_exception_func!(builtin_make_exception_environmenterror, "EnvironmentError");
make_exception_func!(builtin_make_exception_ioerror, "IOError");
make_exception_func!(builtin_make_exception_filenotfounderror, "FileNotFoundError");
make_exception_func!(builtin_make_exception_permissionerror, "PermissionError");
make_exception_func!(builtin_make_exception_notimplementederror, "NotImplementedError");
make_exception_func!(builtin_make_exception_recursionerror, "RecursionError");
make_exception_func!(builtin_make_exception_keyboardinterrupt, "KeyboardInterrupt");
make_exception_func!(builtin_make_exception_generatorexit, "GeneratorExit");
make_exception_func!(builtin_make_exception_systemexit, "SystemExit");
make_exception_func!(builtin_make_exception_modulenotfounderror, "ModuleNotFoundError");
make_exception_func!(builtin_make_exception_stopasynciteration, "StopAsyncIteration");
make_exception_func!(builtin_make_exception_eoferror, "EOFError");
make_exception_func!(builtin_make_exception_syntaxerror, "SyntaxError");
make_exception_func!(builtin_make_exception_cycleerror, "CycleError");
make_exception_func!(builtin_make_exception_decimalexception, "DecimalException");
make_exception_func!(builtin_make_exception_invalidoperation, "InvalidOperation");
make_exception_func!(builtin_make_exception_decimaldivisionbyzero, "DivisionByZero");
make_exception_func!(builtin_make_exception_inexact, "Inexact");
make_exception_func!(builtin_make_exception_rounded, "Rounded");
make_exception_func!(builtin_make_exception_clamped, "Clamped");
make_exception_func!(builtin_make_exception_decimaloverflow, "Overflow");
make_exception_func!(builtin_make_exception_decimalunderflow, "Underflow");
make_exception_func!(builtin_make_exception_floatoperation, "FloatOperation");
make_exception_func!(builtin_make_exception_connectionerror, "ConnectionError");
make_exception_func!(builtin_make_exception_brokenpipeerror, "BrokenPipeError");
make_exception_func!(builtin_make_exception_connectionrefusederror, "ConnectionRefusedError");
make_exception_func!(builtin_make_exception_blockingioerror, "BlockingIOError");
make_exception_func!(builtin_make_exception_childprocesserror, "ChildProcessError");
make_exception_func!(builtin_make_exception_interruptederror, "InterruptedError");
make_exception_func!(builtin_make_exception_timeouterror, "TimeoutError");
make_exception_func!(builtin_make_exception_unicodeerror, "UnicodeError");
make_exception_func!(builtin_make_exception_unicodedecodeerror, "UnicodeDecodeError");
make_exception_func!(builtin_make_exception_unicodeencodeerror, "UnicodeEncodeError");
make_exception_func!(builtin_make_exception_systemerror, "SystemError");
make_exception_func!(builtin_make_exception_warning, "Warning");
make_exception_func!(builtin_make_exception_userwarning, "UserWarning");
make_exception_func!(builtin_make_exception_deprecationwarning, "DeprecationWarning");
make_exception_func!(builtin_make_exception_pendingdeprecationwarning, "PendingDeprecationWarning");
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
make_exception_func!(builtin_make_exception_notadirectoryerror, "NotADirectoryError");
make_exception_func!(builtin_make_exception_isadirectoryerror, "IsADirectoryError");
make_exception_func!(builtin_make_exception_fileexistserror, "FileExistsError");
make_exception_func!(builtin_make_exception_connectionabortederror, "ConnectionAbortedError");
make_exception_func!(builtin_make_exception_connectionreseterror, "ConnectionResetError");
make_exception_func!(builtin_make_exception_processlookuperror, "ProcessLookupError");
make_exception_func!(builtin_make_exception_unicodetranslateerror, "UnicodeTranslateError");
make_exception_func!(builtin_make_exception_indentationerror, "IndentationError");
make_exception_func!(builtin_make_exception_taberror, "TabError");

// ExceptionGroup and BaseExceptionGroup factory functions (PEP 654)
pub fn builtin_make_exception_exceptiongroup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _message = if !args.is_empty() { args[0].str() } else { "".to_string() };
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
    let _message = if !args.is_empty() { args[0].str() } else { "".to_string() };
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

fn json_parse_value<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    json_skip_ws(chars);
    match chars.peek() {
        None => Err(PyError::ValueError("Unexpected end of JSON input".to_string())),
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
        if c.is_ascii_whitespace() { chars.next(); }
        else { break; }
    }
}

fn json_parse_string<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    chars.next();
    let mut s = String::new();
    loop {
        match chars.next() {
            None => return Err(PyError::ValueError("Unterminated JSON string".to_string())),
            Some('"') => return Ok(py_str(&s)),
            Some('\\') => {
                match chars.next() {
                    None => return Err(PyError::ValueError("Unexpected end of JSON string".to_string())),
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
                        if hex.len() < 4 { return Err(PyError::ValueError("Invalid Unicode escape".to_string())); }
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(code) {
                                s.push(c);
                            }
                        }
                    }
                    Some(c) => s.push(c),
                }
            }
            Some(c) => s.push(c),
        }
    }
}

fn json_parse_bool<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    let s: String = chars.by_ref().take(5).collect();
    if s.starts_with("true") { Ok(py_bool(true)) }
    else if s.starts_with("false") { Ok(py_bool(false)) }
    else { Err(PyError::ValueError(format!("Unexpected token '{}'", s))) }
}

fn json_parse_null<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    let s: String = chars.by_ref().take(4).collect();
    if s.starts_with("null") { Ok(py_none()) }
    else { Err(PyError::ValueError(format!("Unexpected token '{}'", s))) }
}

fn json_parse_number<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    let mut num = String::new();
    if let Some(&'-') = chars.peek() { num.push(chars.next().unwrap()); }
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() { num.push(chars.next().unwrap()); }
        else { break; }
    }
    if let Some(&'.') = chars.peek() {
        num.push(chars.next().unwrap());
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() { num.push(chars.next().unwrap()); }
            else { break; }
        }
        let peek_lower = chars.peek().map(|c| c.to_ascii_lowercase());
        if peek_lower == Some('e') {
            num.push(chars.next().unwrap());
            if let Some(&'+') | Some(&'-') = chars.peek() { num.push(chars.next().unwrap()); }
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() { num.push(chars.next().unwrap()); }
                else { break; }
            }
        }
        Ok(py_float(num.parse::<f64>().map_err(|_| PyError::ValueError(format!("Invalid number: {}", num)))?))
    } else {
        Ok(py_int(num.parse::<i64>().map_err(|_| PyError::ValueError(format!("Invalid integer: {}", num)))?))
    }
}

fn json_parse_array<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    chars.next();
    let mut items = Vec::new();
    loop {
        json_skip_ws(chars);
        match chars.peek() {
            None => return Err(PyError::ValueError("Unterminated JSON array".to_string())),
            Some(&']') => { chars.next(); return Ok(py_list(items)); }
            Some(&',') => { chars.next(); continue; }
            _ => { items.push(json_parse_value(chars)?); }
        }
    }
}

fn json_parse_object<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    chars.next();
    let dict = py_dict();
    loop {
        json_skip_ws(chars);
        match chars.peek() {
            None => return Err(PyError::ValueError("Unterminated JSON object".to_string())),
            Some(&'}') => { chars.next(); return Ok(dict); }
            Some(&',') => { chars.next(); continue; }
            Some(&'"') => {
                let key = json_parse_string(chars)?;
                json_skip_ws(chars);
                match chars.next() {
                    Some(':') => {}
                    Some(c) => return Err(PyError::ValueError(format!("Expected ':' got '{}'", c))),
                    None => return Err(PyError::ValueError("Unexpected end of JSON object".to_string())),
                }
                let val = json_parse_value(chars)?;
                if let PyObject::Dict(d) = &mut *dict.borrow_mut() {
                    d.set(key, val)?;
                }
            }
            Some(c) => return Err(PyError::ValueError(format!("Unexpected token '{}' in object", c))),
        }
    }
}

pub fn json_encode_full(val: &PyObjectRef, indent: Option<usize>, sort_keys: bool, level: usize) -> PyResult<PyObjectRef> {
    match &*val.borrow() {
        PyObject::None => Ok(py_str("null")),
        PyObject::Bool(b) => Ok(py_str(if *b { "true" } else { "false" })),
        PyObject::Int(i) => Ok(py_str(&i.to_string())),
        PyObject::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                return Err(PyError::ValueError("Out of range float values are not JSON compliant".to_string()));
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
                let mut sorted: Vec<(String, String)> = d.items().iter()
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
                d.items().iter().map(|(k, v)| {
                    let key_obj = json_encode_full(k, indent, sort_keys, level + 1)
                        .unwrap_or_else(|_| py_str("\"?\""));
                    let val_obj = json_encode_full(v, indent, sort_keys, level + 1)
                        .unwrap_or_else(|_| py_str("null"));
                    (String::new(), format!("{}: {}", key_obj.str(), val_obj.str()))
                }).collect()
            };
            if indent.is_some() {
                let inner_indent = indent.unwrap_or(4);
                let pad = " ".repeat(inner_indent * (level + 1));
                let close_pad = " ".repeat(inner_indent * level);
                let items: Vec<String> = pairs.iter().map(|(_, v)| format!("\n{}{}", pad, v)).collect();
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
        _ => Err(PyError::type_error(format!("Object of type '{}' is not JSON serializable", val.borrow().type_name()))),
    }
}

pub fn call_function(func: &PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
    let f = func.borrow();
    match &*f {
        PyObject::BuiltinFunction { func: bf, .. } => {
            return bf(&args);
        }
        PyObject::Closure(func) => {
            return func(&args);
        }
        _ => {}
    }
    drop(f);
    Err(PyError::type_error(format!("'{}' object is not callable", func.borrow().type_name())))
}

static RNG_STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn fast_random_u64() -> u64 {
    RNG_STATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

pub(crate) fn socket_addr_to_string(addr: &PyObjectRef) -> PyResult<String> {
    let borrowed = addr.borrow();
    match &*borrowed {
        PyObject::Tuple(items) if items.len() == 2 => {
            let host = items[0].str();
            let port = items[1].as_i64().ok_or_else(|| PyError::type_error("port must be int"))?;
            Ok(format!("{}:{}", host, port))
        }
        PyObject::Str(s) => Ok(s.to_string()),
        _ => {
            // Fallback: use str representation
            Ok(addr.str())
        }
    }
}

/// Real `socket.getsockname()`/`getpeername()`/`accept()`'s address element
/// return a `(host, port)` tuple for AF_INET/AF_INET6, matching CPython —
/// the inverse of `socket_addr_to_string` above.
pub(crate) fn socket_addr_to_py_tuple(addr: std::net::SocketAddr) -> PyObjectRef {
    py_tuple(vec![py_str(&addr.ip().to_string()), py_int(addr.port() as i64)])
}

pub struct ThreadInner {
    pub handle: Option<std::thread::JoinHandle<()>>,
    pub result: std::sync::Arc<std::sync::Mutex<Option<PyObjectRef>>>,
    pub target: PyObjectRef,
    pub args: Vec<PyObjectRef>,
    // `handle` is NEVER actually populated (see `start()`'s own comment:
    // `PyObjectRef` is `!Send`, so the "thread" runs synchronously in-place
    // rather than spawning a real OS thread) — `join()` used to check
    // `handle.is_some()` to decide whether `start()` had been called at
    // all, which was ALWAYS false, so `t.start(); t.join()` (an extremely
    // common, correct usage) always raised `RuntimeError: thread not
    // started` even though `start()` legitimately ran (synchronously) to
    // completion. This separate flag tracks "was `start()` called" —
    // independent of whether a real join handle exists.
    pub started: bool,
}

#[derive(Clone)]
pub struct PyArray {
    pub typecode: char,
    pub data: Vec<f64>,
}

/// `array.array` typecodes `f`/`d` read back as `float`; every other real
/// typecode (`bBuhHiIlLqQ`) reads back as `int` — shared by the constructor
/// (`misc.rs`) and both element-access sites (`pyobject.rs`'s `repr`,
/// `subscript.rs`'s `__getitem__`), which previously only special-cased
/// `'i'` and treated every other typecode (including plain integer ones
/// like `'B'`/`'h'`/`'q'`) as float.
pub(crate) fn array_typecode_is_float(tc: char) -> bool {
    tc == 'f' || tc == 'd'
}

pub struct LockInner {
    pub lock: std::sync::atomic::AtomicBool,
}

pub struct RLockInner {
    pub owner: Option<std::thread::ThreadId>,
    pub count: u32,
}

pub struct EventInner {
    pub flag: std::sync::Mutex<bool>,
    pub condvar: std::sync::Condvar,
}

pub struct QueueInner {
    pub queue: std::collections::VecDeque<PyObjectRef>,
}

pub fn create_module(name: &str, dict: HashMap<String, PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Module {
        name: name.to_string(),
        dict: Box::new(str_map_to_typedict(dict)),
    })
}

/// Helper: deep-copy a single object with memo support
pub fn deepcopy_one(obj: &PyObjectRef, memo: &PyObjectRef) -> Result<PyObjectRef, PyError> {
    // Check memo first using identity
    if let PyObject::Dict(memo_dict) = &*memo.borrow() {
        if let Some(cached) = memo_dict.get_by_identity(obj) {
            return Ok(cached);
        }
    }
    // Uses `set_by_identity` (bypasses `.hash()`) — NOT the ordinary
    // `set()`, which would call `key.hash()` and get `Err("unhashable
    // type")` for exactly the container types (dict/list/set) most likely
    // to need cycle protection here, silently failing to store anything.
    fn remember(memo: &PyObjectRef, orig: &PyObjectRef, copy: &PyObjectRef) {
        if let PyObject::Dict(memo_dict) = &mut *memo.borrow_mut() {
            memo_dict.set_by_identity(orig.clone(), copy.clone());
        }
    }
    // List/Dict are MUTABLE, so a self- or mutually-referential structure
    // (`d = {}; d['self'] = d`) is directly constructible in real Python —
    // deep-copying one must therefore create the new (still-empty)
    // container and register it in `memo` BEFORE recursing into its
    // children, so a child that refers back to the original finds the
    // (partially-built) copy already memoized instead of recursing forever.
    // The previous version only called `remember` AFTER fully copying all
    // children — for a self-referential dict/list, the recursive call for
    // the self-reference would run before its own entry ever got memoized,
    // recursing without end and overflowing the native stack (confirmed via
    // CPython's own `test_copy.py::test_deepcopy_reflexive_dict`).
    match &*obj.borrow() {
        PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) | PyObject::Bool(_) | PyObject::None | PyObject::Bytes(_) => Ok(obj.clone()),
        PyObject::List(_) => {
            let new_list = py_list(Vec::new());
            remember(memo, obj, &new_list);
            let items = if let PyObject::List(items) = &*obj.borrow() { items.clone() } else { unreachable!() };
            let mut new_items = Vec::with_capacity(items.len());
            for item in &items {
                new_items.push(deepcopy_one(item, memo)?);
            }
            if let PyObject::List(nl) = &mut *new_list.borrow_mut() {
                *nl = new_items;
            }
            Ok(new_list)
        }
        PyObject::Deque { .. } => {
            let new_deque = py_deque(std::collections::VecDeque::new(), None);
            remember(memo, obj, &new_deque);
            let (items, maxlen) = if let PyObject::Deque { data, maxlen } = &*obj.borrow() {
                (data.iter().cloned().collect::<Vec<_>>(), *maxlen)
            } else { unreachable!() };
            let mut new_data = std::collections::VecDeque::new();
            for item in &items {
                new_data.push_back(deepcopy_one(item, memo)?);
            }
            if let PyObject::Deque { data, maxlen: ml } = &mut *new_deque.borrow_mut() {
                *data = new_data;
                *ml = maxlen;
            }
            Ok(new_deque)
        }
        PyObject::Dict(_) => {
            let new_dict = PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new())));
            remember(memo, obj, &new_dict);
            let items = if let PyObject::Dict(d) = &*obj.borrow() { d.items() } else { unreachable!() };
            for (k, v) in items {
                let new_k = deepcopy_one(&k, memo)?;
                let new_v = deepcopy_one(&v, memo)?;
                if let PyObject::Dict(nd) = &mut *new_dict.borrow_mut() {
                    let _ = nd.set(new_k, new_v);
                }
            }
            Ok(new_dict)
        }
        // Tuples are immutable, so a PURE tuple-only cycle can never exist
        // in real Python (a tuple can only reference already-fully-built
        // objects) — no placeholder-first trick needed here, just the
        // ordinary "build children, then memoize the final result" shape
        // (still useful for diamond references: the same tuple appearing
        // twice in one structure should deep-copy to the same new object).
        PyObject::Tuple(items) => {
            let items = items.clone();
            let mut new_items = Vec::with_capacity(items.len());
            for item in &items {
                new_items.push(deepcopy_one(item, memo)?);
            }
            let result = PyObjectRef::imm(PyObject::Tuple(new_items));
            remember(memo, obj, &result);
            Ok(result)
        }
        // A `slice`'s `start`/`stop`/`step` can be arbitrary (mutable)
        // objects, not just ints (see the `.start`/`.stop`/`.step`
        // attribute-getter's own doc comment in `attrs.rs`) — was falling
        // to the generic `_` fallback below, which has no
        // `native_backing_of`/`__deepcopy__` for a plain `Slice` and so
        // just cloned the `Rc`, returning the SAME object. Real Python
        // deep-copies each of the three fields independently (confirmed:
        // `test_slice.py::test_deepcopy`'s "corner case for mutable
        // indices", `slice([1,2],[3,4],[5,6])`, asserts the copy `is not`
        // the original AND each field `is not` its original counterpart).
        PyObject::Slice { start, stop, step } => {
            let (start, stop, step) = (start.clone(), stop.clone(), step.clone());
            let new_start = deepcopy_one(&start, memo)?;
            let new_stop = deepcopy_one(&stop, memo)?;
            let new_step = deepcopy_one(&step, memo)?;
            let result = PyObjectRef::imm(PyObject::Slice { start: new_start, stop: new_stop, step: new_step });
            remember(memo, obj, &result);
            Ok(result)
        }
        _ => {
            // Custom `__deepcopy__` takes priority (matching real Python's
            // `copy.deepcopy` protocol) — without this, an Instance nested
            // inside a list/dict/tuple being deep-copied always got a bare
            // shallow `.clone()` instead of ever invoking its own
            // `__deepcopy__`.
            if let Ok(dc_method) = obj.borrow().get_attribute("__deepcopy__") {
                let result = call_function(&dc_method, vec![obj.clone(), memo.clone()])?;
                remember(memo, obj, &result);
                return Ok(result);
            }
            // Same native-base-subclass gap as `copy.copy`'s own fallback
            // (`misc.rs`) — a class transparently subclassing a native
            // container with no `__deepcopy__` override fell straight to
            // `obj.clone()` (an `Rc` clone, the SAME object), instead of
            // recursively deep-copying its actual contents. Deep-copy the
            // native backing's elements (not just a shallow copy of the
            // top-level container, unlike `copy.copy`) and wrap the result
            // in a NEW `Instance` of the same class.
            if let Some(native) = native_backing_of(obj) {
                let placeholder = PyObjectRef::new(PyObject::None);
                remember(memo, obj, &placeholder);
                let new_native = deepcopy_one(&native, memo)?;
                let (typ, dict) = if let PyObject::Instance { typ, dict } = &*obj.borrow() {
                    (typ.clone(), dict.clone())
                } else {
                    unreachable!()
                };
                let mut new_dict = dict;
                new_dict.insert(NATIVE_BACKING_KEY.to_string(), new_native);
                let result = PyObjectRef::new(PyObject::Instance { typ, dict: new_dict });
                if let PyObject::Dict(memo_dict) = &mut *memo.borrow_mut() {
                    memo_dict.set_by_identity(obj.clone(), result.clone());
                }
                return Ok(result);
            }
            let result = obj.clone();
            remember(memo, obj, &result);
            Ok(result)
        }
    }
}

