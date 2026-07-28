// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds `PyDict`, the
// hash-based dict implementation backing `PyObject::Dict`.
use super::*;

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
}

impl PyDict {
    pub fn new() -> Self {
        PyDict { entries: Vec::new(), indices: Vec::new(), size: 0, instance_ref: None }
    }
    pub fn is_empty(&self) -> bool { self.size == 0 }
    pub fn len(&self) -> usize { self.size }
    pub fn clear(&mut self) { self.entries.clear(); self.indices.clear(); self.size = 0; }

    fn mask(&self) -> usize { self.indices.len() - 1 }

    /// Find the entry index for `key` via linear probing, or None.
    fn find(&self, key: &PyObjectRef, h: usize) -> Option<usize> {
        if self.indices.is_empty() { return None; }
        let mask = self.mask();
        let start = h & mask;
        let mut i = start;
        loop {
            let idx_val = self.indices[i];
            if idx_val == 0 { return None; }
            let entry_idx = (idx_val - 1) as usize;
            if let Some((k, _)) = &self.entries[entry_idx] {
                if k.is(key) || k.equals(key).unwrap_or(false) {
                    return Some(entry_idx);
                }
            }
            i = (i + 1) & mask;
            if i == start { return None; }
        }
    }

    /// Probe for the key: returns (index_slot, Some(entry_index)) if found,
    /// or (first_empty_or_tombstone_slot, None) if not found.
    fn probe(&self, key: &PyObjectRef, h: usize) -> (usize, Option<usize>) {
        if self.indices.is_empty() { return (0, None); }
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
                if first_tomb.is_none() { first_tomb = Some(i); }
            } else if let Some((k, _)) = &self.entries[entry_idx] {
                if k.equals(key).unwrap_or(false) {
                    return (i, Some(entry_idx));
                }
            }
            i = (i + 1) & mask;
            if i == start { return (first_tomb.unwrap_or(i), None); }
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
                    while new_idx[i] != 0 { i = (i + 1) & mask; }
                    new_idx[i] = (ei + 1) as u32;
                }
            }
        }
        self.indices = new_idx;
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
        self.find(key, h).map(|i| self.entries[i].as_ref().unwrap().1.clone())
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
    pub fn set_with_hash(&mut self, key: PyObjectRef, value: PyObjectRef, h: usize) -> PyResult<()> {
        self.ensure_capacity(1);
        let (slot, existing) = self.probe(&key, h);
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
            if let PyObject::Instance { dict, .. } = &mut *inst_ref.borrow_mut() {
                dict.insert(key.str(), val_for_instance);
            }
        }
        Ok(())
    }
    pub fn remove(&mut self, key: &PyObjectRef) -> PyResult<PyObjectRef> {
        let h = key.hash()?;
        self.remove_with_hash(key, h)
    }
    /// Same as `remove`, but takes an already-computed hash — see
    /// `set_with_hash`'s doc comment for why a caller holding `obj`'s own
    /// mutable borrow must compute the hash before taking it.
    pub fn remove_with_hash(&mut self, key: &PyObjectRef, h: usize) -> PyResult<PyObjectRef> {
        let existing = self.find(key, h).ok_or_else(|| PyError::key_error(key.str()))?;
        let removed = self.entries[existing].take().unwrap().1;
        self.size -= 1;
        Ok(removed)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&PyObjectRef, &PyObjectRef)> {
        self.entries.iter().filter_map(|e| e.as_ref().map(|(k, v)| (k, v)))
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
/// Helper: provide dict methods (items, keys, values, __iter__) for Instance objects
/// that inherit from dict but can't access the built-in dict methods.
pub(crate) fn instance_builtin_dict_method(name: &str, dict_snapshot: Vec<(String, PyObjectRef)>) -> Option<PyObjectRef> {
    let method_name = name.to_string();
    Some(PyObjectRef::new(PyObject::Closure(Rc::new(move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
        match method_name.as_str() {
            "__iter__" => {
                let keys: Vec<PyObjectRef> = dict_snapshot.iter().map(|(k, _)| py_str(k)).collect();
                Ok(PyObjectRef::new(PyObject::List(keys)))
            }
            "items" => {
                let items: Vec<PyObjectRef> = dict_snapshot.iter().map(|(k, v)| {
                    py_tuple(vec![py_str(k), v.clone()])
                }).collect();
                Ok(PyObjectRef::new(PyObject::List(items)))
            }
            "keys" => {
                let keys: Vec<PyObjectRef> = dict_snapshot.iter().map(|(k, _)| py_str(k)).collect();
                Ok(PyObjectRef::new(PyObject::List(keys)))
            }
            "values" => {
                let values: Vec<PyObjectRef> = dict_snapshot.iter().map(|(_, v)| v.clone()).collect();
                Ok(PyObjectRef::new(PyObject::List(values)))
            }
            _ => Err(PyError::type_error(format!("unsupported dict method: {}", method_name))),
        }
    }))))
}

/// Static dict method: get
pub fn dict_method_get(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Err(PyError::type_error("get() requires at least 1 argument")); }
    let instance = &args[0];
    let borrowed = instance.borrow();
    if let PyObject::Instance { dict, .. } = &*borrowed {
        let key = args[1].str();
        let val = dict.get(&key).cloned().unwrap_or_else(|| {
            if args.len() > 2 { args[2].clone() } else { py_none() }
        });
        drop(borrowed);
        Ok(val)
    } else {
        Err(PyError::type_error("get() requires a dict-like instance"))
    }
}

/// Static dict method: __iter__
pub fn dict_method_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("__iter__ requires self")); }
    let instance = &args[0];
    let borrowed = instance.borrow();
    if let PyObject::Instance { dict, .. } = &*borrowed {
        let keys: Vec<PyObjectRef> = dict.keys().map(|k| py_str(k)).collect();
        drop(borrowed);
        Ok(PyObjectRef::new(PyObject::List(keys)))
    } else {
        Err(PyError::type_error("__iter__ requires a dict-like instance"))
    }
}

