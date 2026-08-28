// Split from src/object/core.rs — PyObjectRef and core impl.
use crate::bytecode::CodeObject;
use crate::interner::{self, StrId};
use crate::modules::*;
use crate::object::*;
use super::object_id;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
#[repr(C)]
pub enum PyObjectRef {
    SmallInt(i64),
    SmallBool(bool),
    SmallFloat(f64),    // Inline f64 — avoids Rc + heap alloc
    SmallStr(SmallStr), // Inline short string (<16 bytes)
    None,
    Mut(Rc<RefCell<PyObject>>), // Mutable: List, Dict, Set, Instance
    Imm(Rc<RefCell<PyObject>>), // Immutable: Int, Str, Float, Tuple, Bytes, Code, Function
}

// Identity stack of `PyObject` pointers currently being repr'd, used by
// `repr_inner`'s deque arm for real cycle detection (print `[...]` when the
// SAME deque re-enters, matching CPython's `Py_ReprEnter` behavior — a
// depth-based guard can't produce CPython's exact output for a
// self-referencing deque).
thread_local! {
    static REPR_VISITED: std::cell::RefCell<Vec<*const ()>> = std::cell::RefCell::new(Vec::new());
}

/// True if `f` is one of the per-type native `__repr__` functions
/// (`builtin_list_repr`, `builtin_float_repr`, ...). These call
/// `args[0].repr()`, so dispatching one on a subclass INSTANCE would
/// re-enter repr/str and recurse; user-defined (and type-specific native
/// like Fraction's) `__repr__` implementations format directly and must
/// still be invoked.
fn is_per_type_repr(f: &PyObjectRef) -> bool {
    match &*f.borrow() {
        PyObject::BuiltinFunction { func, .. } => {
            let p = *func as usize;
            p == crate::object::builtin_list_repr as usize
                || p == crate::object::builtin_tuple_repr as usize
                || p == crate::object::builtin_str_repr as usize
                || p == crate::object::builtin_bytes_repr as usize
                || p == crate::object::builtin_bytearray_repr as usize
                || p == crate::object::builtin_int_repr as usize
                || p == crate::object::builtin_float_repr as usize
                || p == crate::object::builtin_complex_repr as usize
                || p == crate::object::builtin_bool_repr as usize
                || p == crate::object::builtin_set_repr as usize
                || p == crate::object::builtin_frozenset_repr as usize
                || p == crate::object::builtin_slice_repr as usize
                || p == crate::object::builtin_dict_repr as usize
                || p == crate::object::builtin_deque_repr as usize
        }
        _ => false,
    }
}

impl PyObjectRef {
    /// Create a MUTABLE PyObjectRef (for List, Dict, Set, Instance)
    pub fn new(obj: PyObject) -> Self {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        let trackable = crate::cycle_gc::is_trackable(&obj);
        let rc = Rc::new(RefCell::new(obj));
        if trackable {
            crate::cycle_gc::track(&rc);
            if matches!(&*rc.try_borrow().unwrap(), PyObject::Instance { .. }) {
                crate::cycle_gc::maybe_register_finalizer(&rc);
            }
        }
        PyObjectRef::Mut(rc)
    }

    /// Create an IMMUTABLE PyObjectRef (for Int, Str, Float, etc.)
    pub fn imm(obj: PyObject) -> Self {
        IMM_COUNT.fetch_add(1, Ordering::Relaxed);
        let trackable = crate::cycle_gc::is_trackable(&obj);
        let rc = Rc::new(RefCell::new(obj));
        if trackable {
            crate::cycle_gc::track(&rc);
        }
        PyObjectRef::Imm(rc)
    }

