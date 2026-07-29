// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds `PyFunction`, the
// core `PyObject` enum definition, `PropertyData`, `SocketInner`, and
// `PyObject`'s basic accessor methods (`type_name`/`repr`/`str`/`truthy`/
// `hash`/`equals`).
use super::*;

/// Boxed function data — separates Function's large payload from the
/// `PyObject` enum so the enum itself stays small (176 -> 8 bytes in enum).
pub struct PyFunction {
    pub code: Rc<CodeObject>,
    pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    pub defaults: Vec<PyObjectRef>,
    pub closure: Vec<PyObjectRef>,
    pub dict: HashMap<String, PyObjectRef>,
    pub jit_ptr: std::cell::Cell<usize>,
    pub jit_consts: std::cell::RefCell<Vec<PyObjectRef>>,
}

impl Clone for PyFunction {
    fn clone(&self) -> Self {
        PyFunction {
            code: self.code.clone(),
            globals: self.globals.clone(),
            defaults: self.defaults.clone(),
            closure: self.closure.clone(),
            dict: self.dict.clone(),
            jit_ptr: std::cell::Cell::new(self.jit_ptr.get()),
            jit_consts: std::cell::RefCell::new(self.jit_consts.borrow().clone()),
        }
    }
}

#[derive(Clone)]
pub enum PyObject {
    None,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Complex(f64, f64),
    Str(compact_str::CompactString),
    Bytes(Vec<u8>),
    ByteArray(Vec<u8>),
    List(Vec<PyObjectRef>),
    Tuple(Vec<PyObjectRef>),
    Dict(Box<PyDict>),
    Set(PySet),
    FrozenSet(PySet),
    Range {
        start: i64,
        stop: i64,
        step: i64,
    },
    RangeIter {
        current: i64,
        stop: i64,
        step: i64,
    },
    ListIter {
        list: Vec<PyObjectRef>,
        index: usize,
    },
    /// Backing for the "old-style sequence iteration" fallback: real Python
    /// makes ANY object with `__getitem__` but no `__iter__` iterable by
    /// calling `obj[0]`, `obj[1]`, ... until `IndexError` (converted to
    /// `StopIteration`) — this was entirely missing, so any such object
    /// (an extremely common pattern predating `__iter__`'s introduction,
    /// still widely used in real code and throughout CPython's own test
    /// suite) raised `TypeError: '...' object is not iterable`. See
    /// `builtin_iter`'s fallback construction and `builtin_next`'s handler.
    GetItemIter {
        obj: PyObjectRef,
        index: i64,
    },
    /// Backing for the two-argument `iter(callable, sentinel)` form: real
    /// Python calls `callable()` repeatedly, yielding each result until one
    /// equals `sentinel`, at which point iteration stops (matching
    /// CPython's own `callable_iterator`). Entirely missing before — found
    /// via CPython's own `test_iter.py`, which uses this extensively
    /// (`iter(file.readline, '')`-style idioms are the classic real-world
    /// use case, not just test-only).
    CallSentinelIter {
        func: PyObjectRef,
        sentinel: PyObjectRef,
        exhausted: bool,
    },
    /// Backing for `itertools.cycle(iterable)` — genuinely INFINITE, unlike
    /// every other `itertools` function in this file (which eagerly
    /// materializes into a plain list/`ListIter`, per this module's own
    /// established convention — impossible here since a cycle has no
    /// natural end). `index` wraps via `% items.len()` on each advance;
    /// an empty source iterable yields nothing forever (matching real
    /// CPython: `cycle([])` is a valid, immediately-exhausted iterator,
    /// not an error).
    CycleIter {
        items: Vec<PyObjectRef>,
        index: usize,
    },
    /// Backing for `itertools.groupby(iterable, key=None)` — must be a
    /// REAL lazy iterator (one `(key, group)` pair produced per `__next__`
    /// call), unlike this file's usual "eagerly materialize into a list"
    /// itertools convention: real CPython's own regression suite
    /// (`test_groupby_reentrant_eq_does_not_crash`, gh-143543) exercises a
    /// key comparison whose `__eq__` reentrantly calls `next()` on the
    /// SAME groupby object being constructed — an eager, single-pass
    /// implementation processes every element before ever returning,
    /// so that reentrant `next()` targets an object that doesn't exist yet
    /// (confirmed: this crashed with a `RefCell`-adjacent panic the first
    /// time an eager `groupby` was tried here). `pending` carries the
    /// (key, value) of the first item of the NEXT group, read one item
    /// ahead while scanning the CURRENT group to its boundary — each
    /// individual group is still eagerly collected into a plain `list`
    /// once its boundary is found (that simplification remains valid; only
    /// the OUTER key/group-pair production needed to become lazy).
    GroupByIter {
        source: PyObjectRef,
        key_func: Option<PyObjectRef>,
        pending: Option<(PyObjectRef, PyObjectRef)>,
        exhausted: bool,
    },
    /// Backing for `enumerate(iterable, start=0)` — `source` is the
    /// underlying iterator (already passed through `builtin_iter`), pulled
    /// from lazily one item per `__next__` call. Used to hold a fully
    /// pre-materialized `items: Vec<PyObjectRef>` instead (eagerly draining
    /// the WHOLE input during `enumerate()`'s own construction) — hung
    /// forever on any genuinely infinite iterable (`itertools.cycle`, a
    /// custom infinite generator, ...), since building that list never
    /// finished. `enumerate()` must not consume more of its argument than
    /// the caller actually asks for, exactly like real Python.
    EnumerateIter {
        source: PyObjectRef,
        pos: usize,
        start: usize,
    },
    MapIterator {
        func: PyObjectRef,
        iterator: Box<PyObjectRef>,
    },
    FilterIterator {
        func: PyObjectRef,
        iterator: Box<PyObjectRef>,
    },
    ZipIterator {
        iterators: Vec<PyObjectRef>,
    },
    Slice {
        start: PyObjectRef,
        stop: PyObjectRef,
        step: PyObjectRef,
    },
    // `Rc` (not `Box`) so `MAKE_FUNCTION` can share the same compiled body
    // across repeated executions of one `def`/`lambda` (e.g. one defined
    // fresh each iteration of a loop) via a cheap pointer clone, instead of
    // deep-cloning the whole `CodeObject` (instructions, consts, ...) every
    // time — see `PyObject::Function.code`'s own field comment, which this
    // directly feeds into via `LOAD_CONST`'s now-cached `PyObject::Code`
    // constant.
    Code(Rc<CodeObject>),
    // `Rc<CodeObject>` (not owned `CodeObject`, which is 344 bytes) — see
    // `PyObject::Generator`'s field comment for why embedding a large
    // struct by value here bloats the ENTIRE enum, not just this variant.
    // Sharing via `Rc` also means every `def`/`lambda` executed inside a
    // loop (a fresh `MAKE_FUNCTION` call each time, same underlying code)
    // no longer deep-clones its whole compiled body (instructions, consts,
    // ...) on every iteration — it did before, since this field previously
    // forced an owned copy.
    Function(Box<PyFunction>),
    BuiltinFunction {
        name: String,
        func: BuiltinFunc,
    },
    BuiltinMethod {
        name: String,
        func: BuiltinFunc,
        self_obj: PyObjectRef,
    },
    Module {
        name: String,
        dict: Box<TypeDict>,
    },
    Type {
        name: String,
        dict: Box<TypeDict>,
        bases: Vec<PyObjectRef>,
        mro: Vec<PyObjectRef>,
    },
    Instance {
        typ: PyObjectRef,
        dict: AttrMap,
    },
    Cell {
        value: Option<PyObjectRef>,
    },
    // Reserved for future C-extension capsule support (ffi_bridge.rs); not
    // constructed anywhere yet.
    #[allow(dead_code)]
    Capsule {
        name: String,
        pointer: *mut std::ffi::c_void,
    },
    Exception {
        typ: String,
        args: Vec<PyObjectRef>,
        cause: Option<PyObjectRef>,
    },
    ExceptionGroup {
        typ: String,
        args: Vec<PyObjectRef>,
        exceptions: Vec<PyObjectRef>,
    },
    BuildClass,
    BoundMethod {
        func: PyObjectRef,
        self_obj: PyObjectRef,
    },
    Partial {
        func: PyObjectRef,
        args: Vec<PyObjectRef>,
    },
    File {
        file: std::rc::Rc<std::cell::RefCell<std::fs::File>>,
        name: String,
    },
    /// Backing for `subprocess.Popen` — holds the spawned child process.
    /// `child` is `None` after `.communicate()`/`.wait()` has reaped it
    /// (via `wait_with_output`, which consumes the `Child`); `returncode`
    /// is filled in at that point and read by subsequent `.poll()`/`.wait()`
    /// calls without needing the (now-gone) child handle.
    Process {
        child: std::rc::Rc<std::cell::RefCell<Option<std::process::Child>>>,
        returncode: std::rc::Rc<std::cell::RefCell<Option<i64>>>,
        pid: i64,
    },
    Socket {
        inner: std::rc::Rc<std::cell::RefCell<SocketInner>>,
    },
    Thread(std::sync::Arc<std::sync::Mutex<ThreadInner>>),
    Lock(std::sync::Arc<std::sync::Mutex<LockInner>>),
    RLock(std::sync::Arc<std::sync::Mutex<RLockInner>>),
    Event(std::sync::Arc<EventInner>),
    Queue(std::sync::Arc<std::sync::Mutex<QueueInner>>),
    Super {
        cls: PyObjectRef,
        obj: PyObjectRef,
    },
    Property(Box<PropertyData>),
    StaticMethod {
        func: PyObjectRef,
    },
    ClassMethod {
        func: PyObjectRef,
    },
    // `Box<Frame>` (not a bare `Frame`) — `Frame` itself is ~536 bytes, and
    // since Rust sizes an enum to its LARGEST variant, embedding it inline
    // here made EVERY `PyObject` (a one-element list, an empty dict, a
    // short string) pay for 552 bytes of heap space regardless of its own
    // actual size. Boxing just these two variants (plus `Function.code`,
    // see its own field comment) shrinks the whole enum down to whatever
    // its new largest variant is instead.
    Generator {
        frame: std::cell::RefCell<Option<Box<crate::vm::Frame>>>,
    },
    Coroutine {
        frame: std::cell::RefCell<Option<Box<crate::vm::Frame>>>,
    },
    Array(PyArray),
    CompiledRegex {
        regex: Box<fancy_regex::Regex>,
        pattern: String,
        flags: i32,
    },
    Closure(Rc<dyn Fn(&[PyObjectRef]) -> PyResult<PyObjectRef>>),
    /// Implements the await protocol for Futures.
    /// __await__ returns this, and SEND drives it: first yield the future,
    /// then on second send (via send(None) from the event loop), return the result.
    /// Fully handled elsewhere in this file, but not constructed yet — part
    /// of the asyncio rewrite in PLAN.md.
    #[allow(dead_code)]
    FutureAwaitIterator {
        future: PyObjectRef,
        yielded: bool,
    },
}

