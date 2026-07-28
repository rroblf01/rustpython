// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the foundational
// object-representation types: the FxHasher/TypeDict/AttrMap map
// infrastructure, PyObjectRef (the tagged-pointer-style enum) and its impl,
// and SmallStr/RefOrOwned helpers.
use super::*;
use std::rc::Rc;
use std::cell::RefCell;
use std::fmt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use num_bigint::{BigInt, Sign};
use num_traits::{Zero, One, ToPrimitive, Signed};
use crate::interner::{self, StrId};
use crate::bytecode::CodeObject;
use crate::modules::*;

/// A minimal FxHash-style hasher (the same public-domain algorithm used by
/// `rustc-hash`/Firefox's own internals: rotate-xor-multiply per word, no
/// cryptographic mixing at all) — used in place of `std::collections::
/// HashMap`'s default `RandomState`/SipHash for internal, never-untrusted-
/// input lookup tables (`Module`/`Type` dicts, see `TypeDict` below).
/// SipHash's DoS-resistance is designed for maps keyed by attacker-
/// controlled data (e.g. HTTP header names); a class's own method/attribute
/// names are never that, so paying SipHash's per-hash setup/mixing cost
/// buys nothing here — hashing a `StrId` (a plain `u32`) should be close to
/// free, not go through several rounds of a cryptographic-strength mix.
/// Implemented locally rather than adding a `rustc-hash` dependency since
/// the whole algorithm is ~15 lines and this project already prefers
/// reimplementing over adding crates where practical (see CLAUDE.md).
#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl std::hash::Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut hash = self.hash;
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let word = u64::from_ne_bytes(chunk.try_into().unwrap());
            hash = (hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            let word = u64::from_ne_bytes(buf);
            hash = (hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
        }
        self.hash = hash;
    }
    fn write_u8(&mut self, i: u8) { self.write_u64(i as u64); }
    fn write_u16(&mut self, i: u16) { self.write_u64(i as u64); }
    fn write_u32(&mut self, i: u32) { self.write_u64(i as u64); }
    fn write_u64(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(FX_SEED);
    }
    fn write_usize(&mut self, i: usize) { self.write_u64(i as u64); }
    fn finish(&self) -> u64 { self.hash }
}

/// `BuildHasher` for `FxHasher` — pass this as a `HashMap`'s 3rd type
/// parameter (`HashMap<K, V, FxBuildHasher>`) to opt out of SipHash.
pub type FxBuildHasher = std::hash::BuildHasherDefault<FxHasher>;

/// `Module`/`Type` dicts' concrete map type: `StrId`-keyed (see
/// `str_map_to_typedict`'s doc comment) AND `FxHasher`-hashed. `StrId` is
/// just a `u32`, so `FxHasher`'s single rotate-xor-multiply per lookup is
/// both correct (see `FxHasher`'s own doc comment on why SipHash's
/// DoS-resistance is wasted effort here) and meaningfully cheaper per
/// attribute/method lookup than SipHash's multi-round mixing.
pub type TypeDict = HashMap<StrId, PyObjectRef, FxBuildHasher>;

/// DictMap trait: provides get_str/insert_str/contains_key_str for HashMap and InternedMap.
pub trait DictMap {
    fn get_str(&self, name: &str) -> Option<&PyObjectRef>;
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef>;
    fn contains_key_str(&self, name: &str) -> bool;
}
impl DictMap for HashMap<String, PyObjectRef> {
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> { self.get(name) }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> { self.insert(name.to_string(), val) }
    fn contains_key_str(&self, name: &str) -> bool { self.contains_key(name) }
}
/// `Module`/`Type` dicts' storage — real hashing (unlike `AttrMap`'s linear
/// scan) still pays for itself here since a module/class can hold many
/// entries (every function `os` exports, every method a big class
/// defines), but each entry no longer needs its own heap-allocated
/// `String` key: `StrId` is a small `Copy` integer (already interned by
/// every `.insert_str()`/`.get_str()` call below), so this cuts the
/// redundant per-entry allocation for repeated names like `"__init__"`/
/// `"__repr__"` across every class/module that defines them, and hashing
/// a `u32` is far cheaper than hashing a variable-length string.
impl<S: std::hash::BuildHasher> DictMap for HashMap<StrId, PyObjectRef, S> {
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> { self.get(&interner::intern(name)) }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> { self.insert(interner::intern(name), val) }
    fn contains_key_str(&self, name: &str) -> bool { self.contains_key(&interner::intern(name)) }
}

