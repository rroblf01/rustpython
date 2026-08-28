// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds attribute
// access: `get_attribute_impl` (the giant dispatcher backing
// `LOAD_ATTR`/`getattr`/method lookup across every builtin type and
// user-defined class) and its supporting helpers. NOT further broken up
// internally in this pass — see the plan's own note on scope.
use super::*;

mod deque;
mod list;
mod bytes1;
mod bytes2;
mod str1;
mod str2;
mod int;
mod float;
mod compiled_regex;
mod range;
mod tuple;
mod array;
mod frozenset;
mod slice;
mod complex;
mod module_obj;
mod exception_group;
mod generator;
mod set;
mod file;
mod socket;
mod thread;
mod bytearray;
mod dict;
mod super_obj;
mod exception;
mod helpers;
pub use helpers::*;
mod instance;
mod callable;
mod sync;

thread_local! {
    // PEP 649 computed-annotation cache, keyed by each function's
    // `__annotate__` closure identity (see the `__annotations__` arm).
    static ANN_CACHE: std::cell::RefCell<std::collections::HashMap<usize, PyObjectRef>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

// ---- Attribute access ----


pub trait ObjectAccess {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef>;
    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()>;
    fn del_attribute(&mut self, name: &str) -> PyResult<()>;
}

impl PyObject {
    /// Every real Python object has `__doc__` (defaulting to `None` if not
    /// otherwise set — `bool`/`int`/etc. all inherit it from `object`).
    /// The per-variant match below (a few thousand lines, one arm per
    /// builtin type, each with its own "no such attribute" catch-all) has
    /// no single place to add a universal fallback without touching every
    /// arm — so it stays untouched as `get_attribute_impl`, and the real
    /// `get_attribute` (the trait method below) just catches this one
    /// specific case on error instead. Real code doing generic attribute
    /// introspection over arbitrary values (e.g. something in the stdlib
    /// `email`/`dataclasses` machinery checking `.__doc__` while walking a
    /// structure that isn't guaranteed to be a function/class) hit this.
    pub(crate) fn get_attribute_impl(&self, name: &str) -> PyResult<PyObjectRef> {
        // `.__class__` (equivalent to `type(x)`) universally, for every
        // variant — this was entirely missing from `get_attribute_impl`
        // (used by the `getattr()` builtin and any other generic
        // attribute-access call site), even for a plain `class Foo: ...`
        // instance, even though `x.__class__` (direct dot-syntax) already
        // worked via a separate, hardcoded special case in `vm.rs`'s
        // LOAD_ATTR opcode handler. So `getattr(x, "__class__")` — a common
        // proxy/introspection idiom real code uses interchangeably with
        // `type(x)` — raised `AttributeError` for literally every object,
        // real trigger: CPython 3.14's own `unittest/case.py`
        // (`self.__class__` reached via a code path that goes through
        // `get_attribute_impl` rather than LOAD_ATTR). Mirrors
        // `builtin_type_of`'s own logic (Instance → its real type;
        // Type → itself; anything else → a freshly-built placeholder Type
        // sharing just the name, same as `type(x)` already does for
        // natives).
        // Per-instance attributes stored on `functools.partial` objects
        // (CPython's partial has a real __dict__; configparser assigns
        // `self.converter = ...` on one).
        if !name.starts_with("__") {
            if let PyObject::Partial { dict, .. } = self {
                if let Some(v) = dict.get(name) {
                    return Ok(v.clone());
                }
            }
        }
        if name == "__class__" {
            match self {
                PyObject::Instance { typ, .. } => return Ok(typ.clone()),
                // A class's own `__class__` is its metaclass — usually
                // plain `type`. `metatype_of()` (used elsewhere for the
                // real, `METATYPE_KEY`-tracked custom-metaclass case) needs
                // a `PyObjectRef`, not the bare `&PyObject` available here;
                // falling back to plain `"type"` is correct for the
                // overwhelmingly common no-custom-metaclass case.
                PyObject::Type { dict, .. } if dict.contains_key_str(METATYPE_KEY) => {
                    return Ok(dict.get_str(METATYPE_KEY).unwrap().clone());
                }
                PyObject::Type { .. } => {
                    return Ok(PyObjectRef::new(PyObject::Type {
                        name: "type".to_string(),
                        dict: Box::new(TypeDict::default()),
                        bases: vec![],
                        mro: vec![],
                    }));
                }
                _ => {
                    return Ok(PyObjectRef::new(PyObject::Type {
                        name: self.type_name().to_string(),
                        dict: Box::new(TypeDict::default()),
                        bases: vec![],
                        mro: vec![],
                    }));
                }
            }
        }
        // `.__dir__` — `dir()` itself (`builtin_dir`) already introspects
        // every variant directly and doesn't need this, but `dir()`'s own
        // listing always advertises a synthetic `"__dir__"` name (matching
        // real CPython, where every object inherits `object.__dir__`), and
        // code that walks that listing generically (`getattr(obj, name) for
        // name in dir(obj)` — real trigger: CPython 3.14's own
        // `unittest/loader.py`'s `loadTestsFromModule`) then does
        // `getattr(module, "__dir__")`, which raised `AttributeError` since
        // no variant actually exposed it as a real bindable attribute.
        // Doesn't check for a user-overridden `__dir__` first (unlike a
        // real per-type dict lookup) — a rare enough case in practice that
        // matching the `.__class__` fix's pragmatic same-shape precedent
        // (a universal fallback) is the right tradeoff here.
        if name == "__dir__" {
            return Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                name: "__dir__".to_string(),
                func: builtin_dir,
                self_obj: py_none(),
            }));
        }
        match self {
            PyObject::Complex(_, _) => return complex::get(self, name),
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    let t = PyObjectRef::Imm(rc);
                    return t.borrow().get_attribute(name);
                } else {
                    return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                }
            }
            PyObject::Module { .. } => return module_obj::get(self, name),
            PyObject::Type { .. } | PyObject::Instance { .. } | PyObject::Property(_) | PyObject::StaticMethod { .. } | PyObject::ClassMethod { .. } => return instance::get(self, name),            PyObject::Exception { .. } => return exception::get(self, name),
            // `ExceptionGroup`/`BaseExceptionGroup` (PEP 654) had NO
            // attribute access implemented at all — not even the two core
            // PEP 654 fields (`.message`, `.exceptions`), let alone the
            // same PEP 3134 chaining/traceback attributes `Exception`
            // itself already supports just above. Real trigger: CPython's
            // own `test_exception_group.py` — even the most basic
            // `ExceptionGroup("msg", [...]).message` raised `AttributeError`.
            PyObject::ExceptionGroup { .. } => return exception_group::get(self, name),
            PyObject::List(_v) => return list::get(self, name),
            PyObject::Deque { data, maxlen } => return deque::get(self, name),
            PyObject::Tuple(_v) => return tuple::get(self, name),
            PyObject::Bytes(_v) => {
                match bytes1::get(self, name) {
                    Ok(v) => return Ok(v),
                    Err(_) => return bytes2::get(self, name),
                }
            }
            PyObject::ByteArray(_b) => return bytearray::get(self, name),
            PyObject::Str(_s) => {
                match str1::get(self, name) {
                    Ok(v) => return Ok(v),
                    Err(_) => return str2::get(self, name),
                }
            }
            // dict-protocol methods on the live `globals()` view — same
            // surface as `dict` below, but operating on the backing
            // `Rc<RefCell<HashMap<StrId, PyObjectRef>>>` so mutators
            // (`update`/`setdefault`/`pop`/`clear`) stay visible to
            // LOAD_GLOBAL.
            PyObject::Globals(_) => return sync::get(self, name),            PyObject::Dict(_d) => return dict::get(self, name),
            PyObject::Set(_s) => return set::get(self, name),
            PyObject::Function(_) => return callable::get(self, name),            PyObject::BoundMethod { .. } => return callable::get(self, name),            PyObject::Generator { .. } => return generator::get(self, name),
            PyObject::Coroutine { frame: _coro_frame } => match name {
                "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "send".to_string(),
                    func: coroutine_send_fallback,
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "throw" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "throw".to_string(),
                    func: coroutine_throw_fallback,
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "close".to_string(),
                    func: |args| {
                        let gen = args[0].borrow();
                        if let PyObject::Coroutine { frame } = &*gen {
                            if let Ok(mut frame_opt) = frame.try_borrow_mut() {
                                *frame_opt = None;
                            }
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__await__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__await__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__iter__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__await__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__await__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__anext__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__anext__".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { .. } = &*args[0].borrow() {
                            let send_method = args[0].borrow().get_attribute("send")?;
                            let (n, f) = {
                                let b = send_method.borrow();
                                if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                    (name.clone(), *func)
                                } else {
                                    return Err(PyError::runtime_error("expected send method"));
                                }
                            };
                            let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: n,
                                func: f,
                                self_obj: args[0].clone(),
                            });
                            let mut vm = crate::vm::VirtualMachine::new();
                            vm.call_function(fixed, vec![crate::object::py_none()], vec![])
                        } else {
                            Err(PyError::runtime_error("__anext__ on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__aiter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__aiter__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "asend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "asend".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { .. } = &*args[0].borrow() {
                            let send_method = args[0].borrow().get_attribute("send")?;
                            let (n, f) = {
                                let b = send_method.borrow();
                                if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                    (name.clone(), *func)
                                } else {
                                    return Err(PyError::runtime_error("expected send method"));
                                }
                            };
                            let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: n,
                                func: f,
                                self_obj: args[0].clone(),
                            });
                            let val = if args.len() > 1 {
                                args[1].clone()
                            } else {
                                crate::object::py_none()
                            };
                            let mut vm = crate::vm::VirtualMachine::new();
                            vm.call_function(fixed, vec![val], vec![])
                        } else {
                            Err(PyError::runtime_error("asend on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "athrow" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "athrow".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { .. } = &*args[0].borrow() {
                            let throw_method = args[0].borrow().get_attribute("throw")?;
                            let (n, f) = {
                                let b = throw_method.borrow();
                                if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                    (name.clone(), *func)
                                } else {
                                    return Err(PyError::runtime_error("expected throw method"));
                                }
                            };
                            let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: n,
                                func: f,
                                self_obj: args[0].clone(),
                            });
                            let exc = if args.len() > 1 {
                                args[1].clone()
                            } else {
                                crate::object::py_none()
                            };
                            let mut vm = crate::vm::VirtualMachine::new();
                            vm.call_function(fixed, vec![exc], vec![])
                        } else {
                            Err(PyError::runtime_error("athrow on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "aclose" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "aclose".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { frame } = &*args[0].borrow() {
                            let mut frame_opt = frame.borrow_mut();
                            *frame_opt = None;
                            Ok(crate::object::py_none())
                        } else {
                            Err(PyError::runtime_error("aclose on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'coroutine' object has no attribute '{}'",
                    name
                ))),
            },
            PyObject::Process {
                child,
                pid,
                returncode,
                stdin_pipe,
                stdout_pipe,
                stderr_pipe,
            } => {
                match name {
                    "pid" => Ok(py_int(*pid)),
                    "returncode" => Ok(returncode.borrow().map(py_int).unwrap_or_else(py_none)),
                    // `Popen.stdout`/`stdin`/`stderr` — real CPython exposes
                    // the pipe file objects here (test_quopri's cleanup
                    // closes them; test_cmd_line_script's interactive_python
                    // WRITES to stdin and READS the prompt back from the
                    // output pipes). Returns a File wrapping the actual pipe
                    // captured at Popen construction.
                    "stdout" | "stderr" | "stdin" => {
                        let pipe = match name {
                            "stdout" => stdout_pipe.as_ref(),
                            "stderr" => stderr_pipe.as_ref(),
                            _ => stdin_pipe.as_ref(),
                        };
                        if let Some(p) = pipe {
                            Ok(PyObjectRef::new(PyObject::File {
                                file: p.clone(),
                                name: "<pipe>".to_string(),
                                binary: true,
                                pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                                closed: false,
                            }))
                        } else if let Ok(f) =
                            std::fs::OpenOptions::new().read(true).open("/dev/null")
                        {
                            Ok(PyObjectRef::new(PyObject::File {
                                file: std::rc::Rc::new(std::cell::RefCell::new(f)),
                                name: "<pipe>".to_string(),
                                binary: true,
                                pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                                closed: false,
                            }))
                        } else {
                            Err(PyError::runtime_error(
                                "cannot open /dev/null for Popen pipe",
                            ))
                        }
                    }
                    "poll" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "poll".to_string(),
                        func: |args| {
                            if let PyObject::Process {
                                child, returncode, ..
                            } = &*args[0].borrow()
                            {
                                if let Some(rc) = *returncode.borrow() {
                                    return Ok(py_int(rc));
                                }
                                let mut child_opt = child.borrow_mut();
                                match child_opt.as_mut() {
                                    Some(c) => match c.try_wait() {
                                        Ok(Some(status)) => {
                                            let rc = status.code().unwrap_or(-1) as i64;
                                            *returncode.borrow_mut() = Some(rc);
                                            Ok(py_int(rc))
                                        }
                                        Ok(None) => Ok(py_none()),
                                        Err(e) => Err(PyError::os_error_from_io(&e)),
                                    },
                                    None => Ok(py_none()),
                                }
                            } else {
                                Err(PyError::runtime_error("poll on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "wait" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "wait".to_string(),
                        func: |args| {
                            if let PyObject::Process {
                                child, returncode, ..
                            } = &*args[0].borrow()
                            {
                                if let Some(rc) = *returncode.borrow() {
                                    return Ok(py_int(rc));
                                }
                                let mut child_opt = child.borrow_mut();
                                match child_opt.as_mut() {
                                    Some(c) => match c.wait() {
                                        Ok(status) => {
                                            let rc = status.code().unwrap_or(-1) as i64;
                                            *returncode.borrow_mut() = Some(rc);
                                            Ok(py_int(rc))
                                        }
                                        Err(e) => Err(PyError::os_error_from_io(&e)),
                                    },
                                    None => Ok(py_none()),
                                }
                            } else {
                                Err(PyError::runtime_error("wait on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `communicate(input=None, timeout=None)` — writes
                    // `input` (if given and stdin was piped) then reads
                    // stdout/stderr to completion via `Child::
                    // wait_with_output` (which internally spawns reader
                    // threads for both streams concurrently, avoiding the
                    // classic "write blocks because the child's stdout
                    // pipe filled up while nobody's reading it yet"
                    // deadlock). Consumes the stored `Child` — a second
                    // `communicate()` call after the first sees `None` and
                    // returns empty output, matching real Python's own
                    // "communicate() should only be called once" contract
                    // closely enough for real-world usage.
                    "communicate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "communicate".to_string(),
                        func: |args| {
                            // Clone the Process internals out first so we can
                            // mutate the Process (closing stdin) without
                            // holding a borrow on it.
                            let (child, returncode, stdin_pipe, stdout_pipe, stderr_pipe) = {
                                match &*args[0].borrow() {
                                    PyObject::Process {
                                        child,
                                        returncode,
                                        stdin_pipe,
                                        stdout_pipe,
                                        stderr_pipe,
                                        ..
                                    } => (
                                        child.clone(),
                                        returncode.clone(),
                                        stdin_pipe.clone(),
                                        stdout_pipe.clone(),
                                        stderr_pipe.clone(),
                                    ),
                                    _ => {
                                        return Err(PyError::runtime_error(
                                            "communicate on non-process",
                                        ))
                                    }
                                }
                            };
                            let input = args
                                .get(1)
                                .filter(|v| !matches!(&*v.borrow(), PyObject::None));
                            // Write the input to the child's stdin pipe.
                            if let (Some(inp), Some(stdin)) = (input, &stdin_pipe) {
                                use std::io::Write;
                                let bytes = match &*inp.borrow() {
                                    PyObject::Bytes(b) => b.clone(),
                                    other => other.str().into_bytes(),
                                };
                                let _ = stdin.borrow_mut().write_all(&bytes);
                            }
                            // CLOSE stdin so the child sees EOF and can
                            // finish (a child reading stdin blocks until the
                            // write end closes — not closing here deadlocked
                            // communicate() against -mquopri, which reads all
                            // of stdin before producing output). The Process's
                            // slot AND our own cloned handle must both drop.
                            if stdin_pipe.is_some() {
                                if let PyObject::Process { stdin_pipe: sp, .. } =
                                    &mut *args[0].borrow_mut()
                                {
                                    *sp = None;
                                }
                            }
                            drop(stdin_pipe);
                            // Read stdout + stderr pipes to EOF.
                            use std::io::Read;
                            let read_all =
                                |p: &std::rc::Rc<std::cell::RefCell<std::fs::File>>| -> Vec<u8> {
                                    let mut buf = Vec::new();
                                    let _ = p.borrow_mut().read_to_end(&mut buf);
                                    buf
                                };
                            let stdout = stdout_pipe.as_ref().map(read_all).unwrap_or_default();
                            let stderr = stderr_pipe.as_ref().map(read_all).unwrap_or_default();
                            // Reap the child for its returncode.
                            let taken = child.borrow_mut().take();
                            let rc = match taken {
                                Some(mut c) => match c.wait() {
                                    Ok(status) => status.code().unwrap_or(-1) as i64,
                                    Err(_) => -1,
                                },
                                None => returncode.borrow().unwrap_or(-1),
                            };
                            *returncode.borrow_mut() = Some(rc);
                            Ok(py_tuple(vec![
                                PyObjectRef::imm(PyObject::Bytes(stdout)),
                                PyObjectRef::imm(PyObject::Bytes(stderr)),
                            ]))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Rust's `std::process::Child` doesn't distinguish a
                    // graceful SIGTERM from a hard SIGKILL the way real
                    // `Popen.terminate()`/`.kill()` do (POSIX-specific) —
                    // both map to `Child::kill()` here, good enough for the
                    // overwhelming majority of real usage (which just wants
                    // "make the child stop").
                    "terminate" | "kill" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args| {
                            if let PyObject::Process { child, .. } = &*args[0].borrow() {
                                if let Some(c) = child.borrow_mut().as_mut() {
                                    let _ = c.kill();
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("terminate/kill on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "send_signal" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send_signal".to_string(),
                        func: |args| {
                            // No portable "send arbitrary signal" in std;
                            // treat any signal as a kill request (correct
                            // for the extremely common SIGTERM/SIGKILL
                            // case, not for exotic signal numbers).
                            if let PyObject::Process { child, .. } = &*args[0].borrow() {
                                if let Some(c) = child.borrow_mut().as_mut() {
                                    let _ = c.kill();
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("send_signal on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            if let PyObject::Process {
                                child, returncode, ..
                            } = &*args[0].borrow()
                            {
                                if returncode.borrow().is_none() {
                                    if let Some(c) = child.borrow_mut().as_mut() {
                                        if let Ok(status) = c.wait() {
                                            *returncode.borrow_mut() =
                                                Some(status.code().unwrap_or(-1) as i64);
                                        }
                                    }
                                }
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'Popen' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::File { file: f_rc, .. } => return file::get(self, name),
            // `array.array` had NO attributes/methods dispatched at all —
            // even the basics (`.itemsize`, `.typecode`, `.tobytes()`,
            // `.tolist()`) were missing, blocking any real usage beyond
            // construction/indexing. Found via `test_memoryview.py`'s own
            // `BaseArrayMemoryTests`, whose class body reads `array.array
            // ('i').itemsize` — a collection-time crash for the WHOLE file
            // otherwise.
            PyObject::Array(arr) => return array::get(self, name),
            PyObject::MemoryView { .. } => {
                let self_ref = PyObjectRef::new(self.clone());
                if let Some(result) = mv_getprop(&self_ref, name) {
                    return result;
                }
                mv_getattr(name).ok_or_else(|| {
                    PyError::attribute_error(format!(
                        "'memoryview' object has no attribute '{}'",
                        name
                    ))
                })
            }
            PyObject::Socket { inner: _ } => return socket::get(self, name),
            PyObject::Thread(inner_arc) => return thread::get(self, name),
            PyObject::Lock(_) | PyObject::RLock(_) | PyObject::Event(_) | PyObject::Queue(_) => return sync::get(self, name),            PyObject::Int(_) | PyObject::Bool(_) => return int::get(self, name),
            PyObject::Float(_) => return float::get(self, name),
            PyObject::Range { .. } | PyObject::RangeIter { .. } => return range::get(self, name),
            PyObject::CompiledRegex { .. } => return compiled_regex::get(self, name),
            PyObject::Super { cls, obj } => return super_obj::get(self, name),
            PyObject::FutureAwaitIterator {
                future: _,
                yielded: _,
            } => {
                match name {
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__next__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__next__".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error("__next__ needs self"));
                            }
                            let self_ref = args[0].borrow();
                            let (done, result) = match &*self_ref {
                                PyObject::FutureAwaitIterator { future, yielded } => {
                                    if *yielded {
                                        let done = future
                                            .borrow()
                                            .get_attribute("_done")
                                            .ok()
                                            .map(|d| d.truthy())
                                            .unwrap_or(false);
                                        let result = future
                                            .borrow()
                                            .get_attribute("_result")
                                            .unwrap_or_else(|_| py_none());
                                        (Some(done), Some(result))
                                    } else {
                                        let f = future.clone();
                                        drop(self_ref);
                                        return Ok(f);
                                    }
                                }
                                _ => {
                                    return Err(PyError::runtime_error(
                                        "__next__ on non-FutureAwaitIterator",
                                    ))
                                }
                            };
                            drop(self_ref);
                            if let Some(true) = done {
                                Err(PyError::Exception(
                                    "StopIteration".to_string(),
                                    result.unwrap_or_else(|| py_none()),
                                ))
                            } else {
                                Ok(py_none())
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error("send needs self"));
                            }
                            let (is_first, future_clone) = match &*args[0].borrow() {
                                PyObject::FutureAwaitIterator { future, yielded } => {
                                    (!*yielded, future.clone())
                                }
                                _ => {
                                    return Err(PyError::runtime_error(
                                        "send on non-FutureAwaitIterator",
                                    ))
                                }
                            };
                            if is_first {
                                let mut obj = args[0].borrow_mut();
                                if let PyObject::FutureAwaitIterator { yielded, .. } = &mut *obj {
                                    *yielded = true;
                                }
                                drop(obj);
                                // Return the future as the yielded value
                                Ok(future_clone)
                            } else {
                                // Second send: check if future is done
                                let done = future_clone
                                    .borrow()
                                    .get_attribute("_done")
                                    .ok()
                                    .map(|d| d.truthy())
                                    .unwrap_or(false);
                                let result = future_clone
                                    .borrow()
                                    .get_attribute("_result")
                                    .unwrap_or_else(|_| py_none());
                                if done {
                                    Err(PyError::Exception("StopIteration".to_string(), result))
                                } else {
                                    Ok(future_clone)
                                }
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'future_await_iterator' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::BuiltinFunction { .. } => return callable::get(self, name),            PyObject::FrozenSet(_items) => return frozenset::get(self, name),
            PyObject::Slice { .. } => return slice::get(self, name),
            PyObject::Code(_) => return callable::get(self, name),            PyObject::BuiltinMethod { .. } => return callable::get(self, name),            PyObject::ListIter { .. }
            | PyObject::MapIterator { .. }
            | PyObject::FilterIterator { .. }
            | PyObject::ZipIterator { .. }
            | PyObject::CycleIter { .. }
            | PyObject::GroupByIter { .. }
            | PyObject::EnumerateIter { .. }
            | PyObject::GetItemIter { .. }
            | PyObject::CallSentinelIter { .. }
                if name == "__next__" || name == "__iter__" =>
            {
                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: name.to_string(),
                    func: if name == "__next__" {
                        builtin_next
                    } else {
                        builtin_iter
                    },
                    self_obj: PyObjectRef::new(self.clone()),
                }))
            }
            _ => Err(PyError::attribute_error(format!(
                "'{}' object has no attribute '{}'",
                self.type_name(),
                name
            ))),
        }
    }
}

impl ObjectAccess for PyObject {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef> {
        match self.get_attribute_impl(name) {
            Err(_) if name == "__doc__" => Ok(py_none()),
            Err(e) if matches!(e, PyError::AttributeError(_)) => {
                // Attach the attribute NAME and owning OBJECT to the raised
                // AttributeError (CPython: `exc.name`/`exc.obj`). This is
                // what `except AttributeError as exc: exc.name` sees after
                // `obj.missing_attr`. The reconstructed PyObjectRef for
                // `self` differs in identity from the original Rc, so
                // `exc.obj is obj` may be False, but value equality and the
                // overwhelmingly common `exc.name` checks work.
                let mut extra = std::collections::HashMap::new();
                extra.insert("name".to_string(), py_str(name));
                extra.insert("obj".to_string(), PyObjectRef::new(self.clone()));
                Err(PyError::Exception(
                    "AttributeError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "AttributeError".to_string(),
                        args: vec![py_str(&format!(
                            "'{}' object has no attribute '{}'",
                            self.type_name(),
                            name
                        ))],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: Some(extra),
                    }),
                ))
            }
            other => other,
        }
    }

    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, typ } => {
                if crate::object::get_type_name_for_instance(typ) == "Dialect" {
                    return Err(PyError::AttributeError("attribute is read-only".to_string()));
                }
                // Check __slots__ restriction if defined on the type or its MRO
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        let type_name = get_type_name_for_instance(typ);
                        return Err(PyError::attribute_error(format!(
                            "'{}' object has no attribute '{}'",
                            type_name, name
                        )));
                    }
                }
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Module { dict, name: mod_name } => {
                dict.insert_str(&name, value.clone());
                // Keep `frame.globals` (the Rc captured by functions
                // defined in this module) in sync with `module.__dict__`
                // when `setattr(module, name, value)` is used (e.g.
                // `mock.patch.object(script_helper, 'interpreter_requires_environment',
                // return_value=True)`). `LOAD_GLOBAL` inside
                // `run_python_until_end` reads from `frame.globals`, not
                // `module.dict`, so without this the mock is invisible.
                crate::object::pydict::update_module_globals(mod_name, name, value.clone());
                Ok(())
            }
            PyObject::Type { dict, .. } => {
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Function(ref mut f) => {
                f.dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Dict(_)
            | PyObject::List(_)
            | PyObject::Tuple(_)
            | PyObject::Set(_)
            | PyObject::FrozenSet(_) => {
                // Store attributes in a side dict (instance-like) for these built-in types
                let _pd = match self {
                    PyObject::Dict(d) => Some(d.clone()),
                    _ => None,
                };
                Err(PyError::attribute_error(format!(
                    "cannot set attribute '{}' on '{}'",
                    name,
                    self.type_name()
                )))
            }
            PyObject::Exception {
                cause,
                suppress_context,
                ..
            } if name == "__cause__" => {
                *cause = Some(value);
                Ok(())
            }
            PyObject::Exception { context, .. } if name == "__context__" => {
                *context = Some(value);
                Ok(())
            }
            PyObject::Exception { traceback, .. } if name == "__traceback__" => {
                *traceback = Some(value);
                Ok(())
            }
            PyObject::Exception {
                suppress_context, ..
            } if name == "__suppress_context__" => {
                let b = value.borrow();
                *suppress_context = matches!(&*b, PyObject::Bool(true));
                Ok(())
            }
            PyObject::Exception { extra, .. } => {
                // Store arbitrary per-instance attributes (BaseException
                // `__dict__` semantics): `e.name = ...`, `e.obj = ...`, etc.
                // This also backs the AttributeError name/obj set by the
                // getattr machinery. `__traceback__`/`__context__` etc. are
                // handled by dedicated arms above.
                let extra = extra.get_or_insert_with(|| std::collections::HashMap::new());
                extra.insert(name.to_string(), value);
                Ok(())
            }
            PyObject::Partial { dict, .. } => {
                dict.insert_str(name, value);
                Ok(())
            }
            PyObject::ExceptionGroup { .. } => {
                // No backing dict on these variants for __traceback__,
                // __context__, __suppress_context__, __notes__, or custom
                // attributes — but `except E as e: e.__traceback__ = tb` (and
                // similar) is an extremely common idiom (contextlib,
                // unittest, ...) that must not hard-crash just because we
                // don't track those fields anywhere.
                Ok(())
            }
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    let t = PyObjectRef::Imm(rc);
                    return t.borrow_mut().set_attribute(name, value);
                } else {
                    return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                }
            }
            _ => Err(PyError::attribute_error(format!(
                "cannot set attribute '{}' on '{}'",
                name,
                self.type_name()
            ))),
        }
    }

    fn del_attribute(&mut self, name: &str) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, typ } => {
                if crate::object::get_type_name_for_instance(typ) == "Dialect" {
                    return Err(PyError::AttributeError("attribute is read-only".to_string()));
                }
                // Check __slots__ restriction if defined on the type or its MRO
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        let type_name = get_type_name_for_instance(typ);
                        return Err(PyError::attribute_error(format!(
                            "'{}' object has no attribute '{}'",
                            type_name, name
                        )));
                    }
                }
                dict.remove(name).ok_or_else(|| {
                    PyError::attribute_error(format!(
                        "'{}' object has no attribute '{}'",
                        self.type_name(),
                        name
                    ))
                })?;
                Ok(())
            }
            PyObject::Module { dict, .. } => {
                dict.remove(&interner::intern(name)).ok_or_else(|| {
                    PyError::attribute_error(format!("module has no attribute '{}'", name))
                })?;
                Ok(())
            }
            PyObject::Type { dict, .. } => {
                dict.remove(&interner::intern(name)).ok_or_else(|| {
                    PyError::attribute_error(format!("type has no attribute '{}'", name))
                })?;
                Ok(())
            }
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    let t = PyObjectRef::Imm(rc);
                    return t.borrow_mut().del_attribute(name);
                } else {
                    return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                }
            }
            _ => Err(PyError::attribute_error(format!(
                "'{}' object has no attribute '{}'",
                self.type_name(),
                name
            ))),
        }
    }
}
