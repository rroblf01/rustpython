// Split from src/object/core.rs — AttrMap and related helpers.
use crate::interner::{self, StrId};
use crate::object::{DictMap, PyObjectRef};
use std::collections::HashMap;

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
