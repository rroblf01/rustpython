// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds `PyFunction`, the
// core `PyObject` enum definition, `PropertyData`, `SocketInner`, and
// `PyObject`'s basic accessor methods (`type_name`/`repr`/`str`/`truthy`/
// `hash`/`equals`).
use super::*;
mod func;
pub use func::PyFunction;
mod format;
pub(crate) use format::{escape_string, format_complex_part, format_py_float};
mod repr;
mod hash;
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
    /// Backing for `collections.deque`. `maxlen` follows real CPython: a
    /// bounded deque drops items from the OPPOSITE end as new ones arrive
    /// (`deque('abcdef', 4)` == `deque('cdef')`), `None` means unbounded.
    /// Stored as a `VecDeque` for O(1) amortized push/pop at both ends.
    Deque {
        data: VecDeque<PyObjectRef>,
        maxlen: Option<usize>,
    },
    Tuple(Vec<PyObjectRef>),
    Dict(Box<PyDict>),
    /// Live view of a frame's `globals` (`Rc<RefCell<HashMap<StrId, ..>>>`)
    /// returned by the `globals()` builtin. Mutations go straight to the
    /// backing globals map, so `globals()['len'] = f` is visible to
    /// `LOAD_GLOBAL` (test_dynamic::test_globals_shadow_builtins) — unlike a
    /// copied `PyDict`.
    Globals(Rc<RefCell<HashMap<StrId, PyObjectRef>>>),
    Set(PySet),
    FrozenSet(PySet),
    Range {
        start: num_bigint::BigInt,
        stop: num_bigint::BigInt,
        step: num_bigint::BigInt,
    },
    RangeIter {
        current: num_bigint::BigInt,
        stop: num_bigint::BigInt,
        step: num_bigint::BigInt,
    },
    ListIter {
        list: Vec<PyObjectRef>,
        index: usize,
    },
    /// Backing for `iter(deque)` — a LIVE iterator over the deque (mutations
    /// are reflected, unlike a snapshot), with CPython's mutation detection:
    /// if the deque's length changes between the iterator's creation and a
    /// `next()` call, `RuntimeError: deque mutated during iteration` is
    /// raised (real trigger: `test_deque.py`'s `test_iter_with_altered_data`
    /// and `test_runtime_error_on_empty_deque`). `start_len` is the length
    /// at iterator creation; `index` advances through `deque[index]`.
    DequeIter {
        deque: PyObjectRef,
        index: usize,
        start_len: usize,
    },
    /// Backing for `reversed(deque)` — reverse LIVE iterator with mutation detection.
    DequeRevIter {
        deque: PyObjectRef,
        index: isize,
        start_len: usize,
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
    /// A real weak reference (`weakref.ref(obj)`): holds only a `Weak` to the
    /// target, so it never keeps the target alive. Calling it yields the
    /// target (or `None`, or a caller-supplied default once dead).
    WeakRef {
        target: std::rc::Weak<std::cell::RefCell<PyObject>>,
        callback: Option<PyObjectRef>,
        hash_cache: std::cell::RefCell<Option<usize>>,
    },
    WeakProxy {
        target: std::rc::Weak<std::cell::RefCell<PyObject>>,
        callback: Option<PyObjectRef>,
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
        suppress_context: bool,
        context: Option<PyObjectRef>,
        traceback: Option<PyObjectRef>,
        /// Per-instance extra attributes (`BaseException.__dict__`): keyword
        /// constructor args (e.g. `AttributeError('x', name=..., obj=...)`)
        /// and any ad-hoc attributes assigned by user code.
        extra: Option<HashMap<String, PyObjectRef>>,
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
        /// Per-instance attribute storage (CPython's partial has __dict__).
        dict: crate::object::core::AttrMap,
    },
    File {
        file: std::rc::Rc<std::cell::RefCell<std::fs::File>>,
        name: String,
        // Real Python's `open(path, mode)` returns `bytes` from `read()`/
        // `readline()`/iteration when `'b'` is in `mode`, `str` otherwise —
        // was previously never tracked at all (every read unconditionally
        // decoded as UTF-8 text), so `open(path, 'rb').read()` returned a
        // `str`, not `bytes` (real trigger: `dbm/dumb.py`'s own
        // `__getitem__`, `with _io.open(self._datfile, 'rb') as f: f.seek
        // (pos); dat = f.read(siz)` — expects `dat` to be raw `bytes`).
        binary: bool,
        // Text-mode incremental-decoder state: an incomplete UTF-8 sequence
        // at the end of a `read(n)` chunk that landed mid-multibyte-character
        // is buffered here and carried into the NEXT read, so streaming
        // `read(1)`-at-a-time (or any chunk boundary) doesn't corrupt a
        // character into U+FFFD replacement chars. Real Python's
        // `TextIOWrapper` keeps exactly this state. (Confirmed via
        // `test_netrc.py::test_token_value_non_ascii`, which reads a
        // UTF-8 file one byte at a time.)
        pending: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
        closed: bool,
    },
    /// Backing for `subprocess.Popen` — holds the spawned child process.
    /// `child` is `None` after `.communicate()`/`.wait()` has reaped it
    /// (via `wait_with_output`, which consumes the `Child`); `returncode`
    /// is filled in at that point and read by subsequent `.poll()`/`.wait()`
    /// calls without needing the (now-gone) child handle. When constructed
    /// with `stdin/stdout/stderr=PIPE`, the corresponding pipe ends are
    /// taken from the child at construction and exposed as `.stdin`/
    /// `.stdout`/`.stderr` file objects (real CPython exposes them; the
    /// interactive REPL tests write a statement then read the prompt).
    Process {
        child: std::rc::Rc<std::cell::RefCell<Option<std::process::Child>>>,
        returncode: std::rc::Rc<std::cell::RefCell<Option<i64>>>,
        pid: i64,
        stdin_pipe: Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>>,
        stdout_pipe: Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>>,
        stderr_pipe: Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>>,
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
    /// A real `memoryview` — previously just aliased to a CLONED
    /// `bytearray` (no `.cast()`/`.format`/`.shape`/multi-dimensional
    /// support at all, and mutations through it never reflected back into
    /// the original buffer). `source` is a clone of the original `bytes`/
    /// `bytearray` `PyObjectRef` — since cloning a `PyObjectRef::Mut(Rc<
    /// RefCell<_>>)` clones the `Rc`, reading/writing through `source`
    /// naturally shares the SAME underlying storage as the original object
    /// (and any other memoryview over it), giving correct write-through
    /// semantics for free. `format`/`shape`/`itemsize` describe how the
    /// flat byte range is reinterpreted (set by `.cast()`); `offset` is a
    /// BYTE offset into `source`'s raw bytes (nonzero after slicing).
    MemoryView {
        source: PyObjectRef,
        format: String,
        shape: Vec<usize>,
        itemsize: usize,
        offset: usize,
        readonly: bool,
        released: bool,
    },
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
            PyObject::Deque { .. } => "deque",
            PyObject::Tuple(_) => "tuple",
            PyObject::Dict(_) => "dict",
            PyObject::Globals(_) => "dict",
            PyObject::Set(_) => "set",
            PyObject::FrozenSet(_) => "frozenset",
            PyObject::Range { .. } => "range",
            PyObject::RangeIter { .. } => "range_iterator",
            PyObject::ListIter { .. } => "list_iterator",
            PyObject::DequeIter { .. } => "deque_iterator",
            PyObject::DequeRevIter { .. } => "deque_reverse_iterator",
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
            PyObject::WeakRef { .. } => "weakref",
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    let b = rc.borrow();
                    let is_callable = match &*b {
                        PyObject::Instance { typ, .. } => crate::object::lookup_dunder_via_mro(typ, "__call__").is_some(),
                        PyObject::Function(_) | PyObject::BuiltinFunction { .. } | PyObject::BuiltinMethod { .. } | PyObject::BoundMethod { .. } => true,
                        _ => b.get_attribute("__call__").is_ok(),
                    };
                    if is_callable { "weakcallableproxy" } else { "weakproxy" }
                } else { "weakproxy" }
            },
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
            PyObject::MemoryView { .. } => "memoryview",
            PyObject::CompiledRegex { .. } => "re.Pattern",
            PyObject::Closure(_) => "builtin_function_or_method",
            PyObject::FutureAwaitIterator { .. } => "future_await_iterator",
            PyObject::Process { .. } => "Popen",
            PyObject::CycleIter { .. } => "itertools.cycle",
            PyObject::GroupByIter { .. } => "itertools.groupby",
        }
        .to_string()
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
                many => format!(
                    "({})",
                    many.iter().map(|a| a.repr()).collect::<Vec<_>>().join(", ")
                ),
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
            PyObject::Deque { data, .. } => !data.is_empty(),
            PyObject::Tuple(v) => !v.is_empty(),
            PyObject::Dict(d) => !d.is_empty(),
            PyObject::Set(s) => !s.is_empty(),
            PyObject::FrozenSet(s) => !s.is_empty(),
            PyObject::Range { start, stop, step } => {
                (step.sign() == num_bigint::Sign::Plus && start < stop)
                    || (step.sign() == num_bigint::Sign::Minus && start > stop)
            }
            PyObject::RangeIter {
                current,
                stop,
                step,
            } => {
                (step.sign() == num_bigint::Sign::Plus && current < stop)
                    || (step.sign() == num_bigint::Sign::Minus && current > stop)
            }
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
                    if let Ok(result) = call_bound_method(
                        f,
                        PyObjectRef::new(PyObject::Instance {
                            typ: typ.clone(),
                            dict: dict.clone(),
                        }),
                        vec![],
                    ) {
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
            PyObject::MemoryView { shape, .. } => shape.first().copied().unwrap_or(0) != 0,
            PyObject::CompiledRegex { .. } => true,
            PyObject::Closure(_) => true,
            // `bytes`/`bytearray` were missing entirely from this match —
            // silently fell to the generic `_ => true` catch-all, so
            // `bool(b'')`/`bool(bytearray())` (and anywhere either is used
            // in an implicit truth-test — `if not f.readline():`, an
            // extremely common EOF-detection idiom) always came back
            // `True` regardless of actual (non-)emptiness. Confirmed via
            // CPython's own `test_bufio.py` (`self.assertFalse(line)` on
            // an empty-bytes EOF sentinel from `readline()`).
            PyObject::Bytes(b) => !b.is_empty(),
            PyObject::ByteArray(b) => !b.is_empty(),
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    return rc.borrow().truthy();
                } else {
                    return false;
                }
            }
            _ => true,
        }
    }


    pub fn equals(&self, other_ref: &PyObjectRef) -> PyResult<bool> {
        if let PyObject::WeakRef { target, .. } = self {
            if let PyObject::WeakRef { target: other_target, .. } = &*other_ref.borrow() {
                let self_alive = target.upgrade();
                let other_alive = other_target.upgrade();
                match (self_alive, other_alive) {
                    (Some(a_rc), Some(b_rc)) => {
                        return PyObjectRef::Imm(a_rc).equals(&PyObjectRef::Imm(b_rc));
                    }
                    _ => {
                        return Ok(std::ptr::eq(self as *const PyObject, &*other_ref.borrow() as *const PyObject));
                    }
                }
            } else {
                return Ok(false);
            }
        }
        if let PyObject::WeakRef { .. } = &*other_ref.borrow() {
            return Ok(false);
        }
        if let PyObject::WeakProxy { target, .. } = self {
            if let Some(rc) = target.upgrade() {
                return rc.borrow().equals(other_ref);
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }
        if let PyObject::WeakProxy { target, .. } = &*other_ref.borrow() {
            if let Some(rc) = target.upgrade() {
                return self.equals(&PyObjectRef::Imm(rc));
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }
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
        // `fractions.Fraction`/`decimal.Decimal` instances join the tower
        // too (`Fraction(2002,2) == 1001+0j` — real test_compare.py's
        // assert_equality_only over the numeric types).
        if let (Some(a), Some(b)) = (
            crate::modules::numeric_parts_from_ref(&PyObjectRef::new(self.clone())),
            crate::modules::numeric_parts_from_ref(other_ref),
        ) {
            if !matches!(self, PyObject::Int(_)) || !matches!(&*other, PyObject::Int(_)) {
                return Ok(a == b);
            }
        }
        let both_plain_ints =
            matches!(self, PyObject::Int(_)) && matches!(&*other, PyObject::Int(_));
        if !both_plain_ints {
            if let (Some(a_parts), Some(b_parts)) =
                (as_complex_parts(self), as_complex_parts(&*other))
            {
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
            (PyObject::Bytes(a), PyObject::ByteArray(b))
            | (PyObject::ByteArray(b), PyObject::Bytes(a)) => {
                return Ok(a.as_slice() == b.as_slice());
            }
            _ => {}
        }
        // `memoryview` compares equal to `bytes`/`bytearray`/another
        // `memoryview` by CONTENT (its own flat byte range, respecting
        // `.cast()`-adjusted format/shape/offset) — checked before the
        // discriminant short-circuit below for the same reason as the
        // `bytes`/`bytearray` cross-type case just above.
        if matches!(self, PyObject::MemoryView { .. }) {
            let self_ref = PyObjectRef::new(self.clone());
            return Ok(mv_equals(&self_ref, other_ref));
        }
        if matches!(&*other, PyObject::MemoryView { .. }) {
            let self_ref = PyObjectRef::new(self.clone());
            return Ok(mv_equals(other_ref, &self_ref));
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
                if a.len() != b.len() {
                    false
                } else {
                    let mut eq = true;
                    for (k, va) in a.items() {
                        match b.get(&k) {
                            Ok(Some(vb)) => {
                                if !va.equals(&vb)? {
                                    eq = false;
                                    break;
                                }
                            }
                            _ => {
                                eq = false;
                                break;
                            }
                        }
                    }
                    eq
                }
            }
            (PyObject::List(a), PyObject::List(b)) => {
                let mut eq = true;
                if a.len() != b.len() {
                    eq = false;
                }
                if eq {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !(x.is(y) || x.equals(y)?) {
                            eq = false;
                            break;
                        }
                    }
                }
                eq
            }
            (PyObject::Deque { data: a, .. }, PyObject::Deque { data: b, .. }) => {
                // Content-only equality — real CPython's `deque.__eq__`
                // ignores `maxlen` (`deque('abc') == deque('abc', 3)` is
                // True) and returns NotImplemented for non-deques (so
                // `deque('abc') == ['a','b','c']` is False).
                let mut eq = true;
                if a.len() != b.len() {
                    eq = false;
                }
                if eq {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !(x.is(y) || x.equals(y)?) {
                            eq = false;
                            break;
                        }
                    }
                }
                eq
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                let mut eq = true;
                if a.len() != b.len() {
                    eq = false;
                }
                if eq {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !(x.is(y) || x.equals(y)?) {
                            eq = false;
                            break;
                        }
                    }
                }
                eq
            }
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() != b.len() {
                    false
                } else {
                    let mut eq = true;
                    for item in a.to_vec() {
                        if !b.contains(&item).unwrap_or(false) {
                            eq = false;
                            break;
                        }
                    }
                    eq
                }
            }
            (PyObject::FrozenSet(a), PyObject::FrozenSet(b)) => {
                if a.len() != b.len() {
                    false
                } else {
                    let mut eq = true;
                    for item in a.to_vec() {
                        if !b.contains(&item).unwrap_or(false) {
                            eq = false;
                            break;
                        }
                    }
                    eq
                }
            }
            (PyObject::Array(a), PyObject::Array(b)) => {
                a.typecode == b.typecode && a.data == b.data
            }
            (
                PyObject::Slice {
                    start: as_,
                    stop: ae,
                    step: ap,
                },
                PyObject::Slice {
                    start: bs,
                    stop: be,
                    step: bp,
                },
            ) => as_.equals(bs)? && ae.equals(be)? && ap.equals(bp)?,
            (
                PyObject::Range {
                    start: a,
                    stop: ae,
                    step: ap,
                },
                PyObject::Range {
                    start: b,
                    stop: be,
                    step: bp,
                },
            ) => {
                // CPython's range equality: equal if same length and
                // (both empty, or same start [and stop+step for multi-
                // element ranges]). Two EMPTY ranges are equal regardless
                // of their differing start/stop/step.
                let la = crate::object::ops_contains::range_len_values(a, ae, ap);
                let lb = crate::object::ops_contains::range_len_values(b, be, bp);
                if la != lb {
                    false
                } else if la == num_bigint::BigInt::from(0)
                    || (la == num_bigint::BigInt::from(1) && a == b)
                {
                    true
                } else {
                    // CPython compares length, start, and step — NOT stop
                    // (range(0, 5, 2) == range(0, 6, 2), both [0, 2, 4]).
                    a == b && ap == bp
                }
            }
            (
                PyObject::CompiledRegex {
                    pattern: a,
                    flags: af,
                    ..
                },
                PyObject::CompiledRegex {
                    pattern: b,
                    flags: bf,
                    ..
                },
            ) => a == b && af == bf,
            // Code objects compare structurally — real CPython's
            // `code.__eq__` does (test_codeop asserts `compile_command(src)
            // == compile(src, ...)`). Two separately compiled-but-identical
            // code objects must be equal even though their `const_cache`
            // (excluded from CodeObject's PartialEq) differs.
            (PyObject::Code(a), PyObject::Code(b)) => **a == **b,
            // Reference-identity types (matching the identity-based hash
            // above): equal iff it's really the same underlying object.
            (PyObject::Function(_), PyObject::Function(_))
            | (PyObject::BuiltinFunction { .. }, PyObject::BuiltinFunction { .. })
            | (PyObject::BuiltinMethod { .. }, PyObject::BuiltinMethod { .. })
            | (PyObject::Type { .. }, PyObject::Type { .. })
            | (PyObject::Module { .. }, PyObject::Module { .. }) => {
                std::ptr::eq(self as *const PyObject, &*other as *const PyObject)
            }
            (PyObject::BoundMethod { func: a_func, self_obj: a_self }, PyObject::BoundMethod { func: b_func, self_obj: b_self }) => {
                (a_func.is(b_func) || a_func.equals(b_func).unwrap_or(false))
                    && (a_self.is(b_self) || a_self.equals(b_self).unwrap_or(false))
            }
            _ => false,
        };
        Ok(result)
    }
}