#[derive(Clone, Debug)]
pub struct PropertyData {
    pub getter: Option<PyObjectRef>,
    pub setter: Option<PyObjectRef>,
    pub deleter: Option<PyObjectRef>,
    pub doc: Option<String>,
}

pub enum SocketInner {
    TcpListener(std::net::TcpListener),
    TcpStream(std::net::TcpStream),
    Uninitialized,
}

fn format_py_float(f: f64) -> String {
    if f.is_nan() { "nan".to_string() }
    else if f.is_infinite() && f.is_sign_positive() { "inf".to_string() }
    else if f.is_infinite() { "-inf".to_string() }
    else {
        let s = format!("{:.17}", f);
        let s = s.trim_end_matches('0').to_string();
        if s.ends_with('.') { format!("{}0", s) } else { s }
    }
}

/// Like `format_py_float`, but for a `complex` literal's real/imaginary
/// components — real CPython's `complex.__repr__` does NOT force a trailing
/// `.0` on whole-number parts the way `float.__repr__` does (`repr(2j)` ==
/// `'2j'`, not `'2.0j'`; `repr(complex(3,4))` == `'(3+4j)'`, not
/// `'(3.0+4.0j)'`).
fn format_complex_part(f: f64) -> String {
    if f.is_nan() { "nan".to_string() }
    else if f.is_infinite() && f.is_sign_positive() { "inf".to_string() }
    else if f.is_infinite() { "-inf".to_string() }
    else {
        let s = format!("{:.17}", f);
        let s = s.trim_end_matches('0').to_string();
        s.strip_suffix('.').map(|s| s.to_string()).unwrap_or(s)
    }
}

