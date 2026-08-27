// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the foundational
// object-representation types: the FxHasher/TypeDict/AttrMap map
// infrastructure, PyObjectRef (the tagged-pointer-style enum) and its impl,
// and SmallStr/RefOrOwned helpers.
use super::*;
use crate::bytecode::CodeObject;
use crate::interner::{self, StrId};
use crate::modules::*;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    fn write_u8(&mut self, i: u8) {
        self.write_u64(i as u64);
    }
    fn write_u16(&mut self, i: u16) {
        self.write_u64(i as u64);
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(FX_SEED);
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
    fn finish(&self) -> u64 {
        self.hash
    }
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
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> {
        self.get(name)
    }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> {
        self.insert(name.to_string(), val)
    }
    fn contains_key_str(&self, name: &str) -> bool {
        self.contains_key(name)
    }
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
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> {
        self.get(&interner::intern(name))
    }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> {
        self.insert(interner::intern(name), val)
    }
    fn contains_key_str(&self, name: &str) -> bool {
        self.contains_key(&interner::intern(name))
    }
}

/// Convert a `HashMap<String, V>` to `HashMap<StrId, V>` (default hasher)
/// by interning all keys — a general-purpose helper used anywhere a
/// String-keyed map needs to become StrId-keyed (the VM's own top-level
/// `builtins`/`exec()`-scratch-globals maps, in `vm.rs`), NOT only for
/// `Module`/`Type` dict construction. See `str_map_to_typedict` for the
/// `TypeDict` (fast-hasher)-targeting variant used specifically there.
pub(crate) fn str_map_to_strid_map<V>(map: HashMap<String, V>) -> HashMap<StrId, V> {
    map.into_iter()
        .map(|(k, v)| (interner::intern(&k), v))
        .collect()
}