/// Convert a `HashMap<String, V>` to `HashMap<StrId, V>` (default hasher)
/// by interning all keys — a general-purpose helper used anywhere a
/// String-keyed map needs to become StrId-keyed (the VM's own top-level
/// `builtins`/`exec()`-scratch-globals maps, in `vm.rs`), NOT only for
/// `Module`/`Type` dict construction. See `str_map_to_typedict` for the
/// `TypeDict` (fast-hasher)-targeting variant used specifically there.
pub(crate) fn str_map_to_strid_map<V>(map: HashMap<String, V>) -> HashMap<StrId, V> {
    map.into_iter().map(|(k, v)| (interner::intern(&k), v)).collect()
}

/// Same as `str_map_to_strid_map`, but targets `TypeDict`'s shape
/// (`StrId`-keyed AND `FxHasher`-hashed) directly — the conversion boundary
/// used when a `HashMap<String, PyObjectRef>` built by a `create_X_dict()`-
/// style function (unchanged, still string-keyed — there's no need to touch
/// the ~1800 call sites across `src/modules/*.rs` that build these) is
/// stored into a real `PyObject::Module`/`PyObject::Type`'s `dict` field.
pub(crate) fn str_map_to_typedict<V>(map: HashMap<String, V>) -> HashMap<StrId, V, FxBuildHasher> {
    map.into_iter().map(|(k, v)| (interner::intern(&k), v)).collect()
}

/// Dense, linear-scan small map used for `PyObject::Instance.dict`.
///
/// Real Python instances typically hold only a handful of attributes (a
/// class with `__init__(self, x, y)` has exactly 2). A `HashMap`'s fixed
/// per-map overhead — an empty struct costs 48 bytes, and inserting just 2
/// entries already forces a table allocation rounded up to capacity 3 —
/// costs more at this scale than a flat `Vec` scan saves in lookup speed.
/// Linear scan over a handful of short strings is also cache-friendlier
/// than hashing + a pointer-chasing bucket lookup. Kept HashMap-API-shaped
/// (`get`/`insert`/`remove`/`contains_key`/`keys`/`iter`/`entry`/...) so it
/// drops into the existing call sites with minimal churn. Module/Type dicts
/// (which can hold many entries — every class method, every module export)
/// deliberately keep `HashMap`, where hashing actually pays for itself.
#[derive(Debug, Clone, Default)]
pub struct AttrMap {
    entries: Vec<(StrId, PyObjectRef)>,
}

impl AttrMap {
    pub fn new() -> Self {
        AttrMap { entries: Vec::new() }
    }

    fn position(&self, key: StrId) -> Option<usize> {
        self.entries.iter().position(|(k, _)| *k == key)
    }

