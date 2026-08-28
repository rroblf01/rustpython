// Split from src/object/exceptions_ctor.rs — misc helpers (call_function, random, socket, thread/array/sync types, create_module).
use super::*;
use crate::object::*;
use std::collections::HashMap;

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
    Err(PyError::type_error(format!(
        "'{}' object is not callable",
        func.borrow().type_name()
    )))
}

static RNG_STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn fast_random_u64() -> u64 {
    RNG_STATE
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

pub(crate) fn socket_addr_to_string(addr: &PyObjectRef) -> PyResult<String> {
    let borrowed = addr.borrow();
    match &*borrowed {
        PyObject::Tuple(items) if items.len() == 2 => {
            let host = items[0].str();
            let port = items[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("port must be int"))?;
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
    py_tuple(vec![
        py_str(&addr.ip().to_string()),
        py_int(addr.port() as i64),
    ])
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
