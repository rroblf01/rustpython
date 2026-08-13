// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds `PySet`, the
// hash-based set implementation backing `PyObject::Set`/`FrozenSet`.
use super::*;

#[derive(Clone)]
pub struct PySet {
    entries: Vec<Option<PyObjectRef>>,
    indices: Vec<u32>,
    size: usize,
}

impl PySet {
    pub fn new() -> Self {
        PySet {
            entries: Vec::new(),
            indices: Vec::new(),
            size: 0,
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
    }

    fn mask(&self) -> usize {
        self.indices.len() - 1
    }

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
            if let Some(k) = &self.entries[entry_idx] {
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
            } else if let Some(k) = &self.entries[entry_idx] {
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
            if let Some(key) = entry {
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
    }

    pub fn contains(&self, key: &PyObjectRef) -> PyResult<bool> {
        let h = key.hash()?;
        Ok(self.find(key, h).is_some())
    }

    pub fn add(&mut self, key: PyObjectRef) -> PyResult<()> {
        let h = key.hash()?;
        self.ensure_capacity(1);
        let (slot, existing) = self.probe(&key, h);
        if existing.is_some() {
            return Ok(());
        }
        let entry_idx = self.entries.len();
        self.entries.push(Some(key));
        self.indices[slot] = (entry_idx + 1) as u32;
        self.size += 1;
        Ok(())
    }

    /// Insert `key` (already confirmed absent by a prior `probe()` done
    /// against a SNAPSHOT — see the `"add"` `BuiltinMethod` closure) purely
    /// mechanically: finds an empty/tombstone slot by hash alone, no
    /// `.equals()` calls at all. Callers use this specifically so the
    /// actual mutation never runs arbitrary Python while a live
    /// `borrow_mut()` of the enclosing `PyObjectRef` is held.
    fn insert_no_check(&mut self, key: PyObjectRef, h: usize) {
        self.ensure_capacity(1);
        let mask = self.mask();
        let start = h & mask;
        let mut i = start;
        let mut first_tomb = None;
        let slot = loop {
            let idx_val = self.indices[i];
            if idx_val == 0 {
                break first_tomb.unwrap_or(i);
            }
            let entry_idx = (idx_val - 1) as usize;
            if self.entries[entry_idx].is_none() && first_tomb.is_none() {
                first_tomb = Some(i);
            }
            i = (i + 1) & mask;
            if i == start {
                break first_tomb.unwrap_or(i);
            }
        };
        let entry_idx = self.entries.len();
        self.entries.push(Some(key));
        self.indices[slot] = (entry_idx + 1) as u32;
        self.size += 1;
    }

    pub fn remove(&mut self, key: &PyObjectRef) -> PyResult<PyObjectRef> {
        let h = key.hash()?;
        let existing = self
            .find(key, h)
            .ok_or_else(|| PyError::key_error(key.str()))?;
        let removed = self.entries[existing].take().unwrap();
        self.size -= 1;
        Ok(removed)
    }

    pub fn pop(&mut self) -> Option<PyObjectRef> {
        for entry in self.entries.iter_mut().rev() {
            if let Some(val) = entry.take() {
                self.size -= 1;
                return Some(val);
            }
        }
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = &PyObjectRef> {
        self.entries.iter().filter_map(|e| e.as_ref())
    }

    pub fn to_vec(&self) -> Vec<PyObjectRef> {
        self.iter().cloned().collect()
    }

    pub fn from_vec(vec: Vec<PyObjectRef>) -> PyResult<Self> {
        let mut set = PySet::new();
        for item in vec {
            set.add(item)?;
        }
        Ok(set)
    }

    pub fn is_superset(&self, other: &PySet) -> bool {
        for item in other.iter() {
            if self.contains(item).unwrap_or(false) == false {
                return false;
            }
        }
        true
    }

    pub fn is_subset(&self, other: &PySet) -> bool {
        other.is_superset(self)
    }
}

/// Safely add `key` to a live `PyObject::Set` referenced by `target`,
/// WITHOUT ever holding `target`'s own `borrow_mut()` across a
/// `.equals()` call. Probing for an existing equal element can run
/// arbitrary Python (a custom `__eq__`) that mutates THIS SAME set (e.g.
/// `s.add(X())`/`s.clear()` from within its own `__eq__`) — real,
/// deliberate CPython regression tests: `test_set.py`'s
/// `test_hash_collision_concurrent_add` and `TestOperationsMutating`
/// (bpo-46615). Holding a live borrow across that used to panic with
/// "RefCell already borrowed" the instant the reentrant call made its own
/// borrow. Used by both `set.add()` and `set.update()`'s BuiltinMethod
/// closures — any other call site that adds items to a set one at a time
/// in a loop should use this too rather than `PySet::add` directly.
pub(crate) fn pyset_safe_add(target: &PyObjectRef, key: PyObjectRef) -> PyResult<()> {
    let h = key.hash()?;
    let snapshot = {
        let obj = target.borrow();
        match &*obj {
            PyObject::Set(set) => set.clone(),
            _ => return Err(PyError::runtime_error("add on non-set")),
        }
    };
    let (_, existing) = snapshot.probe(&key, h);
    if existing.is_none() {
        if let PyObject::Set(set) = &mut *target.borrow_mut() {
            set.insert_no_check(key, h);
        }
    }
    Ok(())
}