    pub fn get(&self, key: &str) -> Option<&PyObjectRef> {
        let sid = interner::intern(key);
        self.position(sid).map(|i| &self.entries[i].1)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut PyObjectRef> {
        let sid = interner::intern(key);
        match self.position(sid) {
            Some(i) => Some(&mut self.entries[i].1),
            None => None,
        }
    }

    pub fn insert(&mut self, key: String, value: PyObjectRef) -> Option<PyObjectRef> {
        let sid = interner::intern(&key);
        match self.position(sid) {
            Some(i) => Some(std::mem::replace(&mut self.entries[i].1, value)),
            None => {
                self.entries.push((sid, value));
                None
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<PyObjectRef> {
        let sid = interner::intern(key);
        self.position(sid).map(|i| self.entries.remove(i).1)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        let sid = interner::intern(key);
        self.position(sid).is_some()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| interner::lookup_str(*k))
    }

    pub fn values(&self) -> impl Iterator<Item = &PyObjectRef> {
        self.entries.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut PyObjectRef> {
        self.entries.iter_mut().map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &PyObjectRef)> {
        self.entries.iter().map(|(k, v)| (interner::lookup_str(*k), v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut PyObjectRef)> {
        self.entries.iter_mut().map(|(k, v)| (interner::lookup_str(*k), v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear()
    }

    pub fn entry(&mut self, key: String) -> AttrEntry<'_> {
        let sid = interner::intern(&key);
        match self.position(sid) {
            Some(i) => AttrEntry::Occupied(&mut self.entries[i].1),
            None => AttrEntry::Vacant(self, sid),
        }
    }
}

impl Extend<(String, PyObjectRef)> for AttrMap {
    fn extend<I: IntoIterator<Item = (String, PyObjectRef)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<const N: usize> From<[(String, PyObjectRef); N]> for AttrMap {
    fn from(arr: [(String, PyObjectRef); N]) -> Self {
        let mut m = AttrMap::new();
        for (k, v) in arr {
            m.insert(k, v);
        }
        m
    }
}

impl FromIterator<(String, PyObjectRef)> for AttrMap {
    fn from_iter<I: IntoIterator<Item = (String, PyObjectRef)>>(iter: I) -> Self {
        let mut m = AttrMap::new();
        for (k, v) in iter {
            m.insert(k, v);
        }
        m
    }
}

impl IntoIterator for AttrMap {
    type Item = (StrId, PyObjectRef);
    type IntoIter = std::vec::IntoIter<(StrId, PyObjectRef)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a> IntoIterator for &'a AttrMap {
    type Item = (&'a str, &'a PyObjectRef);
    type IntoIter = Box<dyn Iterator<Item = (&'a str, &'a PyObjectRef)> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

pub enum AttrEntry<'a> {
    Occupied(&'a mut PyObjectRef),
    Vacant(&'a mut AttrMap, StrId),
}

impl<'a> AttrEntry<'a> {
    pub fn or_insert_with<F: FnOnce() -> PyObjectRef>(self, f: F) -> &'a mut PyObjectRef {
        match self {
            AttrEntry::Occupied(v) => v,
            AttrEntry::Vacant(map, sid) => {
                map.entries.push((sid, f()));
                &mut map.entries.last_mut().unwrap().1
            }
        }
    }
}

impl DictMap for AttrMap {
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> { self.get(name) }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> { self.insert(name.to_string(), val) }
    fn contains_key_str(&self, name: &str) -> bool { self.contains_key(name) }
}

pub type BuiltinFunc = fn(&[PyObjectRef]) -> PyResult<PyObjectRef>;

pub static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static IMM_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Temporary owned, Rc-held, or directly-borrowed PyObject
pub enum RefOrOwned<'a> {
    Ref(std::cell::Ref<'a, PyObject>),
    Owned(PyObject),
    Borrow(&'a PyObject),
}

impl<'a> std::ops::Deref for RefOrOwned<'a> {
    type Target = PyObject;
    fn deref(&self) -> &PyObject {
        match self {
            RefOrOwned::Ref(r) => &**r,
            RefOrOwned::Owned(o) => o,
            RefOrOwned::Borrow(b) => b,
        }
    }
}

/// Inline storage for short strings (<16 bytes).
/// Avoids heap allocation and Rc overhead for small strings.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SmallStr {
    data: [u8; 15],
    len: u8,
}

impl SmallStr {
    pub fn new(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > 15 {
            return None;
        }
        let mut data = [0u8; 15];
        data[..bytes.len()].copy_from_slice(bytes);
        Some(SmallStr { data, len: bytes.len() as u8 })
    }

    pub fn as_str(&self) -> &str {
        // We only store valid UTF-8 (checked in `new()` via `s.as_bytes()`)
        std::str::from_utf8(&self.data[..self.len as usize])
            .expect("SmallStr: invalid UTF-8 data")
    }

}

#[derive(Clone)]
#[repr(C)]
pub enum PyObjectRef {
    SmallInt(i64),
    SmallBool(bool),
    SmallFloat(f64),     // Inline f64 — avoids Rc + heap alloc
    SmallStr(SmallStr),  // Inline short string (<16 bytes)
    None,
    Mut(Rc<RefCell<PyObject>>),  // Mutable: List, Dict, Set, Instance
    Imm(Rc<RefCell<PyObject>>),  // Immutable: Int, Str, Float, Tuple, Bytes, Code, Function
}

impl PyObjectRef {
    /// Create a MUTABLE PyObjectRef (for List, Dict, Set, Instance)
    pub fn new(obj: PyObject) -> Self {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        let trackable = crate::cycle_gc::is_trackable(&obj);
        let rc = Rc::new(RefCell::new(obj));
        if trackable {
            crate::cycle_gc::track(&rc);
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

    pub fn borrow(&self) -> RefOrOwned<'_> {
        match self {
            PyObjectRef::SmallInt(n) => RefOrOwned::Owned(PyObject::Int(BigInt::from(*n))),
            PyObjectRef::SmallBool(b) => RefOrOwned::Owned(PyObject::Bool(*b)),
            PyObjectRef::SmallFloat(f) => RefOrOwned::Owned(PyObject::Float(*f)),
            PyObjectRef::SmallStr(s) => RefOrOwned::Owned(PyObject::Str(compact_str::CompactString::from(s.as_str()))),
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

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, PyObject> {
        match self {
            PyObjectRef::Mut(rc) => {
                let result = rc.try_borrow_mut();
                match result {
                    Ok(guard) => guard,
                    Err(_) => {
                        use std::io::Write;
                        let _ = std::io::stderr().write_all(b"RefCell CONFLICT - borrow_mut while borrowed\n");
                        let _ = std::io::stderr().flush();
                        panic!("RefCell already borrowed");
                    }
                }
            }
            _ => panic!("borrow_mut called on non-Mut value"),
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
            PyObjectRef::Imm(_) | PyObjectRef::Mut(_) => matches!(&*self.borrow(), PyObject::Float(_)),
            _ => false,
        }
    }

    pub fn is(&self, other: &PyObjectRef) -> bool {
        match (self, other) {
            (PyObjectRef::SmallInt(a), PyObjectRef::SmallInt(b)) => a == b,
            (PyObjectRef::SmallBool(a), PyObjectRef::SmallBool(b)) => a == b,
            (PyObjectRef::SmallFloat(a), PyObjectRef::SmallFloat(b)) => a.to_bits() == b.to_bits(),
            (PyObjectRef::None, PyObjectRef::None) => true,
            (PyObjectRef::Mut(a), PyObjectRef::Mut(b)) => Rc::ptr_eq(a, b),
            (PyObjectRef::Imm(a), PyObjectRef::Imm(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn repr(&self) -> String {
        // Guards against a genuinely self-referential container (`d = {};
        // d[42] = d.values(); repr(d)` — real trigger: CPython's own
        // `test_dictviews.py`'s `test_recursive_repr`). Real CPython's
        // `repr()` has proper cycle detection (`Py_ReprEnter`/`Py_ReprLeave`,
        // tracking object IDENTITY to print `[...]`/`{...}` for a repeat);
        // this is a simpler DEPTH-based approximation — good enough to turn
        // an unconditional stack-overflow crash into a plain "..." — since
        // `.repr()` returns a bare `String`, not a `PyResult`, so there's
        // nowhere to propagate a real `RecursionError` from here without a
        // much larger signature change across every call site.
        thread_local! {
            static REPR_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
        }
        let depth = REPR_DEPTH.with(|c| { let d = c.get() + 1; c.set(d); d });
        if depth > 200 {
            REPR_DEPTH.with(|c| c.set(c.get() - 1));
            return "...".to_string();
        }
        let result = self.repr_inner();
        REPR_DEPTH.with(|c| c.set(c.get() - 1));
        result
    }

    fn repr_inner(&self) -> String {
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
            if let Ok(result) = call_bound_method(f, self.clone(), vec![]) {
                return result.str();
            }
        }
        if let Some(native) = native_backing_of(self) {
            return native.repr();
        }
        // For container types, clone elements before calling their .repr()
        // so the RefCell borrow on the container is released first. This
        // prevents RefCell panics when an element's __repr__ (via Python
        // code) tries to mutate the same container (e.g. mylist.pop()
        // during repr(mylist)).
        let obj = self.borrow();
        match &*obj {
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
                let parts: Vec<String> = items.iter().map(|(k, v)| format!("{}: {}", k.repr(), v.repr())).collect();
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
                let parts: Vec<String> = items.iter().map(|x| x.repr()).collect();
                format!("frozenset({{{}}})", parts.join(", "))
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
                PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__str__").or_else(|| lookup_dunder_via_mro(typ, "__repr__")),
                _ => None,
            }
        };
        if let Some(f) = str_func {
            if let Ok(result) = call_bound_method(f, self.clone(), vec![]) {
                return result.str();
            }
        }
        if let Some(native) = native_backing_of(self) {
            return native.str();
        }
        self.borrow().str()
    }
    pub fn truthy(&self) -> bool {
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
    pub fn hash(&self) -> PyResult<usize> {
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
                if f.is_nan() {
                    Ok(0)
                } else if f.fract() == 0.0 && f.is_finite() && f.abs() < 1e18 {
                    Ok(hash_bigint(&BigInt::from(*f as i64)))
                } else {
                    let bits = f.to_bits();
                    Ok(bits as usize ^ (bits >> 32) as usize)
                }
            }
            PyObjectRef::SmallStr(s) => {
                let bytes = s.as_str().as_bytes();
                let mut h: usize = 0;
                for &b in bytes { h = h.wrapping_mul(31).wrapping_add(b as usize); }
                Ok(h)
            }
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
                        let result = call_bound_method(f, self.clone(), vec![])?;
                        let n = result.borrow();
                        return if let PyObject::Int(i) = &*n {
                            let bytes = i.to_signed_bytes_le();
                            let mut h: usize = 0;
                            for (j, &b) in bytes.iter().enumerate() {
                                h ^= (b as usize) << ((j % std::mem::size_of::<usize>()) * 8);
                            }
                            Ok(h)
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
        let depth = EQUALS_DEPTH.with(|c| { let d = c.get() + 1; c.set(d); d });
        if depth > 500 {
            EQUALS_DEPTH.with(|c| c.set(c.get() - 1));
            return Err(PyError::recursion_error("maximum recursion depth exceeded in comparison"));
        }
        let result = self.equals_inner(other);
        EQUALS_DEPTH.with(|c| c.set(c.get() - 1));
        result
    }

    fn equals_inner(&self, other: &PyObjectRef) -> PyResult<bool> {
        if let (Some(ai), Some(bi)) = (self.as_i64(), other.as_i64()) {
            return Ok(ai == bi);
        }
        // Fast path for inline floats
        if let (PyObjectRef::SmallFloat(a), PyObjectRef::SmallFloat(b)) = (self, other) {
            return Ok(a == b);
        }
        // Fast path for inline strings
        if let (PyObjectRef::SmallStr(a), PyObjectRef::SmallStr(b)) = (self, other) {
            return Ok(a.as_str() == b.as_str());
        }
        // Custom __eq__ dispatch needs THIS PyObjectRef's own identity (a
        // real Rc clone) passed as `self` — PyObject::equals below (called
        // via `.borrow()`) only has `&PyObject`, with no way to recover the
        // Rc it lives in, so it used to reconstruct a throwaway
        // `PyObjectRef::new(self.clone())` just to have something to pass
        // as `self`. That throwaway has a *different* identity than the
        // real object, so e.g. `object`'s default (identity-based) __eq__
        // always returned false — even for `x == x` on the very same
        // instance (surfaced by enum member comparisons: `Color.RED ==
        // Color.RED` came out False). Doing the mro lookup and call here,
        // with the real `self`, fixes that at the root.
        let typ = if let PyObject::Instance { typ, .. } = &*self.borrow() { Some(typ.clone()) } else { None };
        if let Some(typ) = typ {
            if let Some(f) = lookup_dunder_via_mro(&typ, "__eq__") {
                let result = call_bound_method(f, self.clone(), vec![other.clone()])?;
                if !is_not_implemented(&result) {
                    return Ok(result.truthy());
                }
            }
        }
        // For container types, clone elements before element-wise comparison
        // so the RefCell borrow on the container is released first. This
        // prevents RefCell panics when an element's __eq__ mutates the same
        // container during comparison (e.g. lst.index(lst) with custom __eq__
        // that calls lst.clear()).
        let self_items = match &*self.borrow() {
            PyObject::List(items) => Some(items.clone()),
            PyObject::Tuple(items) => Some(items.clone()),
            _ => None,
        };
        if let Some(my_items) = self_items {
            let other_items = match &*other.borrow() {
                PyObject::List(items) => Some(items.clone()),
                PyObject::Tuple(items) => Some(items.clone()),
                _ => None,
            };
            if let Some(other_items) = other_items {
                if my_items.len() != other_items.len() { return Ok(false); }
                for (x, y) in my_items.iter().zip(other_items.iter()) {
                    if !x.equals(y)? { return Ok(false); }
                }
                return Ok(true);
            }
        }
        // Handle Dict comparison: clone items and keys to avoid RefCell conflicts
        let self_dict = match &*self.borrow() {
            PyObject::Dict(d) => Some(d.items()),
            _ => None,
        };
        if let Some(my_items) = self_dict {
            let other_dict = match &*other.borrow() {
                PyObject::Dict(d) => Some(d.items()),
                _ => None,
            };
            if let Some(other_items) = other_dict {
                if my_items.len() != other_items.len() { return Ok(false); }
                for (k, va) in my_items {
                    let mut found = None;
                    for (ok, ov) in &other_items {
                        if ok.equals(&k)? { found = Some(ov); break; }
                    }
                    match found {
                        Some(vb) => { if !va.equals(vb)? { return Ok(false); } }
                        None => { return Ok(false); }
                    }
                }
                return Ok(true);
            }
        }
        self.borrow().equals(other)
    }
    pub fn get_type_name(&self) -> String { self.borrow().type_name() }

    pub fn get_id(&self) -> usize {
        match self {
            PyObjectRef::Mut(rc) => Rc::as_ptr(rc) as *const PyObject as usize,
            PyObjectRef::Imm(rc) => &*rc as *const _ as usize,
            inline => inline as *const PyObjectRef as usize,
        }
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
