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
    /// `NameError: name 'x' is not defined` carrying the real attribute
    /// `.name` (test_exceptions::NameErrorTests::test_name_error_has_name).
    pub fn name_error_for(name: &str) -> Self {
        let mut extra = std::collections::HashMap::new();
        extra.insert("name".to_string(), py_str(name));
        PyError::Exception(
            "NameError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "NameError".to_string(),
                args: vec![py_str(&format!("name '{}' is not defined", name))],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: Some(extra),
            }),
        )
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
    pub fn key_error_obj(key: &PyObjectRef) -> Self {
        let obj = PyObjectRef::new(PyObject::Exception {
            typ: "KeyError".to_string(),
            args: vec![key.clone()],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        });
        PyError::Exception("KeyError".to_string(), obj)
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
        Self::syntax_error_with_filename(msg, "<string>", "")
    }
    /// Like `syntax_error` but with the real filename and the source text
    /// line populated (`filename`/`text` attrs — test_flufl's
    /// `cm.exception.filename`/`.text` checks).
    pub fn syntax_error_with_filename(
        msg: impl Into<String>,
        filename: &str,
        source: &str,
    ) -> Self {
        let msg = msg.into();
        // Most parser errors carry an "L<line>:<col>:" prefix; custom
        // validation errors ("unexpected '/' ...") don't. Parse it into real
        // `lineno`/`offset` attributes and produce CPython's `str()` format
        // `msg (<filename>, line N)`; also keep `.msg` clean (no prefix) so
        // `test.support.check_syntax_error`'s assertRaisesRegex(SyntaxError,
        // errtext) matches (test_exceptions' testSyntaxErrorMessage).
        let (clean_msg, line, col) = if let Some(rest) = msg.strip_prefix('L') {
             if let Some((ln, rest)) = rest.split_once(':') {
                 if let Some((col_s, rest)) = rest.split_once(':') {
                     let line = ln.parse::<i64>().ok();
                     let col = col_s.parse::<i64>().ok();
                     (rest.trim_start().to_string(), line, col)
                 } else {
                     (msg.clone(), None, None)
                 }
             } else {
                 (msg.clone(), None, None)
             }
         } else {
             (msg.clone(), None, None)
         };
        // Indentation-related errors are `IndentationError`, a `SyntaxError`
        // subclass in CPython (test_syntax asserts `subclass=IndentationError`).
        let typ = if clean_msg.contains("unexpected indent")
            || clean_msg.contains("unindent does not match")
            || clean_msg.contains("expected an indented block")
        {
            "IndentationError"
        } else {
            "SyntaxError"
        };
        let line = line.unwrap_or(1);
        let col = col.unwrap_or(1);
        let mut extra = std::collections::HashMap::new();
        extra.insert("msg".to_string(), py_str(&clean_msg));
        extra.insert("filename".to_string(), py_str(filename));
        extra.insert("lineno".to_string(), py_int(line));
        extra.insert("offset".to_string(), py_int(col));
        // `text` — the offending source line (real CPython exposes it).
        let text = source.lines().nth((line - 1).max(0) as usize).unwrap_or("");
        extra.insert("text".to_string(), py_str(text));
        // CPython: `str(SyntaxError)` is `msg (filename, line N)` where
        // filename is displayed as basename (e.g. `badsyntax_3131.py`, not
        // the full path) — this matches CPython's actual `SyntaxError`
        // formatting and the test_unicode_identifiers expectation.
        let display_filename = std::path::Path::new(filename)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(filename);
        let display = format!("{} ({}, line {})", clean_msg, display_filename, line);
        PyError::Exception(
            typ.to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: typ.to_string(),
                args: vec![py_str(&display)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: Some(extra),
            }),
        )
    }
    pub fn memory_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception(
            "MemoryError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "MemoryError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
    }
    /// Same `PyError::Exception(name, obj)` pattern as `syntax_error`/
    /// `memory_error` above — `OverflowError` has no dedicated `PyError`
    /// enum variant, so this is how any native function raises it directly
    /// (e.g. `math.pow`'s own genuine-overflow case).
    pub fn overflow_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception(
            "OverflowError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "OverflowError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
    }
    pub fn buffer_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception(
            "BufferError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "BufferError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
    }
    pub fn system_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception(
            "SystemError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "SystemError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
    }
    /// Same `PyError::Exception(name, obj)` pattern as `syntax_error`/
    /// `memory_error`/`overflow_error` above. Real CPython's
    /// `UnboundLocalError` (a `NameError` subclass, see `is_exception_subclass`
    /// in `vm.rs`) is what's actually raised for "local variable referenced
    /// before assignment" — this codebase's `LOAD_FAST` handler used to raise
    /// a plain `PyError::NameError` for this instead, which is wrong both by
    /// message and by exact type (`except UnboundLocalError:` couldn't catch
    /// it, and the name `UnboundLocalError` wasn't even a registered builtin
    /// at all — found via `test_scope.py`'s own `testUnboundLocal`).
    pub fn unbound_local_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception(
            "UnboundLocalError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "UnboundLocalError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
    }
    /// Same `PyError::Exception(name, obj)` pattern as `syntax_error`/
    /// `memory_error`/`overflow_error`/`unbound_local_error` above. Real
    /// CPython raises `ModuleNotFoundError` (a dedicated `ImportError`
    /// subclass, see `is_exception_subclass` in `vm.rs`) specifically for
    /// "module not found" — every "No module named ..." site here used to
    /// raise the plain `PyError::ImportError` variant instead, which reports
    /// as the PARENT class name; `assertRaises(ModuleNotFoundError, ...)`
    /// (e.g. `test_builtin.py::BuiltinTest.test_import`) doesn't accept a
    /// raised parent class even though the subclass relationship holds in
    /// the other direction (same asymmetry already documented for
    /// `binascii.Error`/`ValueError`).
    /// Builds an `OSError` FROM A REAL `std::io::Error`, picking the exact
    /// specific subclass real CPython would raise (`FileNotFoundError`,
    /// `PermissionError`, `FileExistsError`, `IsADirectoryError`,
    /// `NotADirectoryError`, `InterruptedError`) based on `e.kind()`,
    /// instead of the generic `PyError::OsError` every filesystem-touching
    /// call site used to construct unconditionally. Since ALL of these are
    /// real `OSError` subclasses (already registered in `is_exception_
    /// subclass`, `vm.rs`), catching via the specific name is a real,
    /// extremely common Python idiom (`except FileNotFoundError:`,
    /// `Lib/test/support/os_helper.py`'s own `rmtree` helper) that
    /// previously NEVER matched anything this interpreter's own filesystem
    /// functions raised — every such `except` silently let the OSError
    /// propagate uncaught instead of being handled gracefully. Confirmed
    /// via `test_dbm.py::test_whichdb`: `os_helper.rmtree`'s `except
    /// FileNotFoundError: pass` never caught the "directory doesn't exist
    /// yet" case on a fresh run, crashing every single caller.
    pub fn os_error_from_io(e: &std::io::Error) -> Self {
        use std::io::ErrorKind;
        let msg = format!("{}", e);
        let errno = e.raw_os_error();
        let name = match e.kind() {
            ErrorKind::NotFound => "FileNotFoundError",
            ErrorKind::PermissionDenied => "PermissionError",
            ErrorKind::AlreadyExists => "FileExistsError",
            ErrorKind::Interrupted => "InterruptedError",
            _ => {
                // Plain OSError carrying the real errno (test_exceptions'
                // test_errno_ENOTDIR asserts `OSError.errno == errno.ENOTDIR`
                // after os.listdir on a file).
                let mut extra = None;
                if let Some(no) = errno {
                    let mut m = std::collections::HashMap::new();
                    m.insert("errno".to_string(), py_int(no as i64));
                    extra = Some(m);
                }
                return PyError::Exception(
                    "OSError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "OSError".to_string(),
                        args: vec![py_str(&msg), py_str(&msg)],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra,
                    }),
                );
            }
        };
        let mut extra = None;
        if let Some(no) = errno {
            let mut m = std::collections::HashMap::new();
            m.insert("errno".to_string(), py_int(no as i64));
            extra = Some(m);
        }
        PyError::Exception(
            name.to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: name.to_string(),
                args: vec![py_str(&msg), py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra,
            }),
        )
    }

    /// Same pattern as `os_error_from_io` above, for the (rarer) call sites
    /// that construct a "file not found"-shaped message by hand rather than
    /// from a real `std::io::Error` (e.g. an explicit existence check).
    pub fn file_not_found_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception(
            "FileNotFoundError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "FileNotFoundError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
    }

    pub fn module_not_found_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        PyError::Exception(
            "ModuleNotFoundError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "ModuleNotFoundError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
    }
    pub fn runtime_error(msg: impl Into<String>) -> Self {
        PyError::RuntimeError(msg.into())
    }
    pub fn recursion_error(msg: impl Into<String>) -> Self {
        PyError::RecursionError(msg.into())
    }
    pub fn reference_error(msg: impl Into<String>) -> Self {
        PyError::Exception(
            "ReferenceError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "ReferenceError".to_string(),
                args: vec![crate::object::py_str(&msg.into())],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )
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
                    PyObject::Exception { args, .. } => match args.len() {
                        0 => String::new(),
                        1 => args[0].str(),
                        _ => args.iter().map(|a| a.str()).collect::<Vec<_>>().join(", "),
                    },
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
                    PyObject::Instance { dict, .. } => match dict.get("args") {
                        Some(args_obj) => {
                            let is_tuple = matches!(&*args_obj.borrow(), PyObject::Tuple(_));
                            if is_tuple {
                                let items = if let PyObject::Tuple(items) = &*args_obj.borrow() {
                                    items.clone()
                                } else {
                                    unreachable!()
                                };
                                match items.len() {
                                    0 => String::new(),
                                    1 => items[0].str(),
                                    _ => {
                                        items.iter().map(|a| a.str()).collect::<Vec<_>>().join(", ")
                                    }
                                }
                            } else {
                                args_obj.str()
                            }
                        }
                        None => String::new(),
                    },
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