/// Same as `str_map_to_strid_map`, but targets `TypeDict`'s shape
/// (`StrId`-keyed AND `FxHasher`-hashed) directly — the conversion boundary
/// used when a `HashMap<String, PyObjectRef>` built by a `create_X_dict()`-
/// style function (unchanged, still string-keyed — there's no need to touch
/// the ~1800 call sites across `src/modules/*.rs` that build these) is
/// stored into a real `PyObject::Module`/`PyObject::Type`'s `dict` field.
pub(crate) fn str_map_to_typedict<V>(map: HashMap<String, V>) -> HashMap<StrId, V, FxBuildHasher> {
    map.into_iter()
        .map(|(k, v)| (interner::intern(&k), v))
        .collect()
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
        AttrMap {
            entries: Vec::new(),
        }
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
        self.entries
            .iter()
            .map(|(k, v)| (interner::lookup_str(*k), v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut PyObjectRef)> {
        self.entries
            .iter_mut()
            .map(|(k, v)| (interner::lookup_str(*k), v))
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
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> {
        self.get(name)
    }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> {
        self.insert(name.to_string(), val)
    }
    fn contains_key_str(&self, name: &str) -> bool {
        self.contains_key(name)
    }
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
        Some(SmallStr {
            data,
            len: bytes.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        // We only store valid UTF-8 (checked in `new()` via `s.as_bytes()`)
        std::str::from_utf8(&self.data[..self.len as usize]).expect("SmallStr: invalid UTF-8 data")
    }
}

// ---- id() infrastructure ----
//
// `get_id()` (backing the `id()` builtin) used to just return the raw
// pointer address for `Mut`/`Imm` (`Rc::as_ptr`), and the address of
// whatever local/temporary `PyObjectRef` binding happened to hold an inline
// value (`SmallInt`/`SmallBool`/`SmallFloat`/`None`) for everything else.
// Both were broken in ways real Python code (including CPython's own test
// suite) actively relies on NOT being broken:
// - `id(5) == id(5)` was `False` — TWO SEPARATE calls to `id()` on the same
//   int VALUE gave different results, since each call's `args[0]` lived at
//   a different stack address. This directly contradicts `PyObjectRef::is`,
//   which already (deliberately, predating this fix) treats ALL
//   `SmallInt`/`SmallBool`/`SmallFloat`/`None` values as identity-equal by
//   VALUE (i.e. `1000 is 1000` is already `True` here, unlike real
//   CPython's -5..256-only small-int cache) — `id()` must agree with `is()`
//   for the exact same reason CPython's own docs promise it does
//   (`a is b` if and only if `id(a) == id(b)`).
// - Heap-allocated (`Mut`/`Imm`) object ids were raw allocator addresses,
//   which are NOT guaranteed to increase monotonically with allocation
//   order (confirmed: `sorted([Foo() for _ in range(5)], key=id)` does not
//   reproduce creation order) — real CPython's own default allocator
//   happens to correlate address with allocation order closely enough in
//   practice that a non-trivial slice of CPython's OWN test suite relies on
//   it incidentally (real trigger: `test_compare.py`'s
//   `create_sorted_instances` helper, which does exactly this sort-by-id).
//
// Fixed with two matching pieces:
// 1. Inline values get a DETERMINISTIC, value-derived id (a tagged
//    encoding, distinguishing None/bool/int/float from each other and from
//    real heap addresses) — same value now always yields the same id,
//    matching `is()` exactly, with no allocation involved at all.
// 2. Heap (`Mut`/`Imm`) objects get a per-thread MONOTONICALLY INCREASING
//    counter value, assigned the first time `get_id()` is actually called
//    on a given allocation (looked up/cached in a side table keyed by the
//    Rc's raw address, since there's nowhere on `PyObject` itself to stash
//    an id without growing every single object in the interpreter — this
//    project has spent real effort shrinking that enum). This makes id()
//    monotonic with first-QUERY order rather than true allocation order,
//    but for the extremely common "sort a just-built list by id() to
//    recover creation order" idiom (what CPython's own test suite does),
//    the first query for each element happens during that very sort's
//    key-computation pass, in original (creation) list order — so the two
//    orders coincide for exactly the cases that matter in practice.
//    Known, accepted limitation: if the SAME address is reused for a NEW
//    allocation after the OLD occupant was dropped (ordinary allocator
//    behavior), the new object inherits the old occupant's cached id
//    instead of getting a fresh, later one — harmless for identity
//    (the old occupant is dead, so no live collision), but the new object's
//    id may not reflect ITS true creation order relative to other objects
//    allocated in between. Not fixed: would need a `Drop` hook to evict
//    stale entries, a larger change to how `PyObjectRef::Mut`/`Imm` wrap
//    `Rc`, not attempted here. The side table itself also grows without
//    bound for the lifetime of the process (same accepted tradeoff already
//    made for `PRIMITIVE_TYPE_CACHE`).
mod object_id {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    // Tag bits occupy the top byte of the id space, keeping the remaining
    // 56 bits free for the actual payload/counter — real heap pointers on
    // every platform this targets don't set bits this high, so inline-value
    // ids can't collide with heap-object ids (collisions BETWEEN the four
    // inline-value tags are impossible by construction; a same-value
    // collision WITHIN one tag is exactly the point, not a bug).
    const TAG_NONE: usize = 0x10 << 56;
    const TAG_BOOL: usize = 0x11 << 56;
    const TAG_INT: usize = 0x12 << 56;
    const TAG_FLOAT: usize = 0x13 << 56;

    pub(super) fn none_id() -> usize {
        TAG_NONE
    }
    pub(super) fn bool_id(b: bool) -> usize {
        TAG_BOOL | (b as usize)
    }
    pub(super) fn int_id(n: i64) -> usize {
        TAG_INT | ((n as u64 as usize) & 0x00ff_ffff_ffff_ffff)
    }
    pub(super) fn float_id(bits: u64) -> usize {
        TAG_FLOAT | ((bits as usize) & 0x00ff_ffff_ffff_ffff)
    }

    thread_local! {
        static NEXT_HEAP_ID: Cell<usize> = const { Cell::new(1) };
        static HEAP_ID_TABLE: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
    }

    /// Stable, monotonically-increasing (in first-query order) id for a
    /// heap allocation, keyed by its raw address — see this module's own
    /// doc comment (above, on the `object_id` module) for the full
    /// rationale and accepted limitations.
    pub(super) fn heap_id(addr: usize) -> usize {
        HEAP_ID_TABLE.with(|t| {
            if let Some(&id) = t.borrow().get(&addr) {
                return id;
            }
            let id = NEXT_HEAP_ID.with(|c| {
                let v = c.get();
                c.set(v + 1);
                v
            });
            t.borrow_mut().insert(addr, id);
            id
        })
    }
}

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

thread_local! {
    static NATIVE_DISPATCH_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

/// Guards `call_bound_method`'s and `builtin_call`'s `PyObject::Function`
/// arms — both spin up a BRAND NEW disposable `VirtualMachine` (with its own
/// fresh, always-zero `self.frames`) for every single nested native-dispatch
/// call (any dunder invoked from native code: `__call__`, `__repr__`,
/// `__eq__`, ...), so `vm.rs`'s own `call_function` recursion-limit check
/// (`self.frames.len() >= self.recursion_limit`) NEVER trips for recursion
/// that flows through this path — each nesting level resets that counter to
/// zero right when a fresh VM is constructed, while the REAL native (Rust)
/// call stack keeps growing underneath, completely unbounded, until it
/// overflows for real: a hard process abort, not a catchable
/// `RecursionError`. Confirmed via CPython's own `test_descr.py`'s
/// `test_recursive_call` (`A.__call__ = A()`, then `A()()` — a textbook
/// infinite `__call__` cycle real Python catches with `RecursionError`,
/// which this interpreter instead crashed on outright). This guard is a
/// SEPARATE thread-local counter from any specific VM's own frame count —
/// tracking nesting depth across ALL disposable-VM dispatches regardless of
/// which of the two call sites (or how many alternating VMs) are involved.
/// Capped at 500, same as the `EQUALS_DEPTH`/`REPR_DEPTH` guards just above
/// — each nesting level here is more stack-expensive (constructs a whole VM
/// + frame) than one ordinary Python call frame, so a smaller cap is the
/// conservative, safe choice given the same overall native stack budget.
pub(crate) struct NativeDispatchRecursionGuard;

impl NativeDispatchRecursionGuard {
    pub(crate) fn enter() -> PyResult<Self> {
        let depth = NATIVE_DISPATCH_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        if depth > 500 {
            NATIVE_DISPATCH_DEPTH.with(|c| c.set(c.get() - 1));
            return Err(PyError::recursion_error("maximum recursion depth exceeded"));
        }
        Ok(NativeDispatchRecursionGuard)
    }
}

impl Drop for NativeDispatchRecursionGuard {
    fn drop(&mut self) {
        NATIVE_DISPATCH_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

impl PyObjectRef {
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
        let typ = if let PyObject::Instance { typ, .. } = &*self.borrow() {
            Some(typ.clone())
        } else {
            None
        };
        let mut self_eq_not_impl = false;
        if let Some(typ) = typ {
            if let Some(f) = lookup_dunder_via_mro(&typ, "__eq__") {
                let result = call_bound_method(f, self.clone(), vec![other.clone()])?;
                if !is_not_implemented(&result) {
                    return Ok(result.truthy());
                }
                self_eq_not_impl = true;
            }
        }
        // Reflect to OTHER's __eq__ when self's own returned NotImplemented —
        // CPython: `'halibut' == HalibutProxy()` calls
        // `HalibutProxy.__eq__('halibut')`, AND `X() == Y()` where both have
        // __eq__ calls BOTH (`X.__eq__(Y())` then `Y.__eq__(X())`). Was
        // gated on self NOT being an Instance, so two custom-__eq__ objects
        // never reflected — the second __eq__ (and its side effects, e.g.
        // clearing a list — test_list::test_equal_operator_modifying_operand)
        // never ran.
        if self_eq_not_impl || !matches!(&*self.borrow(), PyObject::Instance { .. }) {
            if let PyObject::Instance { typ, .. } = &*other.borrow() {
                let typ = typ.clone();
                if let Some(f) = lookup_dunder_via_mro(&typ, "__eq__") {
                    let result = call_bound_method(f, other.clone(), vec![self.clone()])?;
                    if !is_not_implemented(&result) {
                        return Ok(result.truthy());
                    }
                }
            }
        }
        // Real CPython short-circuits container/slice `==` on POINTER
        // IDENTITY before comparing components — `s1 == s1` where a
        // component's `__eq__` raises (test_slice.py::test_cmp's `BadCmp`)
        // is True, not an exception. Bare Instances with a custom `__eq__`
        // do NOT short-circuit (`b == b` calls `__eq__` and may raise, as
        // the same test asserts).
        if !matches!(&*self.borrow(), PyObject::Instance { .. }) && self.is(other) {
            return Ok(true);
        }
        // For container types, clone elements before element-wise comparison
        // so the RefCell borrow on the container is released first. This
        // prevents RefCell panics when an element's __eq__ mutates the same
        // container during comparison (e.g. lst.index(lst) with custom __eq__
        // that calls lst.clear()).
        // `is_list` distinguishes the two so a `list` and a `tuple` with
        // identical elements don't compare equal — real Python NEVER treats
        // `list`/`tuple` as equal to each other regardless of content (only
        // to another value of the SAME container kind). The previous
        // version matched both into the same `Option<Vec<PyObjectRef>>`
        // without recording which kind `self` was, so `other`'s match arms
        // (also accepting either kind) let a `list` and a `tuple` slip
        // through as comparable — confirmed via CPython's own
        // `test_compare.py`, whose `assert_equality_only(t1, l1, False)`
        // failed because `(1, 2) == [1, 2]` came back `True`.
        let self_items = match &*self.borrow() {
            PyObject::List(items) => Some((true, items.clone())),
            PyObject::Tuple(items) => Some((false, items.clone())),
            _ => None,
        };
        if let Some((is_list, my_items)) = self_items {
            let other_items = match &*other.borrow() {
                PyObject::List(items) if is_list => Some(items.clone()),
                PyObject::Tuple(items) if !is_list => Some(items.clone()),
                _ => None,
            };
            if let Some(other_items) = other_items {
                if my_items.len() != other_items.len() {
                    return Ok(false);
                }
                // Real container `==` shortcuts each element via identity
                // BEFORE falling back to `__eq__` (`x is y or x == y`) —
                // needed for non-reflexive elements (`float('nan')`, or any
                // object whose `__eq__` always returns `False`, e.g. a
                // sentinel type). Was calling `x.equals(y)?` alone, so two
                // lists/tuples containing the exact SAME such object at
                // corresponding positions still compared unequal overall —
                // confirmed via CPython's own `test_contains.py::test_
                // nonreflexive` (`constructor(values) == constructor(values)`
                // for the SAME `values` tuple, containing both `nan` and a
                // never-equal sentinel, must be `True`).
                for (x, y) in my_items.iter().zip(other_items.iter()) {
                    if !(x.is(y) || x.equals(y)?) {
                        // bpo-38588: an element's __eq__ may MUTATE the
                        // lists mid-comparison (list1 = [X()]; list2 = [Y()];
                        // X.__eq__ clears list2, Y.__eq__ clears list1). Real
                        // CPython then re-compares the (now changed) live
                        // lists — both empty -> equal. Retry on the live
                        // contents.
                        let (live_a, live_b): (Option<Vec<PyObjectRef>>, Option<Vec<PyObjectRef>>) = {
                            let a = self.borrow();
                            let b = other.borrow();
                            (
                                match &*a {
                                    PyObject::List(l) => Some(l.clone()),
                                    _ => None,
                                },
                                match &*b {
                                    PyObject::List(l) => Some(l.clone()),
                                    _ => None,
                                },
                            )
                        };
                        if let (Some(la), Some(lb)) = (live_a, live_b) {
                            if la.len() != my_items.len() || lb.len() != other_items.len() {
                                // mutated: re-run equality on the live lists
                                // (both now empty in the bpo-38588 case).
                                if la.len() != lb.len() {
                                    return Ok(false);
                                }
                                let mut eq = true;
                                for (xa, xb) in la.iter().zip(lb.iter()) {
                                    if !(xa.is(xb) || xa.equals(xb)?) {
                                        eq = false;
                                        break;
                                    }
                                }
                                return Ok(eq);
                            }
                        }
                        return Ok(false);
                    }
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
                if my_items.len() != other_items.len() {
                    return Ok(false);
                }
                // Same identity-shortcut fix as the List/Tuple case just
                // above, for both keys and values.
                for (k, va) in my_items {
                    let mut found = None;
                    for (ok, ov) in &other_items {
                        if ok.is(&k) || ok.equals(&k)? {
                            found = Some(ov);
                            break;
                        }
                    }
                    match found {
                        Some(vb) => {
                            if !(va.is(vb) || va.equals(vb)?) {
                                return Ok(false);
                            }
                        }
                        None => {
                            return Ok(false);
                        }
                    }
                }
                return Ok(true);
            }
        }
        self.borrow().equals(other)
    }
    pub fn get_type_name(&self) -> String {
        self.borrow().type_name()
    }

    pub fn get_id(&self) -> usize {
        match self {
            PyObjectRef::None => object_id::none_id(),
            PyObjectRef::SmallBool(b) => object_id::bool_id(*b),
            PyObjectRef::SmallInt(n) => object_id::int_id(*n),
            PyObjectRef::SmallFloat(f) => object_id::float_id(f.to_bits()),
            // `SmallStr` keeps the OLD (already-broken, unstable) behavior
            // deliberately — unlike None/bool/int/float, `PyObjectRef::is`
            // does NOT treat two equal-content `SmallStr`s as identity-equal
            // (falls to the catch-all `_ => false`), so giving them a
            // value-derived id here would make `id(a) == id(b)` disagree
            // with `a is b` in the OTHER direction. Fixing this properly
            // needs a decision about `SmallStr` identity semantics first,
            // not just an `id()` patch — left as a known, separate,
            // lower-priority gap (real CPython doesn't guarantee string
            // interning either, so this is a quirk, not a correctness bug
            // against the language spec).
            PyObjectRef::SmallStr(_) => self as *const PyObjectRef as usize,
            PyObjectRef::Mut(rc) => object_id::heap_id(Rc::as_ptr(rc) as usize),
            PyObjectRef::Imm(rc) => object_id::heap_id(Rc::as_ptr(rc) as usize),
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