/// Static dict method: items
pub fn dict_method_items(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("items() requires self")); }
    let instance = &args[0];
    let borrowed = instance.borrow();
    if let PyObject::Instance { dict, .. } = &*borrowed {
        let items: Vec<PyObjectRef> = dict.iter().map(|(k, v)| {
            py_tuple(vec![py_str(k), v.clone()])
        }).collect();
        drop(borrowed);
        Ok(PyObjectRef::new(PyObject::List(items)))
    } else {
        Err(PyError::type_error("items() requires a dict-like instance"))
    }
}

/// Static dict method: keys
pub fn dict_method_keys(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("keys() requires self")); }
    let instance = &args[0];
    let borrowed = instance.borrow();
    if let PyObject::Instance { dict, .. } = &*borrowed {
        let keys: Vec<PyObjectRef> = dict.keys().map(|k| py_str(k)).collect();
        drop(borrowed);
        Ok(PyObjectRef::new(PyObject::List(keys)))
    } else {
        Err(PyError::type_error("keys() requires a dict-like instance"))
    }
}

/// Static dict method: values
pub fn dict_method_values(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("values() requires self")); }
    let instance = &args[0];
    let borrowed = instance.borrow();
    if let PyObject::Instance { dict, .. } = &*borrowed {
        let values: Vec<PyObjectRef> = dict.values().cloned().collect();
        drop(borrowed);
        Ok(PyObjectRef::new(PyObject::List(values)))
    } else {
        Err(PyError::type_error("values() requires a dict-like instance"))
    }
}

/// dict.__setitem__ function: allows dict.__setitem__(instance, key, value)
pub fn builtin_dict_setitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Handle both calling conventions:
    // - Direct: [instance, key, value] (3 args)
    // - Via BuiltinMethod: [py_none(), instance, key, value] (4 args)
    let instance = if args.len() >= 4 { &args[1] } else if args.len() >= 3 { &args[0] } else {
        return Err(PyError::type_error("dict.__setitem__() requires at least 2 arguments"));
    };
    let key = if args.len() >= 4 { args[2].str() } else if args.len() >= 3 { args[1].str() } else {
        return Err(PyError::type_error("dict.__setitem__() requires at least 2 arguments"));
    };
    let value = if args.len() >= 4 { args[3].clone() } else if args.len() >= 3 { args[2].clone() } else {
        return Err(PyError::type_error("dict.__setitem__() requires at least 2 arguments"));
    };
    // A real dict subclass instance (e.g. `class _EnumDict(dict): ...`,
    // used to give enum.EnumType.__prepare__'s namespace object a place to
    // track member-definition order) has its actual dict *contents* in its
    // native backing, not its own attribute storage — `dict.__setitem__`
    // must write there so a later `classdict[key]` subscript read (which
    // goes through the native backing via py_getitem) actually sees it.
    // Only fall back to treating the instance's own attribute dict as "the
    // dict" when there's no native backing at all.
    if let Some(native) = native_backing_of(instance) {
        py_setitem(&native, &py_str(&key), value)?;
        return Ok(py_none());
    }
    let mut obj = instance.borrow_mut();
    if let PyObject::Instance { dict, .. } = &mut *obj {
        dict.insert(key, value);
    } else if let PyObject::Dict(pd) = &mut *obj {
        pd.set(py_str(&key), value).ok();
    } else {
        drop(obj);
        // Fall back to py_setitem for non-Instance types
        py_setitem(instance, &args[if args.len() >= 4 { 2 } else { 1 }], args[if args.len() >= 4 { 3 } else { 2 }].clone())?;
    }
    Ok(py_none())
}

/// dict.__getitem__ function: allows dict.__getitem__(instance, key)
pub fn builtin_dict_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Handle both calling conventions:
    // - Direct: [instance, key] (2 args)
    // - Via BuiltinMethod: [py_none(), instance, key] (3 args)
    let instance = if args.len() >= 3 { &args[1] } else if args.len() >= 2 { &args[0] } else {
        return Err(PyError::type_error("dict.__getitem__() requires at least 1 argument"));
    };
    let key_ref = if args.len() >= 3 { &args[2] } else if args.len() >= 2 { &args[1] } else {
        return Err(PyError::type_error("dict.__getitem__() requires at least 1 argument"));
    };
    let key = key_ref.str();
    // Check for __missing__ first (dict subclass support, e.g. Counter)
    let missing_result = instance.borrow().get_attribute("__missing__").ok()
        .and_then(|missing| crate::object::call_function(&missing, vec![instance.clone(), key_ref.clone()]).ok());
    if let Some(val) = missing_result {
        return Ok(val);
    }
    // See builtin_dict_setitem's matching comment: a real dict subclass's
    // actual contents live in its native backing, not its attribute dict.
    if let Some(native) = native_backing_of(instance) {
        return py_getitem(&native, key_ref);
    }
    // Directly read from the Instance's dict, bypassing py_getitem (which would recurse)
    let obj = instance.borrow();
    if let PyObject::Instance { dict, .. } = &*obj {
        let val = dict.get(&key).cloned().ok_or_else(|| {
            PyError::key_error(format!("'{}'", key))
        })?;
        drop(obj);
        Ok(val)
    } else if let PyObject::Dict(pd) = &*obj {
        let val = pd.get(key_ref)?.unwrap_or_else(py_none);
        drop(obj);
        Ok(val)
    } else {
        drop(obj);
        // Fall back to py_getitem for non-Instance/Dict types
        py_getitem(instance, key_ref)
    }
}