impl PyObject {
    pub fn type_name(&self) -> String {
        match self {
            PyObject::None => "NoneType",
            PyObject::Bool(_) => "bool",
            PyObject::Int(_) => "int",
            PyObject::Float(_) => "float",
            PyObject::Complex(..) => "complex",
            PyObject::Str(_) => "str",
            PyObject::Bytes(_) => "bytes",
            PyObject::ByteArray(_) => "bytearray",
            PyObject::List(_) => "list",
            PyObject::Tuple(_) => "tuple",
            PyObject::Dict(_) => "dict",
            PyObject::Set(_) => "set",
            PyObject::FrozenSet(_) => "frozenset",
            PyObject::Range { .. } => "range",
            PyObject::RangeIter { .. } => "range_iterator",
            PyObject::ListIter { .. } => "list_iterator",
            PyObject::GetItemIter { .. } => "iterator",
            PyObject::CallSentinelIter { .. } => "callable_iterator",
            PyObject::EnumerateIter { .. } => "enumerate",
            PyObject::MapIterator { .. } => "map",
            PyObject::FilterIterator { .. } => "filter",
            PyObject::ZipIterator { .. } => "zip",
            PyObject::Slice { .. } => "slice",
            PyObject::Code(_) => "code",
            PyObject::Function(_) => "function",
            PyObject::BuiltinFunction { .. } => "builtin_function_or_method",
            PyObject::BuiltinMethod { .. } => "builtin_method",
            PyObject::Module { .. } => "module",
            PyObject::Type { name, .. } => name,
            PyObject::Instance { .. } => "instance",
            PyObject::Cell { .. } => "cell",
            PyObject::Capsule { .. } => "capsule",
            PyObject::Exception { typ, .. } => typ,
            PyObject::ExceptionGroup { typ, .. } => typ,
            PyObject::BuildClass => "builtin_function_or_method",
            PyObject::BoundMethod { .. } => "method",
            PyObject::Partial { .. } => "partial",
            PyObject::File { .. } => "file",
            PyObject::Socket { .. } => "socket",
            PyObject::Thread(_) => "Thread",
            PyObject::Lock(_) => "lock",
            PyObject::RLock(_) => "RLock",
            PyObject::Event(_) => "Event",
            PyObject::Queue(_) => "Queue",
            PyObject::Super { .. } => "super",
            PyObject::Property(_) => "property",
            PyObject::StaticMethod { .. } => "staticmethod",
            PyObject::ClassMethod { .. } => "classmethod",
            PyObject::Generator { .. } => "generator",
            PyObject::Coroutine { .. } => "coroutine",
            PyObject::Array(_) => "array",
            PyObject::CompiledRegex { .. } => "re.Pattern",
            PyObject::Closure(_) => "builtin_function_or_method",
            PyObject::FutureAwaitIterator { .. } => "future_await_iterator",
            PyObject::Process { .. } => "Popen",
            PyObject::CycleIter { .. } => "itertools.cycle",
            PyObject::GroupByIter { .. } => "itertools.groupby",
        }.to_string()
    }

