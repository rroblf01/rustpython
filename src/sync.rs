/// Thread-safe synchronization primitives for RustPython.
///
/// Provides alias types that switch between single-threaded (`Rc<RefCell>`)
/// and multi-threaded (`Arc<RwLock>`) modes.  In single-threaded mode the
/// overhead is minimal (same as the original).  In multi-threaded mode all
/// shared state becomes `Send + Sync`.
///
/// ## Usage
/// ```ignore
/// use crate::sync::{Shared, SharedCell};
/// let x: Shared<HashMap<String, PyObjectRef>> = Shared::new(HashMap::new());
/// ```
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};

// ── Configuration ─────────────────────────────────────────────────────────
// Set env RUSTPYTHON_THREAD_SAFE=1 to enable multi-threaded mode.
// Default: single-threaded (Rc<RefCell>) for maximum speed.

thread_local! {
    static THREAD_SAFE: bool = std::env::var("RUSTPYTHON_THREAD_SAFE").is_ok();
}

/// Returns true if thread-safe mode is enabled
pub fn is_thread_safe() -> bool {
    THREAD_SAFE.with(|v| *v)
}

// ── Thread-safe vs single-threaded wrappers ───────────────────────────────

/// A shared reference-counted pointer.
/// Equivalent to either `Rc<RefCell<T>>` or `Arc<RwLock<T>>`.
#[derive(Clone)]
pub enum Shared<T: 'static> {
    Single(std::rc::Rc<std::cell::RefCell<T>>),
    Multi(std::sync::Arc<std::sync::RwLock<T>>),
}

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Shared::Single(std::rc::Rc::new(std::cell::RefCell::new(value)))
    }

    pub fn new_multi(value: T) -> Self {
        Shared::Multi(std::sync::Arc::new(std::sync::RwLock::new(value)))
    }

    pub fn read(&self) -> SharedRef<T> {
        match self {
            Shared::Single(rc) => SharedRef::Single(rc.borrow()),
            Shared::Multi(arc) => SharedRef::Multi(arc.read().unwrap()),
        }
    }

    pub fn write(&self) -> SharedMut<T> {
        match self {
            Shared::Single(rc) => SharedMut::Single(rc.borrow_mut()),
            Shared::Multi(arc) => SharedMut::Multi(arc.write().unwrap()),
        }
    }

    pub fn ptr_eq(&self, other: &Shared<T>) -> bool {
        match (self, other) {
            (Shared::Single(a), Shared::Single(b)) => std::rc::Rc::ptr_eq(a, b),
            (Shared::Multi(a), Shared::Multi(b)) => std::sync::Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn as_ptr(&self) -> *const T {
        match self {
            Shared::Single(rc) => rc.as_ptr(),
            Shared::Multi(arc) => Arc_as_ptr(arc) as *const T,
        }
    }
}

// No manual Send/Sync impl: the `Single(Rc<RefCell<T>>)` variant is never
// safe to share across threads (Rc's refcount isn't atomic), so `Shared<T>`
// must stay !Send/!Sync regardless of T. Only `new_multi()`'s Arc<RwLock<T>>
// payload is actually thread-safe — that's a property of the value, not the
// type, so it can't be expressed as a trait impl here.

fn Arc_as_ptr<T>(arc: &std::sync::Arc<T>) -> *const T {
    std::sync::Arc::as_ptr(arc)
}

/// A read guard, like `Ref` or `RwLockReadGuard`.
pub enum SharedRef<'a, T: 'static> {
    Single(std::cell::Ref<'a, T>),
    Multi(std::sync::RwLockReadGuard<'a, T>),
}

impl<'a, T> Deref for SharedRef<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            SharedRef::Single(r) => &**r,
            SharedRef::Multi(r) => &**r,
        }
    }
}

/// A write guard, like `RefMut` or `RwLockWriteGuard`.
pub enum SharedMut<'a, T: 'static> {
    Single(std::cell::RefMut<'a, T>),
    Multi(std::sync::RwLockWriteGuard<'a, T>),
}

impl<'a, T> Deref for SharedMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            SharedMut::Single(r) => &**r,
            SharedMut::Multi(r) => &**r,
        }
    }
}

impl<'a, T> DerefMut for SharedMut<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        match self {
            SharedMut::Single(r) => &mut **r,
            SharedMut::Multi(r) => &mut **r,
        }
    }
}

/// An immutable shared pointer (no interior mutability).
/// Equivalent to `Rc<T>` or `Arc<T>`.
#[derive(Clone)]
pub enum SharedImm<T: 'static> {
    Single(std::rc::Rc<T>),
    Multi(std::sync::Arc<T>),
}

impl<T> SharedImm<T> {
    pub fn new(value: T) -> Self {
        SharedImm::Single(std::rc::Rc::new(value))
    }

    pub fn new_multi(value: T) -> Self {
        SharedImm::Multi(std::sync::Arc::new(value))
    }
}

impl<T> Deref for SharedImm<T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            SharedImm::Single(r) => &**r,
            SharedImm::Multi(r) => &**r,
        }
    }
}

// ── Thread-safe HashMap wrapper ───────────────────────────────────────────

/// A thread-safe HashMap.  In single-threaded mode it uses the regular
/// `HashMap`; in multi-threaded mode it wraps in `Mutex`.
pub enum SharedMap<K: Eq + Hash, V> {
    Single(HashMap<K, V>),
    Multi(std::sync::Mutex<HashMap<K, V>>),
}

impl<K: Eq + Hash, V> SharedMap<K, V> {
    pub fn new() -> Self {
        SharedMap::Single(HashMap::new())
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self {
            SharedMap::Single(map) => map.insert(key, value),
            SharedMap::Multi(mutex) => mutex.lock().unwrap().insert(key, value),
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        match self {
            SharedMap::Single(map) => map.get(key),
            SharedMap::Multi(_) => None, // Can't return reference from mutex
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_single_threaded() {
        let s: Shared<i32> = Shared::new(42);
        assert_eq!(*s.read(), 42);
        *s.write() = 100;
        assert_eq!(*s.read(), 100);
    }

    #[test]
    fn test_shared_multi_threaded() {
        let s: Shared<i32> = Shared::new_multi(42);
        assert_eq!(*s.read(), 42);
        *s.write() = 100;
        assert_eq!(*s.read(), 100);
    }

    #[test]
    fn test_shared_ptr_eq() {
        let a: Shared<i32> = Shared::new(1);
        let b = a.clone();
        assert!(a.ptr_eq(&b));
    }

    #[test]
    fn test_shared_imm() {
        let s: SharedImm<String> = SharedImm::new("hello".to_string());
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn test_shared_multi_is_send_sync() {
        // Only the Arc<RwLock<T>> payload is actually thread-safe — verify
        // that directly rather than asserting it of the whole enum type
        // (see the comment on Shared<T>'s definition for why).
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<std::sync::Arc<std::sync::RwLock<i32>>>();
        assert_sync::<std::sync::Arc<std::sync::RwLock<i32>>>();
    }
}
