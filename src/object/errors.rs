// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds `PyError` (the
// interpreter's internal Rust-level error enum) and `PyResult`.
use super::*;
use std::fmt;

#[derive(Debug, Clone)]
pub enum PyError {
    TypeError(String),
    ValueError(String),
    NameError(String),
    AttributeError(String),
    IndexError(String),
    KeyError(String),
    ZeroDivisionError(String),
    RuntimeError(String),
    SystemExit(i32),
    Exception(String, PyObjectRef),
    StopIteration,
    OsError(String),
    ImportError(String),
    RecursionError(String),
}

impl PyError {
    pub fn type_name(&self) -> &str {
        match self {
            PyError::TypeError(_) => "TypeError",
            PyError::ValueError(_) => "ValueError",
            PyError::NameError(_) => "NameError",
            PyError::AttributeError(_) => "AttributeError",
            PyError::IndexError(_) => "IndexError",
            PyError::KeyError(_) => "KeyError",
            PyError::ZeroDivisionError(_) => "ZeroDivisionError",
            PyError::RuntimeError(_) => "RuntimeError",
            PyError::SystemExit(_) => "SystemExit",
            PyError::Exception(_, _) => "Exception",
            PyError::StopIteration => "StopIteration",
            PyError::OsError(_) => "OSError",
            PyError::ImportError(_) => "ImportError",
            PyError::RecursionError(_) => "RecursionError",
        }
    }
    pub fn type_error(msg: impl Into<String>) -> Self {
        PyError::TypeError(msg.into())
    }
    pub fn name_error(msg: impl Into<String>) -> Self {
        PyError::NameError(msg.into())
    }
    pub fn value_error(msg: impl Into<String>) -> Self {
        PyError::ValueError(msg.into())
    }
    pub fn zero_division() -> Self {
        PyError::ZeroDivisionError("division by zero".to_string())
    }
    pub fn attribute_error(msg: impl Into<String>) -> Self {
        PyError::AttributeError(msg.into())
    }
    pub fn index_error(msg: impl Into<String>) -> Self {
        PyError::IndexError(msg.into())
    }
    pub fn key_error(msg: impl Into<String>) -> Self {
        PyError::KeyError(msg.into())
    }
    pub fn stop_iteration() -> Self {
        PyError::StopIteration
    }
    /// A real `SyntaxError` (typ + message, matching the established
    /// `PyError::Exception(name, obj)` pattern used elsewhere for
    /// dynamically-named exceptions like `UnicodeDecodeError`) — NOT
    /// `PyError::TypeError`, which is what every parse-error site used to
    /// raise instead (confirmed via `compile("x=", "<f>", "exec")`,
    /// previously uncatchable by `except SyntaxError:`, an extremely
    /// standard idiom for validating/pre-checking source code).
    pub fn syntax_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception("SyntaxError".to_string(), PyObjectRef::new(PyObject::Exception {
            typ: "SyntaxError".to_string(),
            args: vec![py_str(&msg)],
            cause: None,
        }))
    }
    pub fn memory_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception("MemoryError".to_string(), PyObjectRef::new(PyObject::Exception {
            typ: "MemoryError".to_string(),
            args: vec![py_str(&msg)],
            cause: None,
        }))
    }
    /// Same `PyError::Exception(name, obj)` pattern as `syntax_error`/
    /// `memory_error` above — `OverflowError` has no dedicated `PyError`
    /// enum variant, so this is how any native function raises it directly
    /// (e.g. `math.pow`'s own genuine-overflow case).
    pub fn overflow_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception("OverflowError".to_string(), PyObjectRef::new(PyObject::Exception {
            typ: "OverflowError".to_string(),
            args: vec![py_str(&msg)],
            cause: None,
        }))
    }
    pub fn runtime_error(msg: impl Into<String>) -> Self {
        PyError::RuntimeError(msg.into())
    }
    pub fn recursion_error(msg: impl Into<String>) -> Self {
        PyError::RecursionError(msg.into())
    }
    pub fn message(&self) -> String {
        match self {
            PyError::TypeError(m) => m.clone(),
            PyError::ValueError(m) => m.clone(),
            PyError::NameError(m) => m.clone(),
            PyError::AttributeError(m) => m.clone(),
            PyError::IndexError(m) => m.clone(),
            PyError::KeyError(m) => m.clone(),
            PyError::ZeroDivisionError(m) => m.clone(),
            PyError::RuntimeError(m) => m.clone(),
            PyError::SystemExit(c) => format!("SystemExit({})", c),
            PyError::Exception(m, exc) => {
                // `m` is often just a dispatch tag (e.g. "re-raise" for a
                // reraised with/finally exception) rather than the real
                // message — the actual args live on the wrapped exception
                // object itself.
                match &*exc.borrow() {
                    PyObject::Exception { args, .. } => {
                        match args.len() {
                            0 => String::new(),
                            1 => args[0].str(),
                            _ => args.iter().map(|a| a.str()).collect::<Vec<_>>().join(", "),
                        }
                    }
                    // A user-defined exception class (`class Foo(Exception):
                    // ...`, raised/reraised — e.g. Django's real
                    // `ImproperlyConfigured`) is a plain `PyObject::Instance`,
                    // not our internal `PyObject::Exception` representation —
                    // without this arm every such exception displayed as the
                    // generic dispatch tag ("Exception: re-raise") instead of
                    // its real class name and message, the moment it passed
                    // through a `with`/`finally` (RERAISE) or `except: raise`
                    // (bare reraise) — both extremely common. `args` here
                    // mirrors what `Exception.__init__` conventionally
                    // stores (`self.args = args`), read directly rather than
                    // calling `__str__` (no VM access from this plain method).
                    PyObject::Instance { dict, .. } => {
                        match dict.get("args") {
                            Some(args_obj) => {
                                let is_tuple = matches!(&*args_obj.borrow(), PyObject::Tuple(_));
                                if is_tuple {
                                    let items = if let PyObject::Tuple(items) = &*args_obj.borrow() { items.clone() } else { unreachable!() };
                                    match items.len() {
                                        0 => String::new(),
                                        1 => items[0].str(),
                                        _ => items.iter().map(|a| a.str()).collect::<Vec<_>>().join(", "),
                                    }
                                } else {
                                    args_obj.str()
                                }
                            }
                            None => String::new(),
                        }
                    }
                    _ => m.clone(),
                }
            }
            PyError::StopIteration => "".to_string(),
            PyError::OsError(m) => m.clone(),
            PyError::ImportError(m) => m.clone(),
            PyError::RecursionError(m) => m.clone(),
        }
    }

    /// The exception's real type name for display — for `PyError::Exception`
    /// this must come from the wrapped PyObject::Exception's own `typ`
    /// field, not the outer variant's dispatch tag (which is often a
    /// generic placeholder like "re-raise", not the actual exception type).
    pub fn type_name_for_display(&self) -> String {
        match self {
            PyError::Exception(_, exc) => match &*exc.borrow() {
                PyObject::Exception { typ, .. } => typ.clone(),
                PyObject::ExceptionGroup { typ, .. } => typ.clone(),
                // User-defined exception class instance (`class Foo(Exception)`)
                // — see the matching arm in `message()` for why this matters.
                PyObject::Instance { typ, .. } => get_type_name_for_instance(typ),
                _ => "Exception".to_string(),
            },
            PyError::TypeError(_) => "TypeError".to_string(),
            PyError::ValueError(_) => "ValueError".to_string(),
            PyError::NameError(_) => "NameError".to_string(),
            PyError::AttributeError(_) => "AttributeError".to_string(),
            PyError::IndexError(_) => "IndexError".to_string(),
            PyError::KeyError(_) => "KeyError".to_string(),
            PyError::ZeroDivisionError(_) => "ZeroDivisionError".to_string(),
            PyError::RuntimeError(_) => "RuntimeError".to_string(),
            PyError::SystemExit(_) => "SystemExit".to_string(),
            PyError::StopIteration => "StopIteration".to_string(),
            PyError::OsError(_) => "OSError".to_string(),
            PyError::ImportError(_) => "ImportError".to_string(),
            PyError::RecursionError(_) => "RecursionError".to_string(),
        }
    }
}

impl fmt::Display for PyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.type_name_for_display(), self.message())
    }
}

pub type PyResult<T> = Result<T, PyError>;
