use crate::object::{is_builtin_exception_class_name, PyError, PyObject, PyObjectRef, PyResult};

/// Resolves an `except` clause's type expression against a raised
/// exception's type name — handling the common `except (A, B):` tuple form
/// (matches if ANY member matches), not just a single bare type/name.
pub(crate) fn exc_type_matches(expected: &PyObjectRef, exc_type_name: &str) -> PyResult<bool> {
    match &*expected.borrow() {
        PyObject::Str(s) if is_builtin_exception_class_name(s) => {
            Ok(is_exception_subclass(exc_type_name, s))
        }
        PyObject::Type { name, bases, .. } => {
            if !bases.is_empty() && crate::object::find_exception_base_name(expected).is_none() {
                return Err(PyError::type_error(
                    "catching classes that do not inherit from BaseException is not allowed",
                ));
            }
            Ok(is_exception_subclass(exc_type_name, name))
        }
        PyObject::BuiltinFunction { name, .. } => Ok(is_exception_subclass(exc_type_name, name)),
        PyObject::Tuple(items) | PyObject::List(items) => {
            for item in items {
                if exc_type_matches(item, exc_type_name)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Err(PyError::type_error(
            "catching classes that do not inherit from BaseException is not allowed",
        )),
    }
}

// ── Opcode histogram (RPY_OPCODE_HIST=1) ────────────────────────────
// Profiling aid: per-opcode execution counts, dumped to stderr at exit.
pub(crate) static OPCODE_HIST_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
pub(crate) static OPCODE_HIST: [std::sync::atomic::AtomicU64; 256] = {
    #[allow(clippy::declare_interior_mutable_const)]
    const ZERO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    [ZERO; 256]
};

pub(crate) fn set_sys_modules_priority(on: bool) {
    crate::vm::SYS_MODULES_PRIORITY.with(|c| c.set(on));
}

pub(crate) fn get_shared_builtins_module() -> PyObjectRef {
    crate::vm::SHARED_BUILTINS_MODULE_REF.with(|c| {
        c.borrow().clone().expect("shared builtins module not initialized")
    })
}

pub(crate) fn opcode_hist_init_from_env() {
    if std::env::var("RPY_OPCODE_HIST").is_ok() {
        OPCODE_HIST_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(crate) fn opcode_hist_dump() {
    if !OPCODE_HIST_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut rows: Vec<(u64, u64)> = OPCODE_HIST
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u64, c.load(std::sync::atomic::Ordering::Relaxed)))
        .filter(|&(_, n)| n > 0)
        .collect();
    let total: u64 = rows.iter().map(|r| r.1).sum();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("=== OPCODE HISTOGRAM (total {total}) ===");
    for (op, n) in rows.iter().take(25) {
        let name = crate::bytecode::Opcode::from_u16(*op as u16)
            .map(|o| format!("{o:?}"))
            .unwrap_or_else(|| format!("OP_{op}"));
        eprintln!("{:>10}  {:>5.1}%  {}", n, 100.0 * *n as f64 / total as f64, name);
    }
}

pub(crate) fn is_exception_subclass(child_type: &str, parent_type: &str) -> bool {
    if child_type == parent_type {
        return true;
    }
    if child_type == "UnsupportedOperation" {
        return parent_type == "UnsupportedOperation"
            || is_exception_subclass("OSError", parent_type)
            || is_exception_subclass("ValueError", parent_type);
    }
    let parent: Option<&str> = match child_type {
        "BaseException" => None,
        "Exception" | "SystemExit" | "KeyboardInterrupt" | "GeneratorExit"
        | "BaseExceptionGroup" => Some("BaseException"),
        "ArithmeticError" | "LookupError" | "ImportError" | "RuntimeError" | "Warning"
        | "OSError" | "ValueError" => Some("Exception"),
        "CycleError" => Some("ValueError"),
        "DecimalException" => Some("ArithmeticError"),
        "InvalidOperation" | "DivisionByZero" | "Inexact" | "Rounded" | "Clamped" | "Overflow"
        | "Underflow" | "FloatOperation" => Some("DecimalException"),
        "PickleError" => Some("Exception"),
        "PicklingError" | "UnpicklingError" => Some("PickleError"),
        "ExceptionGroup" => Some("Exception"),
        "FloatingPointError" | "OverflowError" | "ZeroDivisionError" => Some("ArithmeticError"),
        "IndexError" | "KeyError" => Some("LookupError"),
        "EnvironmentError" | "IOError" => Some("OSError"),
        "FileNotFoundError" | "PermissionError" | "NotADirectoryError" | "IsADirectoryError"
        | "FileExistsError" => Some("OSError"),
        "ConnectionError" => Some("OSError"),
        "BrokenPipeError"
        | "ConnectionAbortedError"
        | "ConnectionRefusedError"
        | "ConnectionResetError" => Some("ConnectionError"),
        "BlockingIOError" | "ChildProcessError" | "InterruptedError" | "ProcessLookupError"
        | "TimeoutError" => Some("OSError"),
        "NotImplementedError" | "RecursionError" | "PythonFinalizationError" => {
            Some("RuntimeError")
        }
        "ModuleNotFoundError" => Some("ImportError"),
        "UnboundLocalError" => Some("NameError"),
        "IndentationError" => Some("SyntaxError"),
        "TabError" => Some("IndentationError"),
        "_IncompleteInputError" => Some("SyntaxError"),
        "UnicodeError" => Some("ValueError"),
        "UnicodeEncodeError" | "UnicodeDecodeError" | "UnicodeTranslateError" => {
            Some("UnicodeError")
        }
        "Error" => Some("ValueError"),
        "UserWarning"
        | "DeprecationWarning"
        | "PendingDeprecationWarning"
        | "EncodingWarning"
        | "SyntaxWarning"
        | "RuntimeWarning"
        | "FutureWarning"
        | "ImportWarning"
        | "UnicodeWarning"
        | "BytesWarning"
        | "ResourceWarning" => Some("Warning"),
        "TypeError" | "NameError" | "AttributeError" | "StopIteration" | "StopAsyncIteration"
        | "AssertionError" | "BufferError" | "EOFError" | "MatchError" | "ReferenceError"
        | "MemoryError" => Some("Exception"),
        _ => Some("Exception"),
    };
    match parent {
        Some(p) => {
            if p == parent_type {
                true
            } else {
                is_exception_subclass(p, parent_type)
            }
        }
        None => false,
    }
}