    pub fn repr(&self) -> String {
        match self {
            PyObject::None => "None".to_string(),
            PyObject::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            PyObject::Int(i) => i.to_string(),
            PyObject::Float(f) => format_py_float(*f),
            PyObject::Complex(re, im) => {
                // Real CPython: a zero real part reprs as just `<imag>j`;
                // otherwise `(<real><sign><|imag|>j)` — matches `repr(1+2j)`
                // == '(1+2j)', `repr(2j)` == '2j', `repr(1-2j)` == '(1-2j)'.
                if *re == 0.0 && re.is_sign_positive() {
                    format!("{}j", format_complex_part(*im))
                } else {
                    let sign = if im.is_sign_negative() { "-" } else { "+" };
                    format!("({}{}{}j)", format_complex_part(*re), sign, format_complex_part(im.abs()))
                }
            }
            PyObject::Str(s) => format!("'{}'", escape_string(s)),
            PyObject::Bytes(b) => {
                let s: String = b.iter().map(|&byte| {
                    match byte {
                        b'\\' => "\\\\".to_string(),
                        b'\'' => "\\'".to_string(),
                        b'\n' => "\\n".to_string(),
                        b'\t' => "\\t".to_string(),
                        b'\r' => "\\r".to_string(),
                        0x20..=0x7e => (byte as char).to_string(),
                        _ => format!("\\x{:02x}", byte),
                    }
                }).collect();
                format!("b'{}'", s)
            }
            PyObject::ByteArray(b) => {
                let s: String = b.iter().map(|&byte| {
                    match byte {
                        b'\\' => "\\\\".to_string(),
                        b'\'' => "\\'".to_string(),
                        b'\n' => "\\n".to_string(),
                        b'\t' => "\\t".to_string(),
                        b'\r' => "\\r".to_string(),
                        0x20..=0x7e => (byte as char).to_string(),
                        _ => format!("\\x{:02x}", byte),
                    }
                }).collect();
                format!("bytearray(b'{}')", s)
            }
            PyObject::List(items) => {
                let items: Vec<String> = items.iter().map(|x| x.repr()).collect();
                format!("[{}]", items.join(", "))
            }
            PyObject::Tuple(items) => {
                let items: Vec<String> = items.iter().map(|x| x.repr()).collect();
                if items.len() == 1 {
                    format!("({},)", items[0])
                } else {
                    format!("({})", items.join(", "))
                }
            }
            PyObject::Dict(d) => {
                let items: Vec<String> = d.items().iter()
                    .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            PyObject::Set(items) => {
                let vec = items.to_vec();
                let items: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                format!("{{{}}}", items.join(", "))
            }
            PyObject::FrozenSet(items) => {
                let vec = items.to_vec();
                let items: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                format!("frozenset({{{}}})", items.join(", "))
            }
            PyObject::Range { start, stop, step } => {
                if *step == 1 { format!("range({}, {})", start, stop) }
                else { format!("range({}, {}, {})", start, stop, step) }
            }
            PyObject::RangeIter { .. } => "<range_iterator object>".to_string(),
            PyObject::ListIter { .. } => "<list_iterator object>".to_string(),
            PyObject::GetItemIter { .. } => "<iterator object>".to_string(),
            PyObject::CallSentinelIter { .. } => "<callable_iterator object>".to_string(),
            PyObject::EnumerateIter { .. } => "<enumerate object>".to_string(),
            PyObject::MapIterator { .. } => "<map object>".to_string(),
            PyObject::FilterIterator { .. } => "<filter object>".to_string(),
            PyObject::ZipIterator { .. } => "<zip object>".to_string(),
            PyObject::Slice { start, stop, step } => {
                format!("slice({}, {}, {})", start.repr(), stop.repr(), step.repr())
            }
            PyObject::Function(ref f) => format!("<function {}>", f.code.name),
            PyObject::BuiltinFunction { name, .. } => format!("<built-in function {}>", name),
            PyObject::BuiltinMethod { name, .. } => format!("<built-in method {}>", name),
            PyObject::Module { name, .. } => format!("<module '{}'>", name),
            PyObject::Type { name, .. } => format!("<class '{}'>", name),
            PyObject::Instance { typ, .. } => {
                format!("<{} object>", typ.borrow().type_name())
            }
            PyObject::Code(c) => format!("<code object {}>", c.name),
            PyObject::Cell { value: Some(v) } => v.repr(),
            PyObject::Cell { value: None } => "None".to_string(),
            PyObject::Capsule { name, .. } => format!("<capsule object '{}'>", name),
            PyObject::Exception { typ, args, cause: _ } => {
                let args_str: Vec<String> = args.iter().map(|a| a.repr()).collect();
                format!("{}({})", typ, args_str.join(", "))
            }
            PyObject::ExceptionGroup { typ, args, exceptions } => {
                let args_str: Vec<String> = args.iter().map(|a| a.repr()).collect();
                let exc_str: Vec<String> = exceptions.iter().map(|e| e.repr()).collect();
                format!("{}({}, {})", typ, args_str.join(", "), exc_str.join(", "))
            }
            PyObject::BuildClass => "<builtin function __build_class__>".to_string(),
            PyObject::BoundMethod { func, .. } => format!("<bound method {}>", func.borrow().type_name()),
            PyObject::Partial { func, .. } => format!("<partial {}>", func.borrow().type_name()),
            PyObject::File { name, .. } => format!("<_io.FileIO '{}'>", name),
            PyObject::Socket { .. } => format!("<socket object>"),
            PyObject::Thread(_) => "<Thread>".to_string(),
            PyObject::Lock(_) => "<lock>".to_string(),
            PyObject::RLock(_) => "<RLock>".to_string(),
            PyObject::Event(_) => "<Event>".to_string(),
            PyObject::Queue(_) => "<Queue>".to_string(),
            PyObject::Super { .. } => format!("<super object>"),
            PyObject::Property(_) => format!("<property object>"),
            PyObject::StaticMethod { .. } => format!("<staticmethod object>"),
            PyObject::ClassMethod { .. } => format!("<classmethod object>"),
            PyObject::Generator { .. } => format!("<generator object>"),
            PyObject::Coroutine { .. } => format!("<coroutine object>"),
            PyObject::Array(arr) => {
                let items: Vec<String> = arr.data.iter().map(|v| {
                    if array_typecode_is_float(arr.typecode) {
                        py_float(*v).repr()
                    } else {
                        py_int(*v as i64).repr()
                    }
                }).collect();
                format!("array('{}', [{}])", arr.typecode, items.join(", "))
            },
            PyObject::CompiledRegex { pattern, .. } => format!("re.compile('{}')", pattern),
            PyObject::Closure(_) => "<builtin function>".to_string(),
            PyObject::FutureAwaitIterator { future, yielded } => {
                format!("<future_await_iterator future={} yielded={}>", future.repr(), yielded)
            }
            PyObject::Process { pid, returncode, .. } => {
                format!("<Popen: returncode: {} args: [pid {}]>", returncode.borrow().map(|r| r.to_string()).unwrap_or_else(|| "None".to_string()), pid)
            }
            PyObject::CycleIter { .. } => "<itertools.cycle object>".to_string(),
            PyObject::GroupByIter { .. } => "<itertools.groupby object>".to_string(),
        }
    }

    pub fn str(&self) -> String {
        match self {
            PyObject::Str(s) => s.to_string(),
            // Real `BaseException.__str__` shows just the message, not
            // `TypeName(args...)` — `object.__str__`'s repr-fallback default
            // (used for every other type below) is overridden specifically
            // for exceptions: 0 args -> "", 1 arg -> str(that arg), 2+ args
            // -> repr of the whole args tuple. Missing this meant
            // `str(some_exception)`/an f-string embedding it/anything that
            // implicitly stringifies an exception (very common — logging,
            // `unittest`'s own traceback formatting) showed the redundant
            // `AssertionError('msg')` instead of plain `msg`.
            PyObject::Exception { args, .. } => match args.as_slice() {
                [] => String::new(),
                [one] => one.str(),
                many => format!("({})", many.iter().map(|a| a.repr()).collect::<Vec<_>>().join(", ")),
            },
            _ => self.repr(),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            PyObject::None => false,
            PyObject::Bool(b) => *b,
            PyObject::Int(i) => !i.is_zero(),
            PyObject::Float(f) => *f != 0.0,
            PyObject::Str(s) => !s.is_empty(),
            PyObject::List(v) => !v.is_empty(),
            PyObject::Tuple(v) => !v.is_empty(),
            PyObject::Dict(d) => !d.is_empty(),
            PyObject::Set(s) => !s.is_empty(),
            PyObject::FrozenSet(s) => !s.is_empty(),
            PyObject::Range { start, stop, step } => *step > 0 && *start < *stop || *step < 0 && *start > *stop,
            PyObject::RangeIter { current, stop, step } => *step > 0 && *current < *stop || *step < 0 && *current > *stop,
            // Real CPython's `enumerate` object has no `__bool__`/`__len__`
            // at all — it's always truthy regardless of remaining items
            // (matches the default object truthiness rule). The previous
            // `*pos < items.len()` check is no longer even expressible now
            // that `EnumerateIter` holds a lazy `source` instead of a
            // pre-materialized `items` list — and was arguably wrong
            // before too, since it's not what real Python does.
            PyObject::EnumerateIter { .. } => true,
            PyObject::Instance { typ, dict } => {
                let f = lookup_dunder_via_mro(typ, "__bool__");
                if let Some(f) = f {
                    if let Ok(result) = call_bound_method(f, PyObjectRef::new(PyObject::Instance { typ: typ.clone(), dict: dict.clone() }), vec![]) {
                        // See the matching comment in `PyObjectRef::truthy()`:
                        // a non-bool `__bool__` return (e.g. `return self`)
                        // must not recurse into `.truthy()` again — that
                        // infinite-loops instead of erroring like real
                        // CPython does.
                        if let PyObjectRef::SmallBool(b) = result {
                            return b;
                        }
                        return true;
                    }
                }
                true
            }
            PyObject::Array(arr) => !arr.data.is_empty(),
            PyObject::CompiledRegex { .. } => true,
            PyObject::Closure(_) => true,
            _ => true,
        }
    }

    pub fn hash(&self) -> PyResult<usize> {
        match self {
            PyObject::None => Ok(0),
            PyObject::Bool(b) => Ok(if *b { 1 } else { 0 }),
            PyObject::Int(i) => Ok(hash_bigint(i)),
            // A whole-number float must hash IDENTICALLY to the equal int
            // (`1.0 == 1` is true, per the numeric-tower equality fix above,
            // and Python's dict/set invariant requires `a == b => hash(a) ==
            // hash(b)` — otherwise `{1: 'x'}[1.0]` raises `KeyError` even
            // though `1.0 in {1: 'x'}` reports the key as present via `==`).
            // Reuses Int's own (already-established) hash function directly
            // rather than reimplementing CPython's real mod-2**61-1 float
            // hash algorithm — this covers the overwhelmingly common case
            // (whole-number float dict/set keys) without changing Int's own
            // existing hash values. Non-whole-number floats keep the prior
            // bit-pattern hash (internally consistent, just not
            // cross-type-matching — which only matters for fractional
            // int/float equality, impossible for finite non-whole floats).
            PyObject::Float(f) => {
                if f.is_nan() {
                    Ok(0)
                } else if f.fract() == 0.0 && f.is_finite() && f.abs() < 1e18 {
                    Ok(hash_bigint(&BigInt::from(*f as i64)))
                } else {
                    Ok(f.to_bits() as usize)
                }
            }
            PyObject::Complex(re, im) => {
                let real_hash = PyObject::Float(*re).hash()?;
                if *im == 0.0 {
                    Ok(real_hash)
                } else {
                    let imag_hash = PyObject::Float(*im).hash()?;
                    let combined = (real_hash as i64).wrapping_add(1000003i64.wrapping_mul(imag_hash as i64));
                    Ok((if combined == -1 { -2 } else { combined }) as usize)
                }
            }
            PyObject::Str(s) => {
                let mut h: usize = 0;
                for c in s.chars() {
                    h = h.wrapping_mul(1000003).wrapping_add(c as usize);
                }
                Ok(h)
            }
            PyObject::Bytes(b) => {
                let mut h: usize = 0;
                for &byte in b { h = h.wrapping_mul(1000003).wrapping_add(byte as usize); }
                Ok(h)
            }
            PyObject::Range { start, stop, step } => {
                let mut h: usize = 0x123456;
                h = h.wrapping_mul(1000003).wrapping_add(*start as usize);
                h = h.wrapping_mul(1000003).wrapping_add(*stop as usize);
                h = h.wrapping_mul(1000003).wrapping_add(*step as usize);
                Ok(h)
            }
            PyObject::Tuple(items) => {
                let mut h: usize = 0x345678;
                for item in items {
                    h = h.wrapping_mul(1000003).wrapping_add(item.hash()?);
                }
                Ok(h)
            }
            PyObject::FrozenSet(items) => {
                let mut h: usize = 0x987654;
                for item in items.to_vec() {
                    h = h.wrapping_mul(1000003).wrapping_add(item.hash()?);
                }
                Ok(h)
            }
            PyObject::Instance { typ, dict } => {
                // Check for __hash__ method (walking the MRO)
                let f = lookup_dunder_via_mro(typ, "__hash__");
                if let Some(f) = f {
                    let result = call_bound_method(f, PyObjectRef::new(PyObject::Instance { typ: typ.clone(), dict: dict.clone() }), vec![])?;
                    let n = result.borrow();
                    if let PyObject::Int(i) = &*n {
                        let bytes = i.to_signed_bytes_le();
                        let mut h: usize = 0;
                        for (j, &b) in bytes.iter().enumerate() {
                            h ^= (b as usize) << ((j % (std::mem::size_of::<usize>())) * 8);
                        }
                        Ok(h)
                    } else {
                        Err(PyError::type_error("__hash__ should return an integer"))
                    }
                } else if let Some(native) = dict.get(NATIVE_BACKING_KEY) {
                    native.hash()
                } else {
                    Err(PyError::type_error(format!("unhashable type: '{}'", self.type_name())))
                }
            }
            PyObject::Array(arr) => {
                let mut h: usize = 0xabcdef;
                for &v in &arr.data {
                    let bits = v.to_bits();
                    h = h.wrapping_mul(1000003).wrapping_add(bits as usize);
                }
                Ok(h)
            }
            PyObject::Slice { start, stop, step } => {
                let mut h: usize = 0x345679;
                h = h.wrapping_mul(1000003).wrapping_add(start.hash()?);
                h = h.wrapping_mul(1000003).wrapping_add(stop.hash()?);
                h = h.wrapping_mul(1000003).wrapping_add(step.hash()?);
                Ok(h)
            }
            PyObject::CompiledRegex { pattern, flags, .. } => {
                let mut h: usize = 0x123456;
                for b in pattern.bytes() {
                    h = h.wrapping_mul(1000003).wrapping_add(b as usize);
                }
                h = h.wrapping_mul(1000003).wrapping_add(*flags as usize);
                Ok(h)
            }
            // Functions, types, modules, etc. are hashable by identity in
            // real Python (there's no reasonable structural hash for them,
            // but there's no reason they should be unhashable either — code
            // that registers callbacks in a set/dict, e.g. Django's check
            // registry, relies on this). `self` here is `&PyObject` reached
            // via a `Ref` guard borrowed from the object's own Rc, so its
            // address is stable across calls as long as callers don't
            // reconstruct a throwaway clone first (unlike the Instance case
            // above, which needed its own fix for exactly that reason).
            PyObject::Function(_) | PyObject::BuiltinFunction { .. } | PyObject::BuiltinMethod { .. }
            | PyObject::Type { .. } | PyObject::Module { .. } | PyObject::BoundMethod { .. } => {
                Ok(self as *const PyObject as usize)
            }
            PyObject::Closure(_) => Err(PyError::type_error(format!("unhashable type: '{}'", self.type_name()))),
            _ => Err(PyError::type_error(format!("unhashable type: '{}'", self.type_name()))),
        }
    }

    pub fn equals(&self, other_ref: &PyObjectRef) -> PyResult<bool> {
        // This structural-equality function is what PyDict/PySet actually
        // use to disambiguate a hash bucket — unlike py_compare's `==`
        // operator dispatch, it never checked a custom class's __eq__ at
        // all, so ANY hashable user-defined class (not just list/dict/str
        // native-backed ones) silently failed as a dict/set key: two keys
        // that compared equal via `==` and hashed the same would still
        // fail to be found by that key, because dict lookup calls this
        // function directly, bypassing dunder dispatch entirely.
        if let PyObject::Instance { typ, .. } = self {
            if let Some(f) = lookup_dunder_via_mro(typ, "__eq__") {
                let self_ref = PyObjectRef::new(self.clone());
                let result = call_bound_method(f, self_ref, vec![other_ref.clone()])?;
                if !is_not_implemented(&result) {
                    return Ok(result.truthy());
                }
            }
        }
        // Instances that transparently subclass list/dict/str (and don't
        // override __eq__ themselves) compare via their native backing,
        // matching CPython's list/dict/str __eq__ (structural, regardless
        // of subclass identity).
        if let PyObject::Instance { dict, .. } = self {
            if let Some(native) = dict.get(NATIVE_BACKING_KEY) {
                return native.equals(other_ref);
            }
        }
        let other_native = if let PyObject::Instance { dict, .. } = &*other_ref.borrow() {
            dict.get(NATIVE_BACKING_KEY).cloned()
        } else {
            None
        };
        if let Some(native) = other_native {
            let self_ref = PyObjectRef::new(self.clone());
            return self_ref.equals(&native);
        }
        let other = other_ref.borrow();
        // Numeric cross-type equality — `1 == 1.0`, `1 == (1+0j)`, `True ==
        // 1` must all be True, matching Python's numeric tower (comparison
        // is by VALUE across int/float/bool/complex, never gated by the
        // concrete Rust variant). Excludes Int==Int specifically so two
        // plain ints keep comparing via exact `BigInt` equality below
        // instead of a lossy `to_f64()` round-trip that would silently
        // break equality for integers beyond f64's 53-bit mantissa.
        let both_plain_ints = matches!(self, PyObject::Int(_)) && matches!(&*other, PyObject::Int(_));
        if !both_plain_ints {
            if let (Some(a_parts), Some(b_parts)) = (as_complex_parts(self), as_complex_parts(&*other)) {
                return Ok(a_parts == b_parts);
            }
        }
        // `bytes`/`bytearray` compare equal across the two variants by
        // content — real Python's `bytes.__eq__`/`bytearray.__eq__` both
        // accept either type on the other side (`bytearray(b'abcd') ==
        // b'abcd'` is `True`). Checked before the discriminant short-circuit
        // below, which otherwise always returned `False` for any two
        // differently-tagged variants regardless of content — confirmed via
        // CPython's own `test_base64.py`.
        match (self, &*other) {
            (PyObject::Bytes(a), PyObject::ByteArray(b)) | (PyObject::ByteArray(b), PyObject::Bytes(a)) => {
                return Ok(a.as_slice() == b.as_slice());
            }
            _ => {}
        }
        if std::mem::discriminant(self) != std::mem::discriminant(&*other) {
            return Ok(false);
        }
        let result = match (self, &*other) {
            (PyObject::None, PyObject::None) => true,
            (PyObject::Bool(a), PyObject::Bool(b)) => a == b,
            (PyObject::Int(a), PyObject::Int(b)) => a == b,
            (PyObject::Float(a), PyObject::Float(b)) => a == b,
            (PyObject::Str(a), PyObject::Str(b)) => a == b,
            (PyObject::Bytes(a), PyObject::Bytes(b)) => a == b,
            (PyObject::ByteArray(a), PyObject::ByteArray(b)) => a == b,
            (PyObject::Dict(a), PyObject::Dict(b)) => {
                if a.len() != b.len() { false }
                else {
                    let mut eq = true;
                    for (k, va) in a.items() {
                        match b.get(&k) {
                            Ok(Some(vb)) => { if !va.equals(&vb)? { eq = false; break; } }
                            _ => { eq = false; break; }
                        }
                    }
                    eq
                }
            }
            (PyObject::List(a), PyObject::List(b)) => {
                let mut eq = true;
                if a.len() != b.len() { eq = false; }
                if eq {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !(x.is(y) || x.equals(y)?) { eq = false; break; }
                    }
                }
                eq
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                let mut eq = true;
                if a.len() != b.len() { eq = false; }
                if eq {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !(x.is(y) || x.equals(y)?) { eq = false; break; }
                    }
                }
                eq
            }
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() != b.len() { false }
                else {
                    let mut eq = true;
                    for item in a.to_vec() {
                        if !b.contains(&item).unwrap_or(false) { eq = false; break; }
                    }
                    eq
                }
            }
            (PyObject::FrozenSet(a), PyObject::FrozenSet(b)) => {
                if a.len() != b.len() { false }
                else {
                    let mut eq = true;
                    for item in a.to_vec() {
                        if !b.contains(&item).unwrap_or(false) { eq = false; break; }
                    }
                    eq
                }
            }
            (PyObject::Array(a), PyObject::Array(b)) => a.typecode == b.typecode && a.data == b.data,
            (PyObject::Slice { start: as_, stop: ae, step: ap }, PyObject::Slice { start: bs, stop: be, step: bp }) => {
                as_.equals(bs)? && ae.equals(be)? && ap.equals(bp)?
            }
            (PyObject::Range { start: a, stop: ae, step: ap }, PyObject::Range { start: b, stop: be, step: bp }) => {
                a == b && ae == be && ap == bp
            }
            (PyObject::CompiledRegex { pattern: a, flags: af, .. }, PyObject::CompiledRegex { pattern: b, flags: bf, .. }) => a == b && af == bf,
            // Reference-identity types (matching the identity-based hash
            // above): equal iff it's really the same underlying object.
            (PyObject::Function(_), PyObject::Function(_))
            | (PyObject::BuiltinFunction { .. }, PyObject::BuiltinFunction { .. })
            | (PyObject::BuiltinMethod { .. }, PyObject::BuiltinMethod { .. })
            | (PyObject::Type { .. }, PyObject::Type { .. })
            | (PyObject::Module { .. }, PyObject::Module { .. })
            | (PyObject::BoundMethod { .. }, PyObject::BoundMethod { .. }) => {
                std::ptr::eq(self as *const PyObject, &*other as *const PyObject)
            }
            _ => false,
        };
        Ok(result)
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\"' => out.push_str("\\\""),
            '\x00'..='\x1f' => out.push_str(&format!("\\x{:02x}", c as u8)),
            '\x7f' => out.push_str("\\x7f"),
            c if c.is_control() => match c as u32 {
                code @ 0..=0xff => out.push_str(&format!("\\x{:02x}", code as u8)),
                code @ 0x100..=0xffff => out.push_str(&format!("\\u{:04x}", code)),
                code => out.push_str(&format!("\\U{:08x}", code)),
            },
            c => out.push(c),
        }
    }
    out
}
