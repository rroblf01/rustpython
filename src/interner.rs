/// String interner — replaces String clones with integer IDs.
///
/// In CPython, string interning is crucial for performance because
/// name lookups (LOAD_NAME, LOAD_GLOBAL, LOAD_ATTR) happen on every
/// attribute access. This interner stores each unique string once
/// and returns a u32 index, replacing HashMap<String, ...> lookups
/// with Vec lookups and replacing String clones with integer copies.
///
/// Architecture:
///   - Interner<StringInterner> owns the string storage
///   - StrId is a u32 index into the intern table
///   - String keys in HashMaps are replaced by StrId keys
///   - Two-way mapping: str → StrId and StrId → &str
use std::collections::HashMap;

thread_local! {
    static GLOBAL_INTERNER: std::cell::RefCell<Interner> = std::cell::RefCell::new(Interner::new());
}

pub fn intern(s: &str) -> StrId {
    GLOBAL_INTERNER.with(|i| i.borrow_mut().intern(s))
}

pub fn lookup(id: StrId) -> String {
    GLOBAL_INTERNER.with(|i| i.borrow().lookup(id).to_string())
}

/// A compact string identifier (u32 index).
/// 4 bytes instead of a String's 24 bytes + heap allocation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StrId(pub u32);

impl StrId {
    pub const EMPTY: StrId = StrId(u32::MAX);
}

/// Thread-safe string interner with O(1) insert and lookup.
/// Stores strings in a Vec<Box<str>> for cache-friendly iteration.
pub struct Interner {
    strings: Vec<&'static str>,  // Interned strings (leaked — process lifetime)
    lookup: HashMap<&'static str, StrId>,
}

impl Interner {
    pub fn new() -> Self {
        Interner {
            strings: Vec::new(),
            lookup: HashMap::new(),
        }
    }

    /// Intern a string — returns a stable StrId.
    /// The string data lives for the process lifetime (intentional leak).
    pub fn intern(&mut self, s: &str) -> StrId {
        if let Some(&id) = self.lookup.get(s) {
            return id;
        }
        let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
        let id = StrId(self.strings.len() as u32);
        self.strings.push(leaked);
        self.lookup.insert(leaked, id);
        id
    }

    /// Look up a string by ID — O(1).
    pub fn lookup(&self, id: StrId) -> &str {
        self.strings[id.0 as usize]
    }

    /// Number of interned strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

/// Interned name map — replaces HashMap<String, V> with Vec<Option<V>>.
///
/// Instead of storing String keys (24 bytes + heap) for every name
/// in dicts, frames, and modules, we use StrId u32 keys and a Vec.
/// For small dicts (< 32 entries), this is dramatically faster:
///   - No hashing on lookup (just Vec index)
///   - No allocation per key
///   - Cache-friendly linear scan for small sizes
#[derive(Clone)]
pub struct InternedMap<V> {
    entries: Vec<Option<(StrId, V)>>,
    len: usize,
}

impl<V: Clone> InternedMap<V> {
    pub fn new() -> Self {
        InternedMap {
            entries: Vec::new(),
            len: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        InternedMap {
            entries: Vec::with_capacity(cap),
            len: 0,
        }
    }

    /// Insert by string key (convenience — interns the string first)
    pub fn insert_str(&mut self, name: &str, value: V) -> Option<V> {
        self.insert(crate::interner::intern(name), value)
    }

    pub fn insert(&mut self, key: StrId, value: V) -> Option<V> {
        // Linear scan — fast for small sizes (typical Python dicts)
        for entry in &mut self.entries {
            if let Some((k, _)) = entry {
                if *k == key {
                    let old = std::mem::replace(entry, Some((key, value)));
                    return old.map(|(_, v)| v);
                }
            }
        }
        self.entries.push(Some((key, value)));
        self.len += 1;
        None
    }

    /// Look up by string key (convenience — interns the string first)
    pub fn get_str(&self, name: &str) -> Option<&V> {
        self.get(crate::interner::intern(name))
    }

    pub fn get(&self, key: StrId) -> Option<&V> {
        for entry in &self.entries {
            if let Some((k, ref v)) = entry {
                if *k == key {
                    return Some(v);
                }
            }
        }
        None
    }

    pub fn get_mut(&mut self, key: StrId) -> Option<&mut V> {
        for entry in &mut self.entries {
            if let Some((k, ref mut v)) = entry {
                if *k == key {
                    return Some(v);
                }
            }
        }
        None
    }

    /// Check if a string key exists (convenience — interns the string first)
    pub fn contains_key_str(&self, name: &str) -> bool {
        self.get_str(name).is_some()
    }

    pub fn contains_key(&self, key: StrId) -> bool {
        self.get(key).is_some()
    }

    pub fn remove(&mut self, key: StrId) -> Option<V> {
        for entry in &mut self.entries {
            if let Some((k, _)) = entry {
                if *k == key {
                    let old = entry.take();
                    self.len -= 1;
                    return old.map(|(_, v)| v);
                }
            }
        }
        None
    }

    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn iter(&self) -> impl Iterator<Item = (StrId, &V)> {
        self.entries.iter().filter_map(|e| e.as_ref().map(|(k, v)| (*k, v)))
    }

    pub fn keys(&self) -> impl Iterator<Item = StrId> + '_ {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    /// Convert to a HashMap for compatibility with existing code
    pub fn to_hashmap(&self, interner: &Interner) -> HashMap<String, V>
    where V: Clone {
        let mut map = HashMap::with_capacity(self.len);
        for (id, v) in self.iter() {
            map.insert(interner.lookup(id).to_string(), v.clone());
        }
        map
    }

    /// Build from a HashMap (for migration compatibility)
    pub fn clear(&mut self) {
        self.entries.clear();
        self.len = 0;
    }

    pub fn from_hashmap(map: &HashMap<String, V>, interner: &mut Interner) -> Self
    where V: Clone {
        let mut im = InternedMap::with_capacity(map.len());
        for (k, v) in map {
            im.insert(interner.intern(k), v.clone());
        }
        im
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interner() {
        let mut interner = Interner::new();
        let a = interner.intern("hello");
        let b = interner.intern("hello");
        assert_eq!(a, b);
        assert_eq!(interner.lookup(a), "hello");
    }

    #[test]
    fn test_interned_map() {
        let mut interner = Interner::new();
        let k1 = interner.intern("key1");
        let k2 = interner.intern("key2");

        let mut map: InternedMap<i32> = InternedMap::new();
        map.insert(k1, 42);
        map.insert(k2, 99);
        assert_eq!(*map.get(k1).unwrap(), 42);
        assert_eq!(*map.get(k2).unwrap(), 99);
        assert_eq!(map.len(), 2);

        map.remove(k1);
        assert!(!map.contains_key(k1));
        assert_eq!(map.len(), 1);
    }
}