    /// Backs `sys.getrefcount()`. This interpreter's memory model (`Rc`-based
    /// sharing, distinct from CPython's own refcounting) means the ABSOLUTE
    /// number will never match a real CPython build — but `Rc::strong_count`
    /// is a genuine, real count of live strong references to the SAME
    /// underlying allocation, so a before/after DELTA around some operation
    /// (the overwhelmingly common way real test code actually uses this
    /// function: `before = sys.getrefcount(x); ...; assertEqual(getrefcount(x),
    /// before)`) can still correctly reflect whether this interpreter itself
    /// picked up or released a reference. Inline variants (`SmallInt`/
    /// `SmallBool`/`SmallFloat`/`SmallStr`/`None`) have no `Rc` at all — report
    /// a large constant, matching CPython's own convention for small
    /// cached/immortal objects (`sys.getrefcount(1)` is always huge there too).
    /// CPython's immortal objects report refcounts ≥ 2^32-1, and
    /// test_builtin.ImmortalTests asserts getrefcount(immortal) > 2^31 (or
    /// > 2^30 on 32-bit), so the sentinel must sit above BOTH thresholds.
    pub fn strong_count(&self) -> usize {
        const IMMORTAL_SENTINEL: usize = 4_294_967_295; // 2^32 - 1
        match self {
            PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => Rc::strong_count(rc),
            _ => IMMORTAL_SENTINEL,
        }
    }

