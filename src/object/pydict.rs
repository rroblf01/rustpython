// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds `PyDict`, the
// hash-based dict implementation backing `PyObject::Dict`.
use super::*;

thread_local! {
    static MODULE_GLOBALS_REGISTRY: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<std::cell::RefCell<std::collections::HashMap<crate::interner::StrId, PyObjectRef>>>>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

pub fn register_module_globals(name: &str, globals: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<crate::interner::StrId, PyObjectRef>>>) {
    MODULE_GLOBALS_REGISTRY.with(|m| m.borrow_mut().insert(name.to_string(), globals));
}

pub fn update_module_globals(module_name: &str, attr_name: &str, value: PyObjectRef) {
    MODULE_GLOBALS_REGISTRY.with(|m| {
        if let Some(g) = m.borrow().get(module_name) {
            g.borrow_mut()
                .insert(crate::interner::intern(attr_name), value);
        }
    });
}

// ---- PyDict: hash-based dict with arbitrary hashable keys ----
//
// Dense-array-plus-index-table design (the same shape real CPython's own
// dict uses internally) — each key/value pair is stored EXACTLY ONCE, in
// `entries`, in insertion order (`None` marks a removed/tombstone slot, so
// existing indices stay valid without shifting on removal — same reason
// CPython's own dict never shrinks its dense table on delete either).
// `index` maps a hash to the `entries` positions that share it (a
// collision chain) — it stores plain `usize` positions, not full key
// clones. The PREVIOUS design (`HashMap<hash, Vec<(PyObjectRef,
// PyObjectRef)>>` PLUS a separate `order: Vec<PyObjectRef>` for iteration
// order) stored every key TWICE — once in its hash bucket, once again in
// `order` — confirmed via direct benchmarking against real CPython: a
// 500K-entry `int -> int` dict used 2.6x MORE memory here than in CPython,
// the single largest memory gap found in that comparison. This design
// keeps the exact same public API/behavior (insertion-order iteration,
// re-assigning a key doesn't move it, `O(1)`-amortized get/set/remove) —
// only the internal storage shape changed.
#[derive(Clone)]
pub struct PyDict {
    entries: Vec<Option<(PyObjectRef, PyObjectRef)>>,
    indices: Vec<u32>,
    size: usize,
    pub instance_ref: Option<PyObjectRef>,
    /// Bumped on every structural mutation (insert/update/remove/clear/
    /// rehash). Used by the reentrancy-safe wrappers below (`pydict_safe_set`/
    /// `pydict_safe_get_or_insert`) to detect whether a probed slot/entry
    /// index computed against an earlier snapshot is still valid, or must be
    /// recomputed because a reentrant callback (e.g. a key's `__eq__`
    /// calling `d.clear()`) mutated the dict in the meantime.
    version: u64,
}

impl PyDict {
    pub fn new() -> Self {
        PyDict {
            entries: Vec::new(),
            indices: Vec::new(),
            size: 0,
            instance_ref: None,
            version: 0,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.indices.clear();
        self.size = 0;
        self.version = self.version.wrapping_add(1);
    }
    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    fn mask(&self) -> usize {
        self.indices.len() - 1
    }

    /// Find the entry index for `key` via linear probing, or None.
    fn find(&self, key: &PyObjectRef, h: usize) -> Option<usize> {
        if self.indices.is_empty() {
            return None;
        }
        let mask = self.mask();
        let start = h & mask;
        let mut i = start;
        loop {
            let idx_val = self.indices[i];
            if idx_val == 0 {
                return None;
            }
            let entry_idx = (idx_val - 1) as usize;
            if let Some((k, _)) = &self.entries[entry_idx] {
                if k.is(key) || k.equals(key).unwrap_or(false) {
                    return Some(entry_idx);
                }
            }
            i = (i + 1) & mask;
            if i == start {
                return None;
            }
        }
    }

    /// Probe for the key: returns (index_slot, Some(entry_index)) if found,
    /// or (first_empty_or_tombstone_slot, None) if not found.
    fn probe(&self, key: &PyObjectRef, h: usize) -> (usize, Option<usize>) {
        if self.indices.is_empty() {
            return (0, None);
        }
        let mask = self.mask();
        let start = h & mask;
        let mut first_tomb = None;
        let mut i = start;
        loop {
            let idx_val = self.indices[i];
            if idx_val == 0 {
                return (first_tomb.unwrap_or(i), None);
            }
            let entry_idx = (idx_val - 1) as usize;
            if self.entries[entry_idx].is_none() {
                if first_tomb.is_none() {
                    first_tomb = Some(i);
                }
            } else if let Some((k, _)) = &self.entries[entry_idx] {
                if k.equals(key).unwrap_or(false) {
                    return (i, Some(entry_idx));
                }
            }
            i = (i + 1) & mask;
            if i == start {
                return (first_tomb.unwrap_or(i), None);
            }
        }
    }

    /// Like `probe`, but bails out with `Err(())` instead of ever calling
    /// `.equals()` against an `Instance`-typed key — comparing two
    /// natively-typed keys (int/str/etc.) can never run Python code, so
    /// probing to completion is always safe for those; only an `Instance`
    /// key (either the new key or an existing colliding one) can hide an
    /// arbitrary, possibly dict-reentrant `__eq__`. Lets callers take a fast,
    /// single-borrow path for the overwhelmingly common case (native keys)
    /// and fall back to the slower reentrancy-safe snapshot path (see
    /// `pydict_safe_set`) only when genuinely necessary.
    fn probe_no_reentry_risk(
        &self,
        key: &PyObjectRef,
        h: usize,
    ) -> Result<(usize, Option<usize>), ()> {
        if self.indices.is_empty() {
            return Ok((0, None));
        }
        let mask = self.mask();
        let start = h & mask;
        let mut first_tomb = None;
        let mut i = start;
        loop {
            let idx_val = self.indices[i];
            if idx_val == 0 {
                return Ok((first_tomb.unwrap_or(i), None));
            }
            let entry_idx = (idx_val - 1) as usize;
            if self.entries[entry_idx].is_none() {
                if first_tomb.is_none() {
                    first_tomb = Some(i);
                }
            } else if let Some((k, _)) = &self.entries[entry_idx] {
                if k.is(key) {
                    return Ok((i, Some(entry_idx)));
                }
                if matches!(&*k.borrow(), PyObject::Instance { .. })
                    || matches!(&*key.borrow(), PyObject::Instance { .. })
                {
                    return Err(());
                }
                if k.equals(key).unwrap_or(false) {
                    return Ok((i, Some(entry_idx)));
                }
            }
            i = (i + 1) & mask;
            if i == start {
                return Ok((first_tomb.unwrap_or(i), None));
            }
        }
    }

    fn ensure_capacity(&mut self, additional: usize) {
        let needed = self.size + additional;
        if self.indices.is_empty() {
            self.indices = vec![0u32; 8];
        } else if needed * 3 > self.indices.len() * 2 {
            self.rehash(self.indices.len() * 2);
        }
    }

    fn rehash(&mut self, new_cap: usize) {
        let mut new_idx = vec![0u32; new_cap];
        let mask = new_cap - 1;
        for (ei, entry) in self.entries.iter().enumerate() {
            if let Some((key, _)) = entry {
                if let Ok(h) = key.hash() {
                    let mut i = h & mask;
                    while new_idx[i] != 0 {
                        i = (i + 1) & mask;
                    }
                    new_idx[i] = (ei + 1) as u32;
                }
            }
        }
        self.indices = new_idx;
        self.version = self.version.wrapping_add(1);
    }
    pub fn contains(&self, key: &PyObjectRef) -> PyResult<bool> {
        let h = key.hash()?;
        Ok(self.find(key, h).is_some())
    }
    pub fn get(&self, key: &PyObjectRef) -> PyResult<Option<PyObjectRef>> {
        let h = key.hash()?;
        Ok(self.get_with_hash(key, h))
    }
    /// Same as `get`, but takes an already-computed hash — lets a caller
    /// compute `key.hash()` (which may run arbitrary Python via a custom
    /// `__hash__` and can legally re-enter and mutate this very dict, see
    /// `set_with_hash`'s doc comment) BEFORE taking any borrow that would
    /// alias with that reentrant call.
    pub fn get_with_hash(&self, key: &PyObjectRef, h: usize) -> Option<PyObjectRef> {
        self.find(key, h)
            .map(|i| self.entries[i].as_ref().unwrap().1.clone())
    }
    pub fn set(&mut self, key: PyObjectRef, value: PyObjectRef) -> PyResult<()> {
        let h = key.hash()?;
        self.set_with_hash(key, value, h)
    }
    /// Same as `set`, but takes an already-computed hash. Callers that reach
    /// this through an Rc<RefCell<PyObject>>'s own `borrow_mut()` (e.g.
    /// `py_setitem`'s `PyObject::Dict` arm, backing `d[k] = v`) MUST compute
    /// `key.hash()` themselves before taking that borrow, not rely on this
    /// method to do it internally — a key with a Python-level `__hash__`
    /// override can run arbitrary code, including code that mutates THIS
    /// SAME dict (real CPython test: gh-97591, `d[K()] = V()` where
    /// `K.__hash__` does `d.clear()`) — computing the hash while `obj`'s own
    /// mutable borrow is still held would re-enter `borrow_mut()` on the
    /// identical `RefCell` and panic ("already borrowed"), unlike CPython's
    /// refcounted object model, which has no equivalent static aliasing
    /// restriction.
    pub fn set_with_hash(
        &mut self,
        key: PyObjectRef,
        value: PyObjectRef,
        h: usize,
    ) -> PyResult<()> {
        self.ensure_capacity(1);
        let (slot, existing) = self.probe(&key, h);
        self.apply_probed_set(slot, existing, key, value);
        Ok(())
    }

    /// Commits a slot/entry-index pair already computed by `probe`/
    /// `probe_no_reentry_risk` (against either `self` directly or a
    /// consistent snapshot of it — see `pydict_safe_set`'s doc comment).
    /// Never calls `.equals()`/`.hash()` itself, so it's always safe to run
    /// under a live borrow.
    pub(crate) fn apply_probed_set(
        &mut self,
        slot: usize,
        existing: Option<usize>,
        key: PyObjectRef,
        value: PyObjectRef,
    ) {
        self.version = self.version.wrapping_add(1);
        let val_for_instance = value.clone();
        if let Some(entry_idx) = existing {
            self.entries[entry_idx].as_mut().unwrap().1 = value;
        } else {
            let entry_idx = self.entries.len();
            self.entries.push(Some((key.clone(), value)));
            self.indices[slot] = (entry_idx + 1) as u32;
            self.size += 1;
        }
        // Propagate to Instance dict if this is a __dict__ view
        if let Some(ref inst_ref) = self.instance_ref {
            match &mut *inst_ref.borrow_mut() {
                PyObject::Instance { dict, .. } => {
                    dict.insert(key.str(), val_for_instance);
                }
                // `func.__dict__['k'] = v` (decorator helpers like
                // test_decorators.py's MiscDecorators.author set a function
                // attribute through the live __dict__ view) — the propagation
                // is currently Instance-only; without this arm the write went
                // into the copy and `f.author` never saw it.
                PyObject::Function(f) => {
                    f.dict.insert(key.str(), val_for_instance);
                }
                PyObject::Module { dict, name } => {
                    let sid = crate::interner::intern(&key.str());
                    dict.insert(sid, val_for_instance.clone());
                    // Also keep the module's `frame.globals` (the Rc
                    // captured by functions defined in that module) in
                    // sync — `exec_module_source` registers it in
                    // MODULE_GLOBALS_REGISTRY, and `STORE_GLOBAL` in
                    // vm.rs mirrors the other direction. Without this,
                    // `module.__dict__['__cached...'] = None` (used by
                    // `test.support.script_helper`'s setUp/tearDown to
                    // reset its cache) would update only `module.dict`
                    // and stay invisible to `LOAD_GLOBAL` inside the
                    // function, so the cache never appeared to reset
                    // and `mock.patch`'d `check_call` was never hit.
                    MODULE_GLOBALS_REGISTRY.with(|reg| {
                        if let Some(g) = reg.borrow().get(name) {
                            g.borrow_mut().insert(sid, val_for_instance.clone());
                        }
                    });
                }
                _ => {}
            }
        }
    }
    pub fn remove(&mut self, key: &PyObjectRef) -> PyResult<PyObjectRef> {
        let h = key.hash()?;
        self.remove_with_hash(key, h)
    }
    /// Same as `remove`, but takes an already-computed hash — see
    /// `set_with_hash`'s doc comment for why a caller holding `obj`'s own
    /// mutable borrow must compute the hash before taking it.
    pub fn remove_with_hash(&mut self, key: &PyObjectRef, h: usize) -> PyResult<PyObjectRef> {
        let existing = self
            .find(key, h)
            .ok_or_else(|| PyError::key_error(key.str()))?;
        let removed = self.entries[existing].take().unwrap().1;
        self.size -= 1;
        self.version = self.version.wrapping_add(1);
        Ok(removed)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&PyObjectRef, &PyObjectRef)> {
        self.entries
            .iter()
            .filter_map(|e| e.as_ref().map(|(k, v)| (k, v)))
    }
    pub fn keys(&self) -> Vec<PyObjectRef> {
        self.iter().map(|(k, _)| k.clone()).collect()
    }
    pub fn values(&self) -> Vec<PyObjectRef> {
        self.iter().map(|(_, v)| v.clone()).collect()
    }
    pub fn items(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        self.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
    /// Get a value by object identity (pointer comparison), used for memo cache.
    /// Get a value by object IDENTITY (same semantics as the `is` operator
    /// — i.e. the same underlying heap object, matching CPython's own
    /// `id()`), used for memo caches like `copy.deepcopy`'s cycle/diamond-
    /// reference detection. Previously compared `key: &PyObjectRef`'s OWN
    /// reference address (`*const PyObjectRef`, i.e. wherever the CALLER's
    /// local variable/parameter happens to live on the stack) against the
    /// address of the dict's internally-stored `PyObjectRef` value — two
    /// completely different memory locations for what is logically "the
    /// same object", so this NEVER matched in practice regardless of how
    /// many times the same object was looked up. Confirmed via the
    /// simplest repro: `copy.deepcopy` on a self-referential dict/list
    /// recursing forever, since its cycle-detection memo lookup could
    /// never find the very entry it had just stored. Fixed by delegating
    /// to `PyObjectRef::is`, the established (`Rc::ptr_eq`-based) identity
    /// comparison already used for the `is` operator itself.
    pub fn get_by_identity(&self, key: &PyObjectRef) -> Option<PyObjectRef> {
        for (k, v) in self.entries.iter().flatten() {
            if k.is(key) {
                return Some(v.clone());
            }
        }
        None
    }

    /// Insert/update by object IDENTITY, bypassing `.hash()`/`.equals()`
    /// entirely — the matching counterpart to `get_by_identity`, needed for
    /// the SAME reason: a memo cache (`copy.deepcopy`'s cycle detection)
    /// must be able to use genuinely UNHASHABLE objects (dict, list, set —
    /// precisely the mutable container types most likely to form a cycle)
    /// as keys, keyed by identity rather than value. The ordinary `set()`
    /// computes `key.hash()?` first, which returns `Err("unhashable
    /// type")` for those types — and every call site that ignored that
    /// Result (`let _ = memo_dict.set(...)`) silently no-op'd instead of
    /// ever actually storing the entry, so the memo dict stayed
    /// permanently empty and cycle detection never worked at all.
    pub fn set_by_identity(&mut self, key: PyObjectRef, value: PyObjectRef) {
        for entry in self.entries.iter_mut().flatten() {
            if entry.0.is(&key) {
                entry.1 = value;
                return;
            }
        }
        self.entries.push(Some((key, value)));
        self.size += 1;
    }
}
/// Safely insert/overwrite `key` -> `value` into a live `PyObject::Dict`
/// referenced by `target`, WITHOUT ever holding `target`'s own
/// `borrow_mut()` across a `.equals()` call against another key. Probing
/// for an existing colliding key can run arbitrary Python (a custom
/// `__eq__`) that mutates THIS SAME dict — real, deliberate CPython
/// regression test: `test_dict.py`'s `test_clear_at_lookup` (gh-140551),
/// where a key's `__hash__` always returns `1` and its `__eq__` calls
/// `d.clear()` unconditionally. Holding a live borrow across that used to
/// panic with "RefCell already borrowed" the instant the reentrant call
/// made its own borrow. Mirrors `pyset_safe_add` (`pyset.rs`): take the fast
/// single-borrow path whenever no `Instance`-typed key is involved (the
/// overwhelming common case — native keys can never run Python code during
/// comparison, so there's no O(n) snapshot cost for ordinary str/int-keyed
/// dicts), and fall back to a snapshot-probe-verify retry loop otherwise —
/// verified via a `version` counter that nothing changed underneath before
/// committing, retrying against fresh state if it did (matching CPython's
/// own fix for this same bug: restart the lookup rather than trusting
/// now-stale indices).
pub(crate) fn pydict_safe_set(
    target: &PyObjectRef,
    key: PyObjectRef,
    value: PyObjectRef,
) -> PyResult<()> {
    let h = key.hash()?;
    {
        let mut obj = target.borrow_mut();
        match &mut *obj {
            PyObject::Dict(d) => {
                d.ensure_capacity(1);
                if let Ok((slot, existing)) = d.probe_no_reentry_risk(&key, h) {
                    d.apply_probed_set(slot, existing, key, value);
                    return Ok(());
                }
            }
            _ => return Err(PyError::runtime_error("setitem on non-dict")),
        }
    }
    loop {
        let snap_version = {
            let mut obj = target.borrow_mut();
            match &mut *obj {
                PyObject::Dict(d) => {
                    d.ensure_capacity(1);
                    d.version()
                }
                _ => return Err(PyError::runtime_error("setitem on non-dict")),
            }
        };
        let snapshot = {
            let obj = target.borrow();
            match &*obj {
                PyObject::Dict(d) => (**d).clone(),
                _ => return Err(PyError::runtime_error("setitem on non-dict")),
            }
        };
        let (slot, existing) = snapshot.probe(&key, h);
        let mut obj = target.borrow_mut();
        match &mut *obj {
            PyObject::Dict(d) if d.version() == snap_version => {
                d.apply_probed_set(slot, existing, key, value);
                return Ok(());
            }
            PyObject::Dict(_) => {
                drop(obj);
                continue;
            }
            _ => return Err(PyError::runtime_error("setitem on non-dict")),
        }
    }
}

/// Safely implement `dict.setdefault(key, default)` on a live `PyObject::Dict`
/// referenced by `target` — same reentrancy hazard and technique as
/// `pydict_safe_set` (probing can run an existing key's `__eq__`, which may
/// reentrantly mutate this same dict), just returning the found-or-inserted
/// value instead of `()`.
pub(crate) fn pydict_safe_get_or_insert(
    target: &PyObjectRef,
    key: PyObjectRef,
    default: PyObjectRef,
) -> PyResult<PyObjectRef> {
    let h = key.hash()?;
    {
        let mut obj = target.borrow_mut();
        match &mut *obj {
            PyObject::Dict(d) => {
                d.ensure_capacity(1);
                if let Ok((slot, existing)) = d.probe_no_reentry_risk(&key, h) {
                    if let Some(entry_idx) = existing {
                        return Ok(d.entries[entry_idx].as_ref().unwrap().1.clone());
                    }
                    d.apply_probed_set(slot, None, key, default.clone());
                    return Ok(default);
                }
            }
            _ => return Err(PyError::runtime_error("setdefault on non-dict")),
        }
    }
    loop {
        let snap_version = {
            let mut obj = target.borrow_mut();
            match &mut *obj {
                PyObject::Dict(d) => {
                    d.ensure_capacity(1);
                    d.version()
                }
                _ => return Err(PyError::runtime_error("setdefault on non-dict")),
            }
        };
        let snapshot = {
            let obj = target.borrow();
            match &*obj {
                PyObject::Dict(d) => (**d).clone(),
                _ => return Err(PyError::runtime_error("setdefault on non-dict")),
            }
        };
        let (slot, existing) = snapshot.probe(&key, h);
        let mut obj = target.borrow_mut();
        match &mut *obj {
            PyObject::Dict(d) if d.version() == snap_version => {
                if let Some(entry_idx) = existing {
                    return Ok(d.entries[entry_idx].as_ref().unwrap().1.clone());
                }
                d.apply_probed_set(slot, None, key, default.clone());
                return Ok(default);
            }
            PyObject::Dict(_) => {
                drop(obj);
                continue;
            }
            _ => return Err(PyError::runtime_error("setdefault on non-dict")),
        }
    }
}
mod views;
pub use views::make_dict_view;

mod methods;
pub use methods::{
    builtin_dict_getitem, builtin_dict_setitem, dict_method_get, dict_method_items,
    dict_method_iter, dict_method_keys, dict_method_values,
};
pub(crate) use methods::instance_builtin_dict_method;