    pub fn borrow(&self) -> RefOrOwned<'_> {
        match self {
            PyObjectRef::SmallInt(n) => RefOrOwned::Owned(PyObject::Int(BigInt::from(*n))),
            PyObjectRef::SmallBool(b) => RefOrOwned::Owned(PyObject::Bool(*b)),
            PyObjectRef::SmallFloat(f) => RefOrOwned::Owned(PyObject::Float(*f)),
            PyObjectRef::SmallStr(s) => {
                RefOrOwned::Owned(PyObject::Str(compact_str::CompactString::from(s.as_str())))
            }
            PyObjectRef::None => RefOrOwned::Owned(PyObject::None),
            PyObjectRef::Mut(rc) => RefOrOwned::Ref(rc.borrow()),
            PyObjectRef::Imm(rc) => {
                // Fast path: Imm objects are never mutated, so we can skip
                // RefCell::borrow() and return a direct pointer. The RefCell
                // is still present for borrow_mut() protection, but the read
                // path avoids the atomic increment/decrement entirely.
                let ptr: *const PyObject = rc.as_ref().as_ptr();
                RefOrOwned::Borrow(unsafe { &*ptr })
            }
        }
    }

    /// Fallible version of `borrow_mut()` — returns a `TypeError` instead
    /// of panicking when called on an immutable or inline value. Use this
    /// for any code path that receives a user-provided `PyObjectRef` which
    /// might not be a heap-allocated mutable (`Mut`) object.
    pub fn try_borrow_mut(&self) -> Result<std::cell::RefMut<'_, PyObject>, PyError> {
        match self {
            PyObjectRef::Mut(rc) => match rc.try_borrow_mut() {
                Ok(guard) => Ok(guard),
                Err(_) => {
                    use std::io::Write;
                    let _ = std::io::stderr()
                        .write_all(b"RefCell CONFLICT - borrow_mut while borrowed\n");
                    let _ = std::io::stderr().flush();
                    panic!("RefCell already borrowed");
                }
            },
            PyObjectRef::Imm(_) => Err(PyError::type_error(format!(
                "'{}' object is immutable",
                self.borrow().type_name()
            ))),
            _ => Err(PyError::type_error(format!(
                "'{}' object does not support item assignment",
                self.borrow().type_name()
            ))),
        }
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, PyObject> {
        match self {
            PyObjectRef::Mut(rc) => {
                let result = rc.try_borrow_mut();
                match result {
                    Ok(guard) => guard,
                    Err(_) => {
                        use std::io::Write;
                        let t = self.borrow().type_name();
                        let _ = std::io::stderr()
                            .write_fmt(format_args!("RefCell CONFLICT - borrow_mut while borrowed on type {} addr {:p}\n", t, Rc::as_ptr(rc)));
                        let _ = std::io::stderr().flush();
                        panic!("RefCell already borrowed on {}", t);
                    }
                }
            }
            _ => panic!("borrow_mut called on non-Mut value"),
        }
    }

    /// `borrow_mut()` but only for mutable (heap `Mut`) values — returns None
    /// for immutable/inline values instead of panicking. The VM's exception
    /// chaining/traceback code must mutate a possibly-Imm raised exception
    /// (some internal error paths construct exceptions via `imm()`), so it
    /// uses this and silently skips attaching state when the value can't be
    /// mutated.
    pub fn borrow_mut_if_mut(&self) -> Option<std::cell::RefMut<'_, PyObject>> {
        match self {
            PyObjectRef::Mut(rc) => match rc.try_borrow_mut() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    use std::io::Write;
                    let _ = std::io::stderr()
                        .write_all(b"RefCell CONFLICT - borrow_mut while borrowed\n");
                    let _ = std::io::stderr().flush();
                    panic!("RefCell already borrowed");
                }
            },
            _ => None,
        }
    }

    /// Fast path: extract i64. Avoids borrow() for the inline variants —
    /// `py_int()` caches -5..=257 as boxed `PyObject::Int` rather than
    /// `SmallInt` (matching CPython's small-int cache range), so plain
    /// integer arguments in that very common range need the borrow() path.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PyObjectRef::SmallInt(n) => Some(*n),
            PyObjectRef::SmallBool(b) => Some(if *b { 1 } else { 0 }),
            PyObjectRef::SmallFloat(_) | PyObjectRef::SmallStr(_) | PyObjectRef::None => None,
            PyObjectRef::Imm(_) | PyObjectRef::Mut(_) => match &*self.borrow() {
                PyObject::Int(b) => b.to_i64(),
                _ => None,
            },
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PyObjectRef::SmallFloat(f) => Some(*f),
            PyObjectRef::SmallInt(n) => Some(*n as f64),
            PyObjectRef::SmallBool(b) => Some(if *b { 1.0 } else { 0.0 }),
            PyObjectRef::SmallStr(_) | PyObjectRef::None => None,
            PyObjectRef::Imm(_) | PyObjectRef::Mut(_) => match &*self.borrow() {
                PyObject::Int(b) => b.to_f64(),
                PyObject::Float(f) => Some(*f),
                _ => None,
            },
        }
    }

    /// True iff this value is a REAL float (`SmallFloat` or boxed
    /// `PyObject::Float`) — as opposed to merely being convertible to one via
    /// `as_f64()`, which any `Int`/`BigInt` also satisfies (lossily, via
    /// `to_f64()`, which never fails — it just loses precision or saturates
    /// to infinity for astronomically large magnitudes). `py_add`/`py_sub`/
    /// `py_mul`'s fast paths need this distinction: their float fast-path is
    /// only correct when at least one *actual* operand is a float — using
    /// `as_f64().is_some()` alone made `(2**100) * (2**100)` silently
    /// produce a `float` (with precision already lost) instead of the
    /// correct exact bigint, since a big `Int` converts "successfully" to
    /// f64 just like a real float would.
    pub fn is_float_typed(&self) -> bool {
        match self {
            PyObjectRef::SmallFloat(_) => true,
            PyObjectRef::Imm(_) | PyObjectRef::Mut(_) => {
                matches!(&*self.borrow(), PyObject::Float(_))
            }
            _ => false,
        }
    }

    pub fn is(&self, other: &PyObjectRef) -> bool {
        match (self, other) {
            (PyObjectRef::SmallInt(a), PyObjectRef::SmallInt(b)) => a == b,
            (PyObjectRef::SmallBool(a), PyObjectRef::SmallBool(b)) => a == b,
            (PyObjectRef::SmallFloat(a), PyObjectRef::SmallFloat(b)) => a.to_bits() == b.to_bits(),
            // Short strings are stored INLINE (no Rc to compare), but real
            // CPython interns short strings, so `x is y` for two equal
            // interned strings is True — mirror that here (a SmallStr is
            // inherently the "interned" form of its content).
            (PyObjectRef::SmallStr(a), PyObjectRef::SmallStr(b)) => a.as_str() == b.as_str(),
            (PyObjectRef::None, PyObjectRef::None) => true,
            (PyObjectRef::Mut(a), PyObjectRef::Mut(b)) => Rc::ptr_eq(a, b),
            (PyObjectRef::Imm(a), PyObjectRef::Imm(b)) => Rc::ptr_eq(a, b),
            (PyObjectRef::Mut(a), PyObjectRef::Imm(b)) => Rc::ptr_eq(a, b),
            (PyObjectRef::Imm(a), PyObjectRef::Mut(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn repr(&self) -> String {
        // Guard against self-referential containers (`l = [0, 1, 2, l]`;
        // `repr(l)` must print `[0, 1, 2, [...]]`, CPython's `Py_ReprEnter`
        // marker). Real CPython tracks object IDENTITY (not depth) so a
        // recursive container prints `[...]` at the exact recursion point
        // instead of expanding to depth 200 (the old depth-only
        // approximation). Only CONTAINERS are tracked — a repeated string
        // inside a list must not be marked.
        thread_local! {
            static REPR_STACK: std::cell::RefCell<Vec<usize>> =
                std::cell::RefCell::new(Vec::new());
        }
        let is_container = matches!(
            &*self.borrow(),
            PyObject::List(_)
                | PyObject::Tuple(_)
                | PyObject::Dict(_)
                | PyObject::Set(_)
                | PyObject::FrozenSet(_)
                | PyObject::Deque { .. }
        );
        if is_container {
            let id = self.get_id();
            let in_stack = REPR_STACK.with(|s| s.borrow().contains(&id));
            if in_stack {
                return match &*self.borrow() {
                    PyObject::Deque { .. } => "[...]".to_string(),
                    PyObject::Set(_) | PyObject::FrozenSet(_) => "{...}".to_string(),
                    _ => "[...]".to_string(),
                };
            }
            REPR_STACK.with(|s| s.borrow_mut().push(id));
        }
        // Depth fallback: even without identity cycles, pathological nesting
        // (a >200-deep nested list) must not overflow the Rust stack.
        thread_local! {
            static REPR_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
        }
        let depth = REPR_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        if depth > 200 {
            REPR_DEPTH.with(|c| c.set(c.get() - 1));
            if is_container {
                REPR_STACK.with(|s| {
                    s.borrow_mut().pop();
                });
            }
            return "...".to_string();
        }
        let result = self.repr_inner();
        REPR_DEPTH.with(|c| c.set(c.get() - 1));
        if is_container {
            REPR_STACK.with(|s| {
                s.borrow_mut().pop();
            });
        }
        result
    }

    fn repr_inner(&self) -> String {
        // Depth limit (mirrors CPython's Py_EnterRecursiveCall during repr):
        // deeply NESTED (non-cyclic) containers — e.g. mapping-tests'
        // test_repr_deep building 1000 levels — previously recursed without
        // bound and hung the interpreter. The per-object identity guard
        // below only catches direct cycles, so this depth cap is what
        // bounds legitimate-but-deep nesting.
        thread_local! {
            static REPR_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }
        let depth_ok = REPR_DEPTH.with(|d| {
            let n = d.get();
            if n >= 200 {
                false
            } else {
                d.set(n + 1);
                true
            }
        });
        if !depth_ok {
            return "[...]".to_string();
        }
        let result = self.repr_inner_guarded();
        REPR_DEPTH.with(|d| d.set(d.get() - 1));
        result
    }

    fn repr_inner_guarded(&self) -> String {
        // Check for __repr__ on Instance types (user-defined objects) —
        // self.borrow().repr() can't invoke a bound method (no PyObjectRef
        // handle from &PyObject), so it must be handled here instead.
        let repr_func = {
            let obj = self.borrow();
            match &*obj {
                PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__repr__"),
                _ => None,
            }
        };
        if let Some(f) = repr_func {
            if !is_per_type_repr(&f) {
                if let Ok(result) = call_bound_method(f, self.clone(), vec![]) {
                    return result.str();
                }
            }
        }
        // Native-backed Instances (list/dict/set subclasses) are handled by
        // PyObject::Instance's own repr (pyobject/repr.rs), which adds the
        // subclass prefix (e.g. set2({1,2}) vs {1,2}). The previous shortcut
        // `native.repr()` lost that prefix entirely.
        // For container types, clone elements before calling their .repr()
        // so the RefCell borrow on the container is released first. This
        // prevents RefCell panics when an element's __repr__ (via Python
        // code) tries to mutate the same container (e.g. mylist.pop()
        // during repr(mylist)).
        let obj = self.borrow();
        match &*obj {
            PyObject::Deque { data, maxlen } => {
                // Real CPython's `deque_repr` uses `Py_ReprEnter`/
                // `Py_ReprLeave` (tracking the deque object's IDENTITY, not
                // a depth count) and prints `[...]` when the SAME deque is
                // repr'd reentrantly — `d = deque(range(200)); d.append(d);
                // repr(d)` must end in `..., 199, [...]], ...`, which a
                // pure depth-based guard (the generic `REPR_DEPTH` above)
                // cannot produce.
                let ptr: Option<*const ()> = match self {
                    PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => {
                        Some(Rc::as_ptr(rc) as *const ())
                    }
                    _ => None,
                };
                let reentered = if let Some(ptr) = ptr {
                    REPR_VISITED.with(|v| {
                        let mut v = v.borrow_mut();
                        if v.contains(&ptr) {
                            true
                        } else {
                            v.push(ptr);
                            false
                        }
                    })
                } else {
                    false
                };
                if reentered {
                    return "[...]".to_string();
                }
                let cloned: Vec<PyObjectRef> = data.iter().cloned().collect();
                let maxlen_copy = *maxlen;
                drop(obj);
                let parts: Vec<String> = cloned.iter().map(|x| x.repr()).collect();
                let s = match maxlen_copy {
                    Some(n) => format!("deque([{}], maxlen={})", parts.join(", "), n),
                    None => format!("deque([{}])", parts.join(", ")),
                };
                if let Some(ptr) = ptr {
                    REPR_VISITED.with(|v| {
                        v.borrow_mut().retain(|&p| p != ptr);
                    });
                }
                s
            }
            PyObject::List(items) => {
                let cloned: Vec<PyObjectRef> = items.clone();
                drop(obj);
                let parts: Vec<String> = cloned.iter().map(|x| x.repr()).collect();
                format!("[{}]", parts.join(", "))
            }
            PyObject::Tuple(items) => {
                let cloned: Vec<PyObjectRef> = items.clone();
                drop(obj);
                let parts: Vec<String> = cloned.iter().map(|x| x.repr()).collect();
                if parts.len() == 1 {
                    format!("({},)", parts[0])
                } else {
                    format!("({})", parts.join(", "))
                }
            }
            PyObject::Dict(d) => {
                let items = d.items();
                drop(obj);
                let parts: Vec<String> = items
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            PyObject::Globals(g) => {
                let entries: Vec<(PyObjectRef, PyObjectRef)> = g
                    .borrow()
                    .iter()
                    .map(|(k, v)| (py_str(interner::lookup_str(*k)), v.clone()))
                    .collect();
                drop(obj);
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            PyObject::Set(s) => {
                let items = s.to_vec();
                drop(obj);
                if items.is_empty() {
                    "set()".to_string()
                } else {
                    let parts: Vec<String> = items.iter().map(|x| x.repr()).collect();
                    format!("{{{}}}", parts.join(", "))
                }
            }
            PyObject::FrozenSet(s) => {
                let items = s.to_vec();
                drop(obj);
                if items.is_empty() {
                    "frozenset()".to_string()
                } else {
                    let parts: Vec<String> = items.iter().map(|x| x.repr()).collect();
                    format!("frozenset({{{}}})", parts.join(", "))
                }
            }
            _ => {
                drop(obj);
                self.borrow().repr()
            }
        }
    }
    pub fn str(&self) -> String {
        // Check for __str__ on Instance types (user-defined objects)
        let str_func = {
            let obj = self.borrow();
            match &*obj {
                PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__str__")
                    .or_else(|| lookup_dunder_via_mro(typ, "__repr__")),
                _ => None,
            }
        };
        if let Some(f) = str_func {
            if !is_per_type_repr(&f) {
                if let Ok(result) = call_bound_method(f, self.clone(), vec![]) {
                    return result.str();
                }
            }
        }
        if let Some(native) = native_backing_of(self) {
            return native.str();
        }
        // Containers' str == repr (including CPython's recursive `[...]`
        // marker) — route through the recursion-guarded `repr()`, NOT the
        // bare `PyObject::str` -> `PyObject::repr` path which lacks the
        // identity-based cycle detection.
        if matches!(
            &*self.borrow(),
            PyObject::List(_)
                | PyObject::Tuple(_)
                | PyObject::Dict(_)
                | PyObject::Set(_)
                | PyObject::FrozenSet(_)
                | PyObject::Deque { .. }
        ) {
            return self.repr();
        }
        self.borrow().str()
    }
    pub fn truthy(&self) -> bool {
        if let PyObject::WeakProxy { target, .. } = &*self.borrow() {
            if let Some(rc) = target.upgrade() {
                return PyObjectRef::Imm(rc).truthy();
            } else {
                return false;
            }
        }
        match self {
            PyObjectRef::SmallInt(n) => *n != 0,
            PyObjectRef::SmallBool(b) => *b,
            PyObjectRef::SmallFloat(f) => *f != 0.0,
            PyObjectRef::SmallStr(s) => !s.as_str().is_empty(),
            PyObjectRef::None => false,
            PyObjectRef::Mut(_) | PyObjectRef::Imm(_) => {
                // Handle Instance specially so a __bool__ method sees the
                // real `self` (with its actual instance dict), not a blank
                // stand-in — PyObject::truthy() only has `&PyObject`, with
                // no way to reconstruct the original PyObjectRef/Rc identity.
                let typ_opt = if let PyObject::Instance { typ, .. } = &*self.borrow() {
                    Some(typ.clone())
                } else {
                    None
                };
                if let Some(typ) = typ_opt {
                    if let Some(f) = lookup_dunder_via_mro(&typ, "__bool__") {
                        if let Ok(result) = call_bound_method(f, self.clone(), vec![]) {
                            // Real CPython requires `__bool__` to return an
                            // actual `bool` — a class with `def __bool__(self):
                            // return self` is a real (if malformed) pattern
                            // covered by CPython's own test suite
                            // (`test_bool.test_convert_to_bool`), and real
                            // CPython raises `TypeError` immediately rather
                            // than re-evaluating the returned object's
                            // truthiness. Recursing into `.truthy()` here
                            // instead — as this used to — infinite-loops
                            // (each call returns `self` again) since this
                            // infallible method has no way to raise
                            // TypeError; `bool()` itself (`builtin_bool`)
                            // does the proper check and errors correctly,
                            // this implicit-truth-testing path just needs to
                            // terminate rather than hang.
                            if let PyObjectRef::SmallBool(b) = result {
                                return b;
                            }
                            return true;
                        }
                    }
                    if let Some(native) = native_backing_of(self) {
                        return native.truthy();
                    }
                    true
                } else {
                    self.borrow().truthy()
                }
            }
        }
    }

    /// Fallible truthiness for the explicit `if`/`while`/boolean-context
    /// paths where real CPython MUST propagate a raising `__bool__` /
    /// `__len__` (test_bool's test_interpreter_convert_to_bool_raises: a
    /// condition whose `__bool__` raises TypeError must raise, not be
    /// swallowed). The infallible `truthy()` above is intentionally kept
    /// for places that genuinely cannot error (and would hang instead).
    pub fn try_truthy(&self) -> PyResult<bool> {
        if let PyObject::WeakProxy { target, .. } = &*self.borrow() {
            if let Some(rc) = target.upgrade() {
                return PyObjectRef::Imm(rc).try_truthy();
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }
        match self {
            PyObjectRef::SmallInt(n) => Ok(*n != 0),
            PyObjectRef::SmallBool(b) => Ok(*b),
            PyObjectRef::SmallFloat(f) => Ok(*f != 0.0),
            PyObjectRef::SmallStr(s) => Ok(!s.as_str().is_empty()),
            PyObjectRef::None => Ok(false),
            PyObjectRef::Mut(_) | PyObjectRef::Imm(_) => {
                let typ_opt = if let PyObject::Instance { typ, .. } = &*self.borrow() {
                    Some(typ.clone())
                } else {
                    None
                };
                if let Some(typ) = typ_opt {
                    if let Some(f) = lookup_dunder_via_mro(&typ, "__bool__") {
                        if matches!(&*f.borrow(), PyObject::None) {
                            return Err(PyError::type_error(format!(
                                "'{}' cannot be interpreted as a boolean",
                                typ.borrow().type_name()
                            )));
                        }
                        let result = call_bound_method(f, self.clone(), vec![])?;
                        return match result {
                            PyObjectRef::SmallBool(b) => Ok(b),
                            other => Err(PyError::type_error(format!(
                                "__bool__ should return bool, returned {}",
                                other.borrow().type_name()
                            ))),
                        };
                    }
                    if let Some(f) = lookup_dunder_via_mro(&typ, "__len__") {
                        if matches!(&*f.borrow(), PyObject::None) {
                            return Err(PyError::type_error(format!(
                                "'{}' cannot be interpreted as a boolean",
                                typ.borrow().type_name()
                            )));
                        }
                        let result = call_bound_method(f, self.clone(), vec![])?;
                        let n = result.as_i64().ok_or_else(|| {
                            PyError::type_error("__len__() should return >= 0 integer")
                        })?;
                        return Ok(n != 0);
                    }
                    if let Some(native) = native_backing_of(self) {
                        return Ok(native.truthy());
                    }
                    Ok(true)
                } else {
                    Ok(self.borrow().truthy())
                }
            }
        }
    }

    pub fn hash(&self) -> PyResult<usize> {
        if let PyObject::WeakProxy { .. } = &*self.borrow() {
            return Err(PyError::type_error("unhashable type: 'weakproxy'"));
        }
        if let PyObject::WeakRef { target, hash_cache, .. } = &*self.borrow() {
            if let Some(rc) = target.upgrade() {
                let h = PyObjectRef::Imm(rc.clone()).hash()?;
                if hash_cache.borrow().is_none() {
                    *hash_cache.borrow_mut() = Some(h);
                }
                return Ok(h);
            } else {
                if let Some(h) = *hash_cache.borrow() {
                    return Ok(h);
                }
                return Err(PyError::type_error("weak object has gone away"));
            }
        }
        match self {
            // `hash_bigint` (also used by boxed `PyObject::Int`/whole-number
            // `PyObject::Float`) — MUST stay identical to those so a value
            // that happens to be inlined (small int/float) hashes the same
            // as the same value boxed, and so `1 == 1.0` (numeric-tower
            // equality) implies `hash(1) == hash(1.0)` even when either side
            // is the inline representation — otherwise `{1: 'x'}[1.0]`
            // raises `KeyError` despite `1.0 in {1: 'x'}` being True.
            PyObjectRef::SmallInt(n) => Ok(hash_bigint(&BigInt::from(*n))),
            PyObjectRef::SmallBool(b) => Ok(if *b { 1 } else { 0 }),
            PyObjectRef::SmallFloat(f) => {
                // NaN hashes to 0 (this interpreter's float values are
                // INLINE `SmallFloat`s with no per-object identity to hash —
                // `object.__hash__(nan)` mirrors this, returning
                // `args[0].hash()` for inline values). Finite values use
                // CPython's real mod-2**61-1 `_Py_HashDouble`, so
                // `hash(1.0) == hash(1)` and `hash(inf) == sys.hash_info.inf`.
                if f.is_nan() {
                    Ok(0)
                } else {
                    Ok(hash_double(*f))
                }
            }
            PyObjectRef::SmallStr(s) => Ok(py_hash_str(s.as_str())),
            PyObjectRef::None => Ok(0),
            PyObjectRef::Mut(_) | PyObjectRef::Imm(_) => {
                // For an Instance, __hash__ (including object's own default,
                // pointer-identity-based implementation) must be called
                // with the REAL self — PyObject::Instance::hash() below has
                // no access to the original Rc and reconstructs a throwaway
                // clone instead, giving a fresh (and different) address on
                // every single call. Route through here first, passing
                // `self.clone()` (a genuine Rc clone, same identity) so the
                // default hash is actually stable across calls — otherwise
                // no plain object can ever be used correctly as a dict/set
                // key or inside a tuple used as one.
                let typ = match &*self.borrow() {
                    PyObject::Instance { typ, .. } => Some(typ.clone()),
                    _ => None,
                };
                if let Some(typ) = typ {
                    if let Some(f) = lookup_dunder_via_mro(&typ, "__hash__") {
                        // `__hash__ = None` makes an instance unhashable
                        // (CPython: TypeError: unhashable type: 'H').
                        if matches!(&*f.borrow(), PyObject::None) {
                            return Err(PyError::type_error(format!(
                                "unhashable type: '{}'",
                                typ.borrow().type_name()
                            )));
                        }
                        let result = call_bound_method(f, self.clone(), vec![])?;
                        let n = result.borrow();
                        // Real Python's `__hash__` protocol: whatever `int`
                        // is returned BECOMES the hash value directly (bit
                        // pattern reinterpreted as unsigned) — the previous
                        // extraction instead XOR-folded the value's
                        // signed-little-endian BYTES together, which is not
                        // a hash of the value at all, just visibly wrong for
                        // anything beyond small positives (e.g. `-3` folds
                        // to `253` — its own raw two's-complement byte —
                        // instead of staying `-3`). Any user class with a
                        // custom `__hash__` returning a negative int (an
                        // extremely common pattern — `hash()` results are
                        // routinely negative) got a completely wrong,
                        // silently-mangled hash, breaking dict/set lookups
                        // for such objects and any invariant checks
                        // comparing `hash(x)` against a known value (e.g.
                        // `hash(Fraction(n)) == hash(n)` for an integral
                        // Fraction backed by a custom `__hash__`).
                        return if let PyObject::Int(i) = &*n {
                            let h = i.to_i64().ok_or_else(|| {
                                PyError::type_error("__hash__ result too large to fit in a C long")
                            })?;
                            Ok(h as usize)
                        } else {
                            Err(PyError::type_error("__hash__ should return an integer"))
                        };
                    }
                }
                self.borrow().hash()
            }
        }
    }
    pub fn equals(&self, other: &PyObjectRef) -> PyResult<bool> {
        // Real CPython's own container `==` has no cycle detection either —
        // comparing two reflexive structures (`x = {}; x['k'] = x`) recurses
        // until ITS OWN recursion limit trips, raising a real, catchable
        // `RecursionError` (confirmed expected behavior: CPython's own
        // `test_copy.py::test_deepcopy_reflexive_dict` explicitly asserts
        // `RecursionError` is raised for `y == x` on two such structures).
        // This interpreter's `equals()` had NO depth guard at all, so the
        // same comparison genuinely overflowed the native stack — a hard
        // process abort, not a catchable Python exception, unlike CPython's
        // own controlled failure. Mirrors the existing `REPR_DEPTH` guard's
        // shape (same thread-local counter pattern) but — since `equals`
        // already returns a `PyResult`, unlike `repr()`'s bare `String` —
        // propagates a REAL `RecursionError` instead of silently
        // approximating with a placeholder value.
        thread_local! {
            static EQUALS_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
        }
        let depth = EQUALS_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        if depth > 500 {
            EQUALS_DEPTH.with(|c| c.set(c.get() - 1));
            return Err(PyError::recursion_error(
                "maximum recursion depth exceeded in comparison",
            ));
        }
        let result = self.equals_inner(other);
        EQUALS_DEPTH.with(|c| c.set(c.get() - 1));
        result
    }
}

impl fmt::Display for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str())
    }
}

impl fmt::Debug for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.repr())
    }
}
