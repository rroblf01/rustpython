use std::rc::Rc;
use std::cell::RefCell;

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

use std::fmt;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use num_bigint::{BigInt, Sign};
use num_traits::{Zero, One, ToPrimitive, float::FloatCore, Signed};
use regex::Regex;
use crate::bytecode::{needs_arg, CodeObject};
use crate::modules::*;

pub type BuiltinFunc = fn(&[PyObjectRef]) -> PyResult<PyObjectRef>;

pub static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static IMM_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Temporary owned, Rc-held, or RefCell-referenced PyObject
pub enum RefOrOwned<'a> {
    Ref(std::cell::Ref<'a, PyObject>),
    RcRef(std::rc::Rc<PyObject>),
    Owned(PyObject),
}

impl<'a> std::ops::Deref for RefOrOwned<'a> {
    type Target = PyObject;
    fn deref(&self) -> &PyObject {
        match self {
            RefOrOwned::Ref(r) => &**r,
            RefOrOwned::RcRef(r) => &**r,
            RefOrOwned::Owned(o) => o,
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

    pub fn to_string(&self) -> String {
        self.as_str().to_string()
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
        PyObjectRef::Mut(Rc::new(RefCell::new(obj)))
    }

    /// Create an IMMUTABLE PyObjectRef (for Int, Str, Float, etc.)
    pub fn imm(obj: PyObject) -> Self {
        IMM_COUNT.fetch_add(1, Ordering::Relaxed);
        PyObjectRef::Imm(Rc::new(RefCell::new(obj)))
    }

    pub fn borrow(&self) -> RefOrOwned<'_> {
        match self {
            PyObjectRef::SmallInt(n) => RefOrOwned::Owned(PyObject::Int(BigInt::from(*n))),
            PyObjectRef::SmallBool(b) => RefOrOwned::Owned(PyObject::Bool(*b)),
            PyObjectRef::SmallFloat(f) => RefOrOwned::Owned(PyObject::Float(*f)),
            PyObjectRef::SmallStr(s) => RefOrOwned::Owned(PyObject::Str(compact_str::CompactString::from(s.as_str()))),
            PyObjectRef::None => RefOrOwned::Owned(PyObject::None),
            PyObjectRef::Mut(rc) => RefOrOwned::Ref(rc.borrow()),
            PyObjectRef::Imm(rc) => RefOrOwned::Ref(rc.borrow()),
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
            PyObjectRef::Imm(rc) => {
                let result = rc.try_borrow_mut();
                match result {
                    Ok(guard) => guard,
                    Err(_) => {
                        use std::io::Write;
                        let _ = std::io::stderr().write_all(b"RefCell CONFLICT - Imm borrow_mut while borrowed\n");
                        let _ = std::io::stderr().flush();
                        panic!("RefCell already borrowed (Imm)");
                    }
                }
            }
            _ => panic!("borrow_mut on non-mutable value"),
        }
    }

    pub fn is_imm(&self) -> bool {
        matches!(self, PyObjectRef::Imm(_))
    }

    /// Fast path: extract i64 without borrow()
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PyObjectRef::SmallInt(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PyObjectRef::SmallFloat(f) => Some(*f),
            PyObjectRef::SmallInt(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn is(&self, other: &PyObjectRef) -> bool {
        match (self, other) {
            (PyObjectRef::SmallInt(a), PyObjectRef::SmallInt(b)) => a == b,
            (PyObjectRef::SmallBool(a), PyObjectRef::SmallBool(b)) => a == b,
            (PyObjectRef::None, PyObjectRef::None) => true,
            (PyObjectRef::Mut(a), PyObjectRef::Mut(b)) => Rc::ptr_eq(a, b),
            (PyObjectRef::Imm(a), PyObjectRef::Imm(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn repr(&self) -> String { self.borrow().repr() }
    pub fn str(&self) -> String {
        // Check for __str__ on Instance types (user-defined objects)
        let str_func = {
            let obj = self.borrow();
            match &*obj {
                PyObject::Instance { typ, .. } => {
                    let typ_ref = typ.borrow();
                    match &*typ_ref {
                        PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__str__").cloned(),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        if let Some(f) = str_func {
            if let Ok(result) = call_bound_method(f, self.clone(), vec![]) {
                return result.str();
            }
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
            PyObjectRef::Mut(rc) => rc.borrow().truthy(),
            PyObjectRef::Imm(rc) => rc.borrow().truthy(),
        }
    }
    /// Return a raw pointer to the inner PyObject for identity comparison.
    /// For inline variants (SmallInt, SmallBool, SmallFloat, SmallStr, None),
    /// the address of the PyObjectRef itself is returned as a unique identity.
    pub fn raw_ptr(&self) -> *const PyObjectRef {
        self as *const PyObjectRef
    }
    pub fn hash(&self) -> PyResult<usize> {
        match self {
            PyObjectRef::SmallInt(n) => {
                let mut h: usize = 0;
                let bytes = BigInt::from(*n).to_signed_bytes_le();
                for (j, &b) in bytes.iter().enumerate() { h ^= (b as usize) << ((j % 8) * 8); }
                Ok(h)
            }
            PyObjectRef::SmallBool(b) => Ok(if *b { 1 } else { 0 }),
            PyObjectRef::SmallFloat(f) => {
                let bits = f.to_bits();
                Ok(bits as usize ^ (bits >> 32) as usize)
            }
            PyObjectRef::SmallStr(s) => {
                let bytes = s.as_str().as_bytes();
                let mut h: usize = 0;
                for &b in bytes { h = h.wrapping_mul(31).wrapping_add(b as usize); }
                Ok(h)
            }
            PyObjectRef::None => Ok(0),
            PyObjectRef::Mut(rc) => rc.borrow().hash(),
            PyObjectRef::Imm(rc) => rc.borrow().hash(),
        }
    }
    pub fn equals(&self, other: &PyObjectRef) -> PyResult<bool> {
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

// ---- Python Error Types ----

#[derive(Debug, Clone)]
pub enum PyError {
    TypeError(String),
    ValueError(String),
    NameError(String),
    AttributeError(String),
    IndexError(String),
    KeyError(String),
    ZeroDivisionError(String),
    RuntimeError(String),
    SystemExit(i32),
    Exception(String, PyObjectRef),
    MatchError(String),
    StopIteration,
    AssertionError(String),
    OsError(String),
    ImportError(String),
}

impl PyError {
    pub fn type_error(msg: impl Into<String>) -> Self {
        PyError::TypeError(msg.into())
    }
    pub fn name_error(msg: impl Into<String>) -> Self {
        PyError::NameError(msg.into())
    }
    pub fn value_error(msg: impl Into<String>) -> Self {
        PyError::ValueError(msg.into())
    }
    pub fn zero_division() -> Self {
        PyError::ZeroDivisionError("division by zero".to_string())
    }
    pub fn attribute_error(msg: impl Into<String>) -> Self {
        PyError::AttributeError(msg.into())
    }
    pub fn index_error(msg: impl Into<String>) -> Self {
        PyError::IndexError(msg.into())
    }
    pub fn key_error(msg: impl Into<String>) -> Self {
        PyError::KeyError(msg.into())
    }
    pub fn stop_iteration() -> Self {
        PyError::StopIteration
    }
    pub fn runtime_error(msg: impl Into<String>) -> Self {
        PyError::RuntimeError(msg.into())
    }
    pub fn import_error(msg: impl Into<String>) -> Self {
        PyError::ImportError(msg.into())
    }

    pub fn message(&self) -> String {
        match self {
            PyError::TypeError(m) => m.clone(),
            PyError::ValueError(m) => m.clone(),
            PyError::NameError(m) => m.clone(),
            PyError::AttributeError(m) => m.clone(),
            PyError::IndexError(m) => m.clone(),
            PyError::KeyError(m) => m.clone(),
            PyError::ZeroDivisionError(m) => m.clone(),
            PyError::RuntimeError(m) => m.clone(),
            PyError::SystemExit(c) => format!("SystemExit({})", c),
            PyError::Exception(m, _) => m.clone(),
            PyError::MatchError(m) => m.clone(),
            PyError::StopIteration => "".to_string(),
            PyError::AssertionError(m) => m.clone(),
            PyError::OsError(m) => m.clone(),
            PyError::ImportError(m) => m.clone(),
        }
    }
}

impl fmt::Display for PyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            PyError::TypeError(_) => "TypeError",
            PyError::ValueError(_) => "ValueError",
            PyError::NameError(_) => "NameError",
            PyError::AttributeError(_) => "AttributeError",
            PyError::IndexError(_) => "IndexError",
            PyError::KeyError(_) => "KeyError",
            PyError::ZeroDivisionError(_) => "ZeroDivisionError",
            PyError::RuntimeError(_) => "RuntimeError",
            PyError::SystemExit(_) => "SystemExit",
            PyError::Exception(_, _) => "Exception",
            PyError::MatchError(_) => "MatchError",
            PyError::StopIteration => "StopIteration",
            PyError::AssertionError(_) => "AssertionError",
            PyError::OsError(_) => "OSError",
            PyError::ImportError(_) => "ImportError",
        };
        write!(f, "{}: {}", name, self.message())
    }
}

pub type PyResult<T> = Result<T, PyError>;

// ---- PySet: hash-based set for O(1) membership ----

#[derive(Clone)]
pub struct PySet {
    buckets: std::collections::HashMap<usize, Vec<PyObjectRef>>,
    size: usize,
}

impl PySet {
    pub fn new() -> Self { PySet { buckets: std::collections::HashMap::new(), size: 0 } }
    pub fn is_empty(&self) -> bool { self.size == 0 }
    pub fn len(&self) -> usize { self.size }
    pub fn clear(&mut self) { self.buckets.clear(); self.size = 0; }
    fn bucket(&self, key: &PyObjectRef) -> PyResult<Option<&Vec<PyObjectRef>>> {
        let h = key.hash()?; Ok(self.buckets.get(&h))
    }
    fn bucket_mut(&mut self, key: &PyObjectRef) -> PyResult<&mut Vec<PyObjectRef>> {
        let h = key.hash()?; Ok(self.buckets.entry(h).or_default())
    }
    fn find(bucket: &[PyObjectRef], key: &PyObjectRef) -> Option<usize> {
        bucket.iter().position(|k| k.equals(key).unwrap_or(false))
    }
    pub fn contains(&self, key: &PyObjectRef) -> PyResult<bool> {
        match self.bucket(key)? { Some(b) => Ok(Self::find(b, key).is_some()), None => Ok(false) }
    }
    pub fn add(&mut self, key: PyObjectRef) -> PyResult<()> {
        let h = key.hash()?;
        let bucket = self.buckets.entry(h).or_default();
        if !Self::find(bucket, &key).is_some() { bucket.push(key); self.size += 1; }
        Ok(())
    }
    pub fn remove(&mut self, key: &PyObjectRef) -> PyResult<PyObjectRef> {
        let h = key.hash()?;
        let bucket = self.buckets.get_mut(&h).ok_or_else(|| PyError::key_error(key.str()))?;
        let pos = Self::find(bucket, key).ok_or_else(|| PyError::key_error(key.str()))?;
        self.size -= 1; Ok(bucket.swap_remove(pos))
    }
    pub fn pop(&mut self) -> Option<PyObjectRef> {
        let bucket = self.buckets.values_mut().next()?;
        let val = bucket.pop()?;
        if bucket.is_empty() { self.buckets.retain(|_, v| !v.is_empty()); }
        self.size -= 1; Some(val)
    }
    pub fn to_vec(&self) -> Vec<PyObjectRef> {
        self.buckets.values().flat_map(|b| b.clone()).collect()
    }
    pub fn from_vec(vec: Vec<PyObjectRef>) -> PyResult<Self> {
        let mut set = PySet::new();
        for item in vec { set.add(item)?; }
        Ok(set)
    }
    pub fn is_superset(&self, other: &PySet) -> bool {
        for item in other.to_vec() {
            if self.contains(&item).unwrap_or(false) == false { return false; }
        }
        true
    }
    pub fn is_subset(&self, other: &PySet) -> bool {
        other.is_superset(self)
    }
}

// ---- PyDict: hash-based dict with arbitrary hashable keys ----

#[derive(Clone)]
pub struct PyDict {
    pub buckets: std::collections::HashMap<usize, Vec<(PyObjectRef, PyObjectRef)>>,
    pub size: usize,
    pub instance_ref: Option<PyObjectRef>,
}

impl PyDict {
    pub fn new() -> Self { PyDict { buckets: std::collections::HashMap::new(), size: 0, instance_ref: None } }
    pub fn is_empty(&self) -> bool { self.size == 0 }
    pub fn len(&self) -> usize { self.size }
    pub fn clear(&mut self) { self.buckets.clear(); self.size = 0; }
    fn bucket(&self, key: &PyObjectRef) -> PyResult<Option<&Vec<(PyObjectRef, PyObjectRef)>>> {
        let h = key.hash()?; Ok(self.buckets.get(&h))
    }
    fn bucket_mut(&mut self, key: &PyObjectRef) -> PyResult<&mut Vec<(PyObjectRef, PyObjectRef)>> {
        let h = key.hash()?; Ok(self.buckets.entry(h).or_default())
    }
    fn find(bucket: &[(PyObjectRef, PyObjectRef)], key: &PyObjectRef) -> Option<usize> {
        bucket.iter().position(|(k, _)| k.equals(key).unwrap_or(false))
    }
    pub fn contains(&self, key: &PyObjectRef) -> PyResult<bool> {
        match self.bucket(key)? { Some(b) => Ok(Self::find(b, key).is_some()), None => Ok(false) }
    }
    pub fn get(&self, key: &PyObjectRef) -> PyResult<Option<PyObjectRef>> {
        match self.bucket(key)? { Some(b) => Ok(Self::find(b, key).map(|i| b[i].1.clone())), None => Ok(None) }
    }
    pub fn set(&mut self, key: PyObjectRef, value: PyObjectRef) -> PyResult<()> {
        let h = key.hash()?;
        let bucket = self.buckets.entry(h).or_default();
        match Self::find(bucket, &key) {
            Some(i) => bucket[i].1 = value.clone(),
            None => { bucket.push((key.clone(), value.clone())); self.size += 1; }
        }
        // Propagate to Instance dict if this is a __dict__ view
        if let Some(ref inst_ref) = self.instance_ref {
            if let PyObject::Instance { dict, .. } = &mut *inst_ref.borrow_mut() {
                dict.insert(key.str(), value);
            }
        }
        Ok(())
    }
    pub fn remove(&mut self, key: &PyObjectRef) -> PyResult<PyObjectRef> {
        let h = key.hash()?;
        let bucket = self.buckets.get_mut(&h).ok_or_else(|| PyError::key_error(key.str()))?;
        let pos = Self::find(bucket, key).ok_or_else(|| PyError::key_error(key.str()))?;
        self.size -= 1; Ok(bucket.swap_remove(pos).1)
    }
    pub fn keys(&self) -> Vec<PyObjectRef> {
        self.buckets.values().flat_map(|b| b.iter().map(|(k, _)| k.clone())).collect()
    }
    pub fn values(&self) -> Vec<PyObjectRef> {
        self.buckets.values().flat_map(|b| b.iter().map(|(_, v)| v.clone())).collect()
    }
    pub fn items(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        self.buckets.values().flat_map(|b| b.clone()).collect()
    }
    /// Get a value by object identity (pointer comparison), used for memo cache.
    pub fn get_by_identity(&self, key: &PyObjectRef) -> Option<PyObjectRef> {
        let key_ptr: *const PyObjectRef = key;
        for entry in self.buckets.values() {
            for (k, v) in entry {
                let k_ptr: *const PyObjectRef = k;
                if k_ptr == key_ptr {
                    return Some(v.clone());
                }
            }
        }
        None
    }
}

#[derive(Clone)]
pub enum PyObject {
    None,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(compact_str::CompactString),
    Bytes(Vec<u8>),
    ByteArray(Vec<u8>),
    List(Vec<PyObjectRef>),
    Tuple(Vec<PyObjectRef>),
    Dict(PyDict),
    Set(PySet),
    FrozenSet(PySet),
    Range {
        start: i64,
        stop: i64,
        step: i64,
    },
    RangeIter {
        current: i64,
        stop: i64,
        step: i64,
    },
    ListIter {
        list: Vec<PyObjectRef>,
        index: usize,
    },
    EnumerateIter {
        items: Vec<PyObjectRef>,
        pos: usize,
        start: usize,
    },
    Slice {
        start: PyObjectRef,
        stop: PyObjectRef,
        step: PyObjectRef,
    },
    Code(Box<CodeObject>),
    Function {
        code: CodeObject,
        globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
        name: String,
        defaults: Vec<PyObjectRef>,
        closure: Vec<PyObjectRef>,
        dict: HashMap<String, PyObjectRef>,
        jit_ptr: std::cell::Cell<usize>,
        jit_consts: std::cell::RefCell<Vec<PyObjectRef>>,
    },
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
        dict: HashMap<String, PyObjectRef>,
    },
    Type {
        name: String,
        dict: HashMap<String, PyObjectRef>,
        bases: Vec<PyObjectRef>,
        mro: Vec<PyObjectRef>,
    },
    Instance {
        typ: PyObjectRef,
        dict: HashMap<String, PyObjectRef>,
    },
    Cell {
        value: Option<PyObjectRef>,
    },
    Capsule {
        name: String,
        pointer: *mut std::ffi::c_void,
    },
    Exception {
        typ: String,
        args: Vec<PyObjectRef>,
        cause: Option<PyObjectRef>,
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
    },
    File {
        file: std::rc::Rc<std::cell::RefCell<std::fs::File>>,
        name: String,
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
    Property {
        getter: Option<PyObjectRef>,
        setter: Option<PyObjectRef>,
        deleter: Option<PyObjectRef>,
        doc: Option<String>,
    },
    StaticMethod {
        func: PyObjectRef,
    },
    ClassMethod {
        func: PyObjectRef,
    },
    Generator {
        frame: std::cell::RefCell<Option<super::vm::Frame>>,
    },
    Coroutine {
        frame: std::cell::RefCell<Option<super::vm::Frame>>,
    },
    Array(PyArray),
    CompiledRegex {
        regex: regex::Regex,
        pattern: String,
        flags: i32,
    },
    Closure(Rc<dyn Fn(&[PyObjectRef]) -> PyResult<PyObjectRef>>),
    /// Implements the await protocol for Futures.
    /// __await__ returns this, and SEND drives it: first yield the future,
    /// then on second send (via send(None) from the event loop), return the result.
    FutureAwaitIterator {
        future: PyObjectRef,
        yielded: bool,
    },
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
            PyObject::Str(_) => "str",
            PyObject::Bytes(_) => "bytes",
            PyObject::ByteArray(_) => "bytearray",
            PyObject::List(_) => "list",
            PyObject::Tuple(_) => "tuple",
            PyObject::Dict(_) => "dict",
            PyObject::Set(_) => "set",
            PyObject::FrozenSet(_) => "frozenset",
            PyObject::Range { .. } => "range",
            PyObject::RangeIter { .. } => "range_iterator",
            PyObject::ListIter { .. } => "list_iterator",
            PyObject::EnumerateIter { .. } => "enumerate",
            PyObject::Slice { .. } => "slice",
            PyObject::Code(_) => "code",
            PyObject::Function { .. } => "function",
            PyObject::BuiltinFunction { .. } => "builtin_function_or_method",
            PyObject::BuiltinMethod { .. } => "builtin_method",
            PyObject::Module { .. } => "module",
            PyObject::Type { name, .. } => name,
            PyObject::Instance { .. } => "instance",
            PyObject::Cell { .. } => "cell",
            PyObject::Capsule { .. } => "capsule",
            PyObject::Exception { .. } => "Exception",
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
            PyObject::Property { .. } => "property",
            PyObject::StaticMethod { .. } => "staticmethod",
            PyObject::ClassMethod { .. } => "classmethod",
            PyObject::Generator { .. } => "generator",
            PyObject::Coroutine { .. } => "coroutine",
            PyObject::Array(_) => "array",
            PyObject::CompiledRegex { .. } => "re.Pattern",
            PyObject::Closure(_) => "builtin_function_or_method",
            PyObject::FutureAwaitIterator { .. } => "future_await_iterator",
        }.to_string()
    }

    pub fn repr(&self) -> String {
        match self {
            PyObject::None => "None".to_string(),
            PyObject::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            PyObject::Int(i) => i.to_string(),
            PyObject::Float(f) => {
                if f.is_nan() { "nan".to_string() }
                else if f.is_infinite() && f.is_sign_positive() { "inf".to_string() }
                else if f.is_infinite() { "-inf".to_string() }
                else {
                    let s = format!("{:.17}", f);
                    let s = s.trim_end_matches('0').to_string();
                    if s.ends_with('.') { format!("{}0", s) } else { s }
                }
            }
            PyObject::Str(s) => format!("'{}'", escape_string(s)),
            PyObject::Bytes(b) => {
                let s: String = b.iter().map(|&byte| {
                    match byte {
                        b'\\' => "\\\\".to_string(),
                        b'\'' => "\\'".to_string(),
                        b'\n' => "\\n".to_string(),
                        b'\t' => "\\t".to_string(),
                        b'\r' => "\\r".to_string(),
                        0x20..=0x7e => (byte as char).to_string(),
                        _ => format!("\\x{:02x}", byte),
                    }
                }).collect();
                format!("b'{}'", s)
            }
            PyObject::ByteArray(b) => {
                let s: String = b.iter().map(|&byte| {
                    match byte {
                        b'\\' => "\\\\".to_string(),
                        b'\'' => "\\'".to_string(),
                        b'\n' => "\\n".to_string(),
                        b'\t' => "\\t".to_string(),
                        b'\r' => "\\r".to_string(),
                        0x20..=0x7e => (byte as char).to_string(),
                        _ => format!("\\x{:02x}", byte),
                    }
                }).collect();
                format!("bytearray(b'{}')", s)
            }
            PyObject::List(items) => {
                let items: Vec<String> = items.iter().map(|x| x.repr()).collect();
                format!("[{}]", items.join(", "))
            }
            PyObject::Tuple(items) => {
                let items: Vec<String> = items.iter().map(|x| x.repr()).collect();
                if items.len() == 1 {
                    format!("({},)", items[0])
                } else {
                    format!("({})", items.join(", "))
                }
            }
            PyObject::Dict(d) => {
                let items: Vec<String> = d.items().iter()
                    .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            PyObject::Set(items) => {
                let vec = items.to_vec();
                let items: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                format!("{{{}}}", items.join(", "))
            }
            PyObject::FrozenSet(items) => {
                let vec = items.to_vec();
                let items: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                format!("frozenset({{{}}})", items.join(", "))
            }
            PyObject::Range { start, stop, step } => {
                if *step == 1 { format!("range({}, {})", start, stop) }
                else { format!("range({}, {}, {})", start, stop, step) }
            }
            PyObject::RangeIter { .. } => "<range_iterator object>".to_string(),
            PyObject::ListIter { .. } => "<list_iterator object>".to_string(),
            PyObject::EnumerateIter { .. } => "<enumerate object>".to_string(),
            PyObject::Slice { start, stop, step } => {
                format!("slice({}, {}, {})", start.repr(), stop.repr(), step.repr())
            }
            PyObject::Function { name, .. } => format!("<function {}>", name),
            PyObject::BuiltinFunction { name, .. } => format!("<built-in function {}>", name),
            PyObject::BuiltinMethod { name, .. } => format!("<built-in method {}>", name),
            PyObject::Module { name, .. } => format!("<module '{}'>", name),
            PyObject::Type { name, .. } => format!("<class '{}'>", name),
            PyObject::Instance { typ, .. } => {
                format!("<{} object>", typ.borrow().type_name())
            }
            PyObject::Code(c) => format!("<code object {}>", c.name),
            PyObject::Cell { value: Some(v) } => v.repr(),
            PyObject::Cell { value: None } => "None".to_string(),
            PyObject::Capsule { name, .. } => format!("<capsule object '{}'>", name),
            PyObject::Exception { typ, args, cause: _ } => {
                let args_str: Vec<String> = args.iter().map(|a| a.repr()).collect();
                format!("{}({})", typ, args_str.join(", "))
            }
            PyObject::ExceptionGroup { typ, args, exceptions } => {
                let args_str: Vec<String> = args.iter().map(|a| a.repr()).collect();
                let exc_str: Vec<String> = exceptions.iter().map(|e| e.repr()).collect();
                format!("{}({}, {})", typ, args_str.join(", "), exc_str.join(", "))
            }
            PyObject::BuildClass => "<builtin function __build_class__>".to_string(),
            PyObject::BoundMethod { func, .. } => format!("<bound method {}>", func.borrow().type_name()),
            PyObject::Partial { func, .. } => format!("<partial {}>", func.borrow().type_name()),
            PyObject::File { name, .. } => format!("<_io.FileIO '{}'>", name),
            PyObject::Socket { .. } => format!("<socket object>"),
            PyObject::Thread(_) => "<Thread>".to_string(),
            PyObject::Lock(_) => "<lock>".to_string(),
            PyObject::RLock(_) => "<RLock>".to_string(),
            PyObject::Event(_) => "<Event>".to_string(),
            PyObject::Queue(_) => "<Queue>".to_string(),
            PyObject::Super { .. } => format!("<super object>"),
            PyObject::Property { .. } => format!("<property object>"),
            PyObject::StaticMethod { .. } => format!("<staticmethod object>"),
            PyObject::ClassMethod { .. } => format!("<classmethod object>"),
            PyObject::Generator { .. } => format!("<generator object>"),
            PyObject::Coroutine { .. } => format!("<coroutine object>"),
            PyObject::Array(arr) => {
                let items: Vec<String> = arr.data.iter().map(|v| {
                    if arr.typecode == 'i' {
                        py_int(*v as i64).repr()
                    } else {
                        py_float(*v).repr()
                    }
                }).collect();
                format!("array('{}', [{}])", arr.typecode, items.join(", "))
            },
            PyObject::CompiledRegex { pattern, .. } => format!("re.compile('{}')", pattern),
            PyObject::Closure(_) => "<builtin function>".to_string(),
            PyObject::FutureAwaitIterator { future, yielded } => {
                format!("<future_await_iterator future={} yielded={}>", future.repr(), yielded)
            }
        }
    }

    pub fn str(&self) -> String {
        match self {
            PyObject::Str(s) => s.to_string(),
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
            PyObject::Tuple(v) => !v.is_empty(),
            PyObject::Dict(d) => !d.is_empty(),
            PyObject::Set(s) => !s.is_empty(),
            PyObject::FrozenSet(s) => !s.is_empty(),
            PyObject::Range { start, stop, step } => *step > 0 && *start < *stop || *step < 0 && *start > *stop,
            PyObject::RangeIter { current, stop, step } => *step > 0 && *current < *stop || *step < 0 && *current > *stop,
            PyObject::EnumerateIter { items, pos, .. } => *pos < items.len(),
            PyObject::Instance { typ, .. } => {
                // Check for __bool__ method
                // If no __bool__, objects are truthy by default
                let f = {
                    let typ_ref = typ.borrow();
                    match &*typ_ref {
                        PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__bool__").cloned(),
                        _ => None,
                    }
                };
                if let Some(f) = f {
                    if let Ok(result) = call_bound_method(f, PyObjectRef::new(PyObject::Instance { typ: typ.clone(), dict: HashMap::new() }), vec![]) {
                        return result.truthy();
                    }
                }
                true
            }
            PyObject::Array(arr) => !arr.data.is_empty(),
            PyObject::CompiledRegex { .. } => true,
            PyObject::Closure(_) => true,
            _ => true,
        }
    }

    pub fn hash(&self) -> PyResult<usize> {
        match self {
            PyObject::None => Ok(0),
            PyObject::Bool(b) => Ok(if *b { 1 } else { 0 }),
            PyObject::Int(i) => {
                // Simple hash: take lower bits
                let bytes = i.to_signed_bytes_le();
                let mut h: usize = 0;
                for (j, &b) in bytes.iter().enumerate() {
                    h ^= (b as usize) << ((j % (std::mem::size_of::<usize>())) * 8);
                }
                Ok(h)
            }
            PyObject::Float(f) => {
                if f.is_nan() {
                    Ok(0)
                } else {
                    Ok(f.to_bits() as usize)
                }
            }
            PyObject::Str(s) => {
                let mut h: usize = 0;
                for c in s.chars() {
                    h = h.wrapping_mul(1000003).wrapping_add(c as usize);
                }
                Ok(h)
            }
            PyObject::Bytes(b) => {
                let mut h: usize = 0;
                for &byte in b { h = h.wrapping_mul(1000003).wrapping_add(byte as usize); }
                Ok(h)
            }
            PyObject::Range { start, stop, step } => {
                let mut h: usize = 0x123456;
                h = h.wrapping_mul(1000003).wrapping_add(*start as usize);
                h = h.wrapping_mul(1000003).wrapping_add(*stop as usize);
                h = h.wrapping_mul(1000003).wrapping_add(*step as usize);
                Ok(h)
            }
            PyObject::Tuple(items) => {
                let mut h: usize = 0x345678;
                for item in items {
                    h = h.wrapping_mul(1000003).wrapping_add(item.hash()?);
                }
                Ok(h)
            }
            PyObject::FrozenSet(items) => {
                let mut h: usize = 0x987654;
                for item in items.to_vec() {
                    h = h.wrapping_mul(1000003).wrapping_add(item.hash()?);
                }
                Ok(h)
            }
            PyObject::Instance { typ, dict } => {
                // Check for __hash__ method
                let f = {
                    let typ_ref = typ.borrow();
                    match &*typ_ref {
                        PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__hash__").cloned(),
                        _ => None,
                    }
                };
                if let Some(f) = f {
                    let result = call_bound_method(f, PyObjectRef::new(PyObject::Instance { typ: typ.clone(), dict: dict.clone() }), vec![])?;
                    let n = result.borrow();
                    if let PyObject::Int(i) = &*n {
                        let bytes = i.to_signed_bytes_le();
                        let mut h: usize = 0;
                        for (j, &b) in bytes.iter().enumerate() {
                            h ^= (b as usize) << ((j % (std::mem::size_of::<usize>())) * 8);
                        }
                        Ok(h)
                    } else {
                        Err(PyError::type_error("__hash__ should return an integer"))
                    }
                } else {
                    Err(PyError::type_error(format!("unhashable type: '{}'", self.type_name())))
                }
            }
            PyObject::Array(arr) => {
                let mut h: usize = 0xabcdef;
                for &v in &arr.data {
                    let bits = v.to_bits();
                    h = h.wrapping_mul(1000003).wrapping_add(bits as usize);
                }
                Ok(h)
            }
            PyObject::CompiledRegex { pattern, flags, .. } => {
                let mut h: usize = 0x123456;
                for b in pattern.bytes() {
                    h = h.wrapping_mul(1000003).wrapping_add(b as usize);
                }
                h = h.wrapping_mul(1000003).wrapping_add(*flags as usize);
                Ok(h)
            }
            PyObject::Closure(_) => Err(PyError::type_error(format!("unhashable type: '{}'", self.type_name()))),
            _ => Err(PyError::type_error(format!("unhashable type: '{}'", self.type_name()))),
        }
    }

    pub fn equals(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
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
                if a.len() != b.len() { false }
                else {
                    let mut eq = true;
                    for (k, va) in a.items() {
                        match b.get(&k) {
                            Ok(Some(vb)) => { if !va.equals(&vb).unwrap_or(false) { eq = false; break; } }
                            _ => { eq = false; break; }
                        }
                    }
                    eq
                }
            }
            (PyObject::List(a), PyObject::List(b)) => {
                let mut eq = true;
                if a.len() != b.len() { eq = false; }
                if eq {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !x.equals(y)? { eq = false; break; }
                    }
                }
                eq
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                let mut eq = true;
                if a.len() != b.len() { eq = false; }
                if eq {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !x.equals(y)? { eq = false; break; }
                    }
                }
                eq
            }
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() != b.len() { false }
                else {
                    let mut eq = true;
                    for item in a.to_vec() {
                        if !b.contains(&item).unwrap_or(false) { eq = false; break; }
                    }
                    eq
                }
            }
            (PyObject::FrozenSet(a), PyObject::FrozenSet(b)) => {
                if a.len() != b.len() { false }
                else {
                    let mut eq = true;
                    for item in a.to_vec() {
                        if !b.contains(&item).unwrap_or(false) { eq = false; break; }
                    }
                    eq
                }
            }
            (PyObject::Array(a), PyObject::Array(b)) => a.typecode == b.typecode && a.data == b.data,
            (PyObject::CompiledRegex { pattern: a, flags: af, .. }, PyObject::CompiledRegex { pattern: b, flags: bf, .. }) => a == b && af == bf,
            _ => false,
        };
        Ok(result)
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\"' => out.push_str("\\\""),
            '\x00'..='\x1f' => out.push_str(&format!("\\x{:02x}", c as u8)),
            '\x7f' => out.push_str("\\x7f"),
            c if c.is_control() => match c as u32 {
                code @ 0..=0xff => out.push_str(&format!("\\x{:02x}", code as u8)),
                code @ 0x100..=0xffff => out.push_str(&format!("\\u{:04x}", code)),
                code => out.push_str(&format!("\\U{:08x}", code)),
            },
            c => out.push(c),
        }
    }
    out
}

// ---- Builtin Object Constructors ----

thread_local! {
    pub static VM_PTR: std::cell::RefCell<Option<*mut super::vm::VirtualMachine>> = std::cell::RefCell::new(None);
}

/// Safely access the current VM via VM_PTR.
///
/// Returns `Err(runtime_error)` if no VM is active.
/// The single `unsafe` dereference for VM_PTR access lives here;
/// all callers use this safe wrapper instead of inline `unsafe { &*ptr }`.
pub fn with_vm_mut<F, R>(f: F) -> PyResult<R>
where
    F: FnOnce(&mut super::vm::VirtualMachine) -> R,
{
    VM_PTR.with(|p| {
        let opt = p.borrow();
        if let Some(ptr) = *opt {
            // SAFETY:
            // - VM_PTR is set in `VirtualMachine::execute()` before execution begins
            //   and remains valid for the duration of the call.
            // - It is only set on the current thread (thread_local!).
            // - The pointer is cleared after execution completes.
            // - Therefore, while we are inside a builtin function being called by the VM,
            //   the pointer is guaranteed to point to a live VirtualMachine.
            let vm = unsafe { &mut *ptr };
            Ok(f(vm))
        } else {
            Err(PyError::runtime_error("no active VM"))
        }
    })
}

thread_local! {
    static SMALL_INT_CACHE: std::cell::RefCell<Vec<Option<PyObjectRef>>> = std::cell::RefCell::new(vec![None; 263]);
}

pub fn py_int(i: impl Into<BigInt>) -> PyObjectRef {
    let big = i.into();
    if let Some(n) = big.to_i64() {
        if n >= -5 && n <= 257 {
            let idx = (n + 5) as usize;
            return SMALL_INT_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if let Some(ref cached) = cache[idx] {
                    cached.clone()
                } else {
                    let val = PyObjectRef::imm(PyObject::Int(BigInt::from(n)));
                    cache[idx] = Some(val.clone());
                    val
                }
            });
        }
        return PyObjectRef::SmallInt(n);
    }
    PyObjectRef::imm(PyObject::Int(big))
}

pub fn py_bool(b: bool) -> PyObjectRef {
    PyObjectRef::SmallBool(b)
}

pub fn py_none() -> PyObjectRef {
    PyObjectRef::None
}

/// Convert a Python object to a PySet by checking common iterable types.
/// Used as a replacement for the non-existent `py_set_from_iter`.
pub fn convert_to_set(obj: &PyObjectRef) -> PyResult<PySet> {
    let borrowed = obj.borrow();
    match &*borrowed {
        PyObject::Set(s) => Ok(s.clone()),
        PyObject::FrozenSet(s) => Ok(s.clone()),
        PyObject::List(v) => Ok(PySet::from_vec(v.clone())?),
        PyObject::Tuple(items) => Ok(PySet::from_vec(items.clone())?),
        PyObject::Str(s) => {
            let chars: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
            Ok(PySet::from_vec(chars)?)
        }
        _ => Err(PyError::type_error(format!(
            "cannot convert '{}' to set", borrowed.type_name()
        ))),
    }
}

pub fn py_float(f: f64) -> PyObjectRef {
    // Use inline SmallFloat to avoid Rc + heap alloc
    PyObjectRef::SmallFloat(f)
}

pub fn py_str(s: &str) -> PyObjectRef {
    // Use inline SmallStr for strings < 16 bytes to avoid Rc + heap alloc
    if let Some(small) = SmallStr::new(s) {
        return PyObjectRef::SmallStr(small);
    }
    PyObjectRef::imm(PyObject::Str(compact_str::CompactString::from(s)))
}

pub fn py_list(items: Vec<PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::List(items))
}

pub fn py_tuple(items: Vec<PyObjectRef>) -> PyObjectRef {
    PyObjectRef::imm(PyObject::Tuple(items))
}

pub fn py_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::Dict(PyDict::new()))
}

pub fn py_set() -> PyObjectRef {
    PyObjectRef::new(PyObject::Set(PySet::new()))
}

/// printf-style string interpolation (% operator)
fn string_interpolate(fmt: &str, arg: &PyObjectRef) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = fmt.chars();

    // Handle tuple/list arguments: consume one element per format spec
    let mut arg_iter: Option<Box<dyn Iterator<Item = PyObjectRef>>> = None;
    let arg0 = arg.clone();
    {
        let obj = arg0.borrow();
        match &*obj {
            PyObject::Tuple(items) => {
                let vec = items.clone();
                let iter = vec.into_iter();
                arg_iter = Some(Box::new(iter));
            }
            PyObject::List(items) => {
                let vec = items.clone();
                let iter = vec.into_iter();
                arg_iter = Some(Box::new(iter));
            }
            _ => {}
        }
    }
    // Helper: get next arg (consume from tuple/list iterator, or always use the single arg)
    let mut get_arg = || -> PyObjectRef {
        if let Some(ref mut it) = arg_iter {
            it.next().unwrap_or_else(|| py_str(""))
        } else {
            arg.clone()
        }
    };

    while let Some(ch) = chars.next() {
        if ch == '%' {
            // Parse optional width specifier (e.g., %03o, %02d, %3s, %4d)
            let mut width: Option<usize> = None;
            // Check for flags (only '0' flag supported)
            let mut flags = String::new();
            let mut peek = chars.clone();
            if let Some(c) = peek.as_str().chars().next() {
                if c == '0' {
                    flags.push('0');
                    chars.next();
                }
            }
            // Parse width (digits)
            let mut width_str = String::new();
            loop {
                let mut peek2 = chars.clone();
                match peek2.next() {
                    Some(c) if c.is_ascii_digit() => { width_str.push(c); chars.next(); }
                    _ => break,
                }
            }
            if !width_str.is_empty() {
                width = Some(width_str.parse::<usize>().map_err(|_| "invalid width".to_string())?);
            }

            match chars.next() {
                None => return Err("incomplete format: trailing %".to_string()),
                Some('%') => result.push('%'),
                Some(conv @ 's') | Some(conv @ 'r') | Some(conv @ 'f') | Some(conv @ 'd') | Some(conv @ 'i')
                | Some(conv @ 'o') | Some(conv @ 'x') | Some(conv @ 'X') => {
                    let raw = get_arg();

                    let formatted = match conv {
                        's' => raw.str(),
                        'r' => format!("'{}'", raw.str()),
                        'f' => raw.str(),
                        'd' | 'i' => {
                            if let Some(i) = raw.as_i64() {
                                i.to_string()
                            } else {
                                "0".to_string()
                            }
                        }
                        'o' => {
                            if let Some(i) = raw.as_i64() {
                                format!("{:o}", i)
                            } else {
                                "0".to_string()
                            }
                        }
                        'x' => {
                            if let Some(i) = raw.as_i64() {
                                format!("{:x}", i)
                            } else {
                                "0".to_string()
                            }
                        }
                        'X' => {
                            if let Some(i) = raw.as_i64() {
                                format!("{:X}", i)
                            } else {
                                "0".to_string()
                            }
                        }
                        _ => unreachable!(),
                    };

                    // Apply width
                    let padded = if let Some(w) = width {
                        if flags.contains('0') {
                            format!("{:0>width$}", formatted, width = w)
                        } else {
                            format!("{:>width$}", formatted, width = w)
                        }
                    } else {
                        formatted
                    };
                    result.push_str(&padded);
                }
                Some(c) => return Err(format!("unsupported format character '{}'", c)),
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

// ---- Unary Operations ----

pub fn py_neg(val: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(i) = val.as_i64() {
        return Ok(py_int(-i));
    }
    if let Some(f) = val.as_f64() {
        return Ok(py_float(-f));
    }
    let obj = val.borrow();
    match &*obj {
        PyObject::Int(n) => Ok(py_int(-n.clone())),
        PyObject::Float(n) => Ok(py_float(-n)),
        _ => Err(PyError::type_error(format!("bad operand type for unary -: '{}'", obj.type_name()))),
    }
}

pub fn py_not(val: &PyObjectRef) -> PyObjectRef {
    py_bool(!val.truthy())
}

// ---- Binary Operations ----

pub fn try_dunder_binop(a: &PyObjectRef, b: &PyObjectRef, method: &str) -> PyResult<Option<PyObjectRef>> {
    let f = {
        let a_borrowed = a.borrow();
        match &*a_borrowed {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get(method).cloned(),
                    _ => None,
                }
            }
            _ => {
                // Try dunder method directly on the object (for builtin types like str, list, etc.)
                a_borrowed.get_attribute(method).ok()
            }
        }
    };
    if let Some(f) = f {
        let result = call_bound_method(f, a.clone(), vec![b.clone()])?;
        Ok(Some(result))
    } else {
        Ok(None)
    }
}

pub fn py_add(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Ok(py_int(ai + bi));
    }
    if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
        return Ok(py_float(af + bf));
    }
    if let Some(r) = try_dunder_binop(a, b, "__add__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() + b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a + b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() + b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a + b.to_f64().unwrap())),
        (PyObject::Str(a), PyObject::Str(b)) => Ok(py_str(&format!("{}{}", a, b))),
        (PyObject::List(a), PyObject::List(b)) => {
            let mut v = a.clone();
            v.extend(b.clone());
            Ok(py_list(v))
        }
        (PyObject::Tuple(a), PyObject::Tuple(b)) => {
            let mut v = a.clone();
            v.extend(b.clone());
            Ok(py_tuple(v))
        }
        (PyObject::Bytes(a), PyObject::Bytes(b)) => {
            let mut v = a.clone();
            v.extend(b);
            Ok(PyObjectRef::imm(PyObject::Bytes(v)))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for +: '{}' and '{}'", 
            a_obj.type_name(), b_obj.type_name())))
    }
}

pub fn py_sub(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Ok(py_int(ai - bi));
    }
    if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
        return Ok(py_float(af - bf));
    }
    if let Some(r) = try_dunder_binop(a, b, "__sub__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() - b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a - b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() - b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a - b.to_f64().unwrap())),
        (PyObject::Set(a), PyObject::Set(b)) => set_difference(a, b),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for -: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_mul(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Ok(py_int(ai * bi));
    }
    if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
        return Ok(py_float(af * bf));
    }
    if let Some(r) = try_dunder_binop(a, b, "__mul__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() * b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a * b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() * b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a * b.to_f64().unwrap())),
        (PyObject::Str(s), PyObject::Int(n)) => {
            if let Some(n) = n.to_usize() {
                Ok(py_str(&s.repeat(n)))
            } else {
                Err(PyError::value_error("cannot multiply string by negative number"))
            }
        }
        (PyObject::Int(n), PyObject::Str(s)) => {
            if let Some(n) = n.to_usize() {
                Ok(py_str(&s.repeat(n)))
            } else {
                Err(PyError::value_error("cannot multiply string by negative number"))
            }
        }
        (PyObject::List(v), PyObject::Int(n)) => {
            if let Some(n) = n.to_usize() {
                let mut result = Vec::new();
                for _ in 0..n {
                    result.extend(v.clone());
                }
                Ok(py_list(result))
            } else {
                Err(PyError::value_error("cannot multiply list by negative number"))
            }
        }
        (PyObject::Tuple(v), PyObject::Int(n)) => {
            if let Some(n) = n.to_usize() {
                let mut result = Vec::new();
                for _ in 0..n {
                    result.extend(v.clone());
                }
                Ok(py_tuple(result))
            } else {
                Err(PyError::value_error("cannot multiply tuple by negative number"))
            }
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for *: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_div(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 { return Err(PyError::zero_division()); }
        return Ok(py_float(ai as f64 / bi as f64));
    }
    if let Some(r) = try_dunder_binop(a, b, "__truediv__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            Ok(py_float(a.to_f64().unwrap() / b.to_f64().unwrap()))
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float(a / b))
        }
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float(a.to_f64().unwrap() / b))
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            Ok(py_float(a / b.to_f64().unwrap()))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for /: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_floor_div(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 { return Err(PyError::zero_division()); }
        let q = ai / bi; let r = ai % bi;
        return if r != 0 && ((ai < 0) != (bi < 0)) { Ok(py_int(q - 1)) } else { Ok(py_int(q)) };
    }
    if let Some(r) = try_dunder_binop(a, b, "__floordiv__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            if a.sign() == Sign::Minus && &(a % b) != &BigInt::zero() {
                Ok(py_int((a / b) - 1))
            } else {
                Ok(py_int(a / b))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float((a / b).floor()))
        }
        (PyObject::Int(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float((a.to_f64().unwrap() / b).floor()))
        }
        (PyObject::Float(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            Ok(py_float((a / b.to_f64().unwrap()).floor()))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for //: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_mod(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi == 0 { return Err(PyError::zero_division()); }
        let rem = ai % bi;
        return if rem < 0 { Ok(py_int(rem + bi)) } else { Ok(py_int(rem)) };
    }
    if let Some(r) = try_dunder_binop(a, b, "__mod__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            let rem = a % b;
            if rem.sign() == Sign::Minus {
                Ok(py_int(rem + b))
            } else {
                Ok(py_int(rem))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => {
            if *b == 0.0 { return Err(PyError::zero_division()); }
            Ok(py_float(a % b))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for %: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_pow(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return Ok(py_float((ai as f64).powi(bi as i32))); }
        if bi == 0 { return Ok(py_int(1)); }
        if bi == 1 { return Ok(py_int(ai)); }
        if bi > 63 {
            // Use BigInt for large exponents to avoid overflow
            let big_a = BigInt::from(ai);
            let big_b = BigInt::from(bi);
            let result = big_a.pow(bi as u32);
            return Ok(py_int(result));
        }
        let mut result: i64 = 1;
        for _ in 0..bi { result = result.wrapping_mul(ai); }
        return Ok(py_int(result));
    }
    if let Some(r) = try_dunder_binop(a, b, "__pow__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if let Some(exp) = b.to_usize() {
                let result = a.pow(exp as u32);
                Ok(py_int(result))
            } else if b.is_zero() {
                Ok(py_int(BigInt::one()))
            } else if b.sign() == Sign::Minus {
                // For now, return float
                let f = a.to_f64().unwrap().powf(b.to_f64().unwrap());
                Ok(py_float(f))
            } else {
                Err(PyError::value_error("int too large to convert to int"))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a.powf(*b))),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap().powf(*b))),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a.powf(b.to_f64().unwrap()))),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for **: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_lshift(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return Err(PyError::value_error("negative shift count")); }
        return Ok(py_int(ai.wrapping_shl(bi as u32)));
    }
    if let Some(r) = try_dunder_binop(a, b, "__lshift__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            let shift = b.to_usize().ok_or_else(|| PyError::value_error("negative shift count"))?;
            Ok(py_int(a << shift))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for <<: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_rshift(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 { return Err(PyError::value_error("negative shift count")); }
        if bi >= 64 { return Ok(py_int(if ai < 0 { -1i64 } else { 0i64 })); }
        return Ok(py_int(ai.wrapping_shr(bi as u32)));
    }
    if let Some(r) = try_dunder_binop(a, b, "__rshift__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            let shift = b.to_usize().ok_or_else(|| PyError::value_error("negative shift count"))?;
            Ok(py_int(a >> shift))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for >>: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

fn set_union(a: &PySet, b: &PySet) -> PyResult<PyObjectRef> {
    let mut result = a.clone();
    for item in b.to_vec() { result.add(item)?; }
    Ok(PyObjectRef::new(PyObject::Set(result)))
}
fn set_intersection(a: &PySet, b: &PySet) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() { if b.contains(&item)? { result.add(item)?; } }
    Ok(PyObjectRef::new(PyObject::Set(result)))
}
fn set_difference(a: &PySet, b: &PySet) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() { if !b.contains(&item)? { result.add(item)?; } }
    Ok(PyObjectRef::new(PyObject::Set(result)))
}
fn set_symmetric_diff(a: &PySet, b: &PySet) -> PyResult<PyObjectRef> {
    let mut result = PySet::new();
    for item in a.to_vec() { if !b.contains(&item)? { result.add(item)?; } }
    for item in b.to_vec() { if !a.contains(&item)? { result.add(item)?; } }
    Ok(PyObjectRef::new(PyObject::Set(result)))
}

fn i64_binop(a: &PyObjectRef, b: &PyObjectRef, f: impl Fn(i64, i64) -> i64) -> Option<PyResult<PyObjectRef>> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Some(Ok(py_int(f(ai, bi))));
    }
    None
}

pub fn py_bit_or(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(r) = i64_binop(a, b, |x, y| x | y) { return r; }
    if let Some(r) = try_dunder_binop(a, b, "__or__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() | b)),
        (PyObject::Set(a), PyObject::Set(b)) => set_union(a, b),
        (PyObject::Dict(a), PyObject::Dict(b)) => {
            let mut merged = PyDict::new();
            for k in a.keys() {
                if let Ok(Some(v)) = a.get(&k) { merged.set(k, v)?; }
            }
            for k in b.keys() {
                if let Ok(Some(v)) = b.get(&k) { merged.set(k, v)?; }
            }
            Ok(PyObjectRef::new(PyObject::Dict(merged)))
        }
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for |: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_bit_xor(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(r) = i64_binop(a, b, |x, y| x ^ y) { return r; }
    if let Some(r) = try_dunder_binop(a, b, "__xor__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() ^ b)),
        (PyObject::Set(a), PyObject::Set(b)) => set_symmetric_diff(a, b),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for ^: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_bit_and(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(r) = i64_binop(a, b, |x, y| x & y) { return r; }
    if let Some(r) = try_dunder_binop(a, b, "__and__")? { return Ok(r); }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() & b)),
        (PyObject::Set(a), PyObject::Set(b)) => set_intersection(a, b),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for &: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

// ---- Comparison Operations ----

pub fn py_compare(a: &PyObjectRef, b: &PyObjectRef, op: u32) -> PyResult<PyObjectRef> {
    // Fast path for small int comparisons — no borrow() needed
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Ok(py_bool(match op {
            0 => ai < bi,
            1 => ai <= bi,
            2 => ai == bi,
            3 => ai >= bi,
            4 => ai > bi,
            5 => ai != bi,
            _ => return Ok(py_bool(false)),
        }));
    }
    // Check for __eq__/__ne__ on Instance types
    if op == 2 || op == 5 {
        let is_a_instance = matches!(&*a.borrow(), PyObject::Instance { .. });
        let is_b_instance = matches!(&*b.borrow(), PyObject::Instance { .. });
        if is_a_instance || is_b_instance {
            let method_name = if op == 2 { "__eq__" } else { "__ne__" };
            if let Some(result) = try_dunder_comparison(a, b, method_name)? {
                return Ok(py_bool(result));
            }
        }
    }
    let result = match op {
        0 => a.borrow().lt(b)?,
        1 => a.borrow().le(b)?,
        2 => a.borrow().equals(b)?,
        3 => a.borrow().ge(b)?,
        4 => a.borrow().gt(b)?,
        5 => a.borrow().ne(b)?,
        6 => contains_op(b, a)?,
        7 => !contains_op(b, a)?,
        8 => a.is(b),
        9 => !a.is(b),
        _ => return Err(PyError::runtime_error("unknown comparison operator")),
    };
    Ok(py_bool(result))
}

fn try_dunder_comparison(a: &PyObjectRef, b: &PyObjectRef, method: &str) -> PyResult<Option<bool>> {
    // Try a.__eq__(b) first
    let f_a = try_get_method(a, method);
    if let Some(f) = f_a {
        let result = call_bound_method(f, a.clone(), vec![b.clone()])?;
        return Ok(Some(result.truthy()));
    }
    // Try b.__eq__(a) if different type
    if a.get_type_name() != b.get_type_name() {
        let f_b = try_get_method(b, method);
        if let Some(f) = f_b {
            let result = call_bound_method(f, b.clone(), vec![a.clone()])?;
            return Ok(Some(result.truthy()));
        }
    }
    Ok(None)
}

fn try_get_method(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    let obj_borrowed = obj.borrow();
    match &*obj_borrowed {
        PyObject::Instance { typ, .. } => {
            let typ_ref = typ.borrow();
            match &*typ_ref {
                PyObject::Type { dict: type_dict, .. } => type_dict.get_str(&name).cloned(),
                _ => None,
            }
        }
        _ => None,
    }
}

pub trait Compare {
    fn lt(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn le(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn gt(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn ge(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn ne(&self, other: &PyObjectRef) -> PyResult<bool>;
}

impl Compare for PyObject {
    fn lt(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a < b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a < b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() < *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a < b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a < b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a < b),
            (PyObject::Bool(a), PyObject::Int(b)) => Ok((*a as i32) < b.to_i32().unwrap_or(0)),
            (PyObject::Int(a), PyObject::Bool(b)) => Ok(a.to_i32().unwrap_or(0) < (*b as i32)),
            (PyObject::Set(a), PyObject::Set(b)) => {
                // a < b: proper subset (a <= b and a != b)
                if a.len() >= b.len() { return Ok(false); }
                for item in a.to_vec() { if !b.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return x.borrow().lt(y); }
                }
                Ok(a.len() < b.len())
            }
            (PyObject::None, PyObject::None) => Ok(false),
            _ => Err(PyError::type_error(format!("'<' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()))),
        }
    }

    fn le(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a <= b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a <= b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() <= *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a <= b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a <= b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a <= b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() > b.len() { return Ok(false); }
                for item in a.to_vec() { if !b.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return x.borrow().lt(y); }
                }
                Ok(a.len() <= b.len())
            }
            _ => Err(PyError::type_error(format!("'<=' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()))),
        }
    }

    fn gt(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a > b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a > b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() > *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a > b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a > b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a > b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() <= b.len() { return Ok(false); }
                for item in b.to_vec() { if !a.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return x.borrow().gt(y); }
                }
                Ok(a.len() > b.len())
            }
            _ => Err(PyError::type_error(format!("'>' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()))),
        }
    }

    fn ge(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a >= b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a >= b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() >= *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a >= b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a >= b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a >= b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() < b.len() { return Ok(false); }
                for item in b.to_vec() { if !a.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return x.borrow().gt(y); }
                }
                Ok(a.len() >= b.len())
            }
            _ => Err(PyError::type_error(format!("'>=' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()))),
        }
    }
    
    fn ne(&self, other: &PyObjectRef) -> PyResult<bool> {
        self.equals(other).map(|b| !b)
    }
}

// ---- Containment ----

pub fn contains_op(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
    // Check for __contains__ on instances
    let f = {
        let container = a.borrow();
        match &*container {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, mro, .. } => {
                        type_dict.get_str("__contains__").cloned().or_else(|| {
                            for base in mro.iter() {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if let Some(val) = base_dict.get_str("__contains__") {
                                        return Some(val.clone());
                                    }
                                }
                            }
                            None
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        let result = call_bound_method(f, a.clone(), vec![b.clone()])?;
        return Ok(result.truthy());
    }
    let container = a.borrow();
    match &*container {
        PyObject::Str(s) => {
            let item_str = b.str();
            Ok(s.contains(&item_str))
        }
        PyObject::List(items) => {
            for item in items {
                if item.equals(b)? { return Ok(true); }
            }
            Ok(false)
        }
        PyObject::Tuple(items) => {
            for item in items {
                if item.equals(b)? { return Ok(true); }
            }
            Ok(false)
        }
        PyObject::Dict(d) => {
            d.contains(b)
        }
        PyObject::Set(items) => {
            items.contains(b)
        }
        PyObject::FrozenSet(items) => items.contains(b),
        PyObject::Range { start, stop, step } => {
            let item = b.borrow();
            if let PyObject::Int(n) = &*item {
                let n = n.to_i64().unwrap_or(0);
                if *step > 0 { Ok(n >= *start && n < *stop && (n - *start) % *step == 0) }
                else { Ok(n <= *start && n > *stop && (n - *start) % *step == 0) }
            } else { Ok(false) }
        }
        _ => Err(PyError::type_error(format!("argument of type '{}' is not iterable", container.type_name()))),
    }
}

pub fn py_contains(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    contains_op(a, b).map(py_bool)
}

// ---- Built-in functions ----

pub fn builtin_print(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let strings: Vec<String> = args.iter().map(|a| a.str()).collect();
    println!("{}", strings.join(" "));
    Ok(py_none())
}

pub fn builtin_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("len() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Str(s) => Ok(py_int(s.chars().count())),
        PyObject::List(v) => Ok(py_int(v.len())),
        PyObject::Tuple(v) => Ok(py_int(v.len())),
        PyObject::Dict(d) => Ok(py_int(d.len())),
        PyObject::Set(s) => Ok(py_int(s.len())),
        PyObject::FrozenSet(s) => Ok(py_int(s.len())),
        PyObject::Range { start, stop, step } => {
            if *step > 0 && *start >= *stop { Ok(py_int(0)) }
            else if *step < 0 && *start <= *stop { Ok(py_int(0)) }
            else {
                let len = ((*stop - *start) / *step) as i64;
                if (*stop - *start) % *step != 0 { Ok(py_int(len.abs() + 1)) }
                else { Ok(py_int(len.abs())) }
            }
        }
        PyObject::Bytes(b) => Ok(py_int(b.len())),
        PyObject::ByteArray(b) => Ok(py_int(b.len())),
        PyObject::Array(arr) => Ok(py_int(arr.data.len())),
        PyObject::Instance { typ, .. } => {
            let f = {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__len__").cloned(),
                    _ => None,
                }
            };
            if let Some(f) = f {
                let result = call_bound_method(f, args[0].clone(), vec![])?;
                let n = result.borrow();
                if let PyObject::Int(i) = &*n { return Ok(py_int(i.clone())) }
                return Err(PyError::type_error("__len__() should return an int"))
            }
            Err(PyError::type_error(format!("object of type '{}' has no len()", obj.type_name())))
        }
        _ => Err(PyError::type_error(format!("object of type '{}' has no len()", obj.type_name()))),
    }
}

pub fn builtin_range(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        1 => {
            let stop = args[0].borrow();
            if let PyObject::Int(n) = &*stop {
                let stop = n.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                Ok(PyObjectRef::imm(PyObject::Range { start: 0, stop, step: 1 }))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        2 => {
            let start = args[0].borrow();
            let stop = args[1].borrow();
            if let (PyObject::Int(a), PyObject::Int(b)) = (&*start, &*stop) {
                let a = a.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                let b = b.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                Ok(PyObjectRef::imm(PyObject::Range { start: a, stop: b, step: 1 }))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        3 => {
            let start = args[0].borrow();
            let stop = args[1].borrow();
            let step = args[2].borrow();
            if let (PyObject::Int(a), PyObject::Int(b), PyObject::Int(s)) = (&*start, &*stop, &*step) {
                let a = a.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                let b = b.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                let s = s.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                if s == 0 { return Err(PyError::value_error("range() arg 3 must not be zero")); }
                Ok(PyObjectRef::imm(PyObject::Range { start: a, stop: b, step: s }))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        _ => Err(PyError::type_error("range() takes at most 3 arguments")),
    }
}

pub fn builtin_type_of(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() == 1 {
        // type(obj) -> return the type of an object
        let borrowed = args[0].borrow();
        match &*borrowed {
            PyObject::Instance { typ, .. } => Ok(typ.clone()),
            PyObject::Type { .. } => Ok(args[0].clone()),
            _ => {
                let name = borrowed.type_name();
                Ok(PyObjectRef::new(PyObject::Type {
                    name,
                    dict: HashMap::new(),
                    bases: vec![],
                    mro: vec![],
                }))
            }
        }
    } else if args.len() == 3 {
        // type(name, bases, dict) -> create a new class (metaclass usage)
        let name_str = args[0].str();
        let bases_vec = if let PyObject::Tuple(t) = &*args[1].borrow() {
            t.clone()
        } else if matches!(&*args[1].borrow(), PyObject::None) {
            vec![]
        } else {
            vec![args[1].clone()]
        };
        let namespace_dict = {
            let b = args[2].borrow();
            match &*b {
                PyObject::Dict(d) => {
                    let mut h = HashMap::new();
                    for (k, v) in d.items() {
                        h.insert(k.str(), v);
                    }
                    h
                }
                _ => return Err(PyError::type_error("type() third argument must be a dict")),
            }
        };
        // Look up the 'object' type from builtins to use as default base
        // Since we're in object.rs we can't easily access builtins, so we
        // create the class without implicit object base if no bases given
        let class = PyObjectRef::new(PyObject::Type {
            name: name_str,
            dict: namespace_dict,
            bases: bases_vec.clone(),
            mro: vec![],
        });
        // Simple MRO: class itself + each base's linearization
        let mut mro = vec![class.clone()];
        for base in &bases_vec {
            let base_mro = match &*base.borrow() {
                PyObject::Type { mro: b_mro, .. } if !b_mro.is_empty() => b_mro.clone(),
                _ => vec![base.clone()],
            };
            for item in base_mro {
                if !mro.iter().any(|m| m.is(&item)) {
                    mro.push(item);
                }
            }
        }
        if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
            *mro_field = mro;
        }
        Ok(class)
    } else {
        Err(PyError::type_error("type() takes exactly one or three arguments"))
    }
}

pub fn builtin_int(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() > 0 {
        let t = args[0].borrow().type_name().to_string();
        if t == "instance" {
        }
    }
    if args.is_empty() { return Ok(py_int(0)); }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(_) => Ok(args[0].clone()),
        PyObject::Float(f) => Ok(py_int(*f as i64)),
        PyObject::Str(s) => {
            let s_trim = s.trim();
            // Remove underscores (Python visual separator, e.g. "0xFF_FF" or "1_000_000")
            let s_clean: String = s_trim.chars().filter(|&c| c != '_').collect();
            // Split optional sign from body
            let (sign, body) = match s_clean.as_bytes().first() {
                Some(b'-') => (-1, &s_clean[1..]),
                Some(b'+') => (1, &s_clean[1..]),
                _ => (1, &s_clean[..]),
            };
            let make_err = || PyError::value_error(format!("invalid literal for int(): '{}'", s));
            let obj = if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
                if let Ok(n) = i64::from_str_radix(oct, 8) { py_int(sign * n) }
                else if let Some(n) = BigInt::parse_bytes(oct.as_bytes(), 8) { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            } else if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
                if let Ok(n) = i64::from_str_radix(hex, 16) { py_int(sign * n) }
                else if let Some(n) = BigInt::parse_bytes(hex.as_bytes(), 16) { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            } else if let Some(bin) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
                if let Ok(n) = i64::from_str_radix(bin, 2) { py_int(sign * n) }
                else if let Some(n) = BigInt::parse_bytes(bin.as_bytes(), 2) { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            } else if args.len() > 1 {
                // int(x, base): parse x in given base
                drop(obj);
                let base_val = args[1].borrow();
                let base = if let PyObject::Int(i) = &*base_val { i.to_i64().unwrap_or(10) as u32 }
                    else { return Err(PyError::type_error("int() base must be an integer")) };
                if base < 2 || base > 36 {
                    return Err(PyError::value_error("int() base must be >= 2 and <= 36"));
                }
                // Re-borrow the string
                let obj2 = args[0].borrow();
                if let PyObject::Str(s) = &*obj2 {
                    let s_trim = s.trim();
                    let s_clean: String = s_trim.chars().filter(|&c| c != '_').collect();
                    let (sign, body) = match s_clean.as_bytes().first() {
                        Some(b'-') => (-1, &s_clean[1..]),
                        Some(b'+') => (1, &s_clean[1..]),
                        _ => (1, &s_clean[..]),
                    };
                    if let Ok(n) = i64::from_str_radix(body, base) { return Ok(py_int(sign * n)); }
                    else if let Some(n) = BigInt::parse_bytes(body.as_bytes(), base) { return Ok(py_int(if sign < 0 { -n } else { n })); }
                    else { return Err(PyError::value_error(format!("invalid literal for int(): '{}'", s))); }
                } else {
                    return Err(PyError::type_error("int() can convert strings only with base"));
                }
            } else {
                if let Ok(n) = body.parse::<i64>() { py_int(sign * n) }
                else if let Ok(n) = body.parse::<BigInt>() { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            };
            Ok(obj)
        }
        PyObject::Bool(b) => Ok(py_int(if *b { 1 } else { 0 })),
        PyObject::Instance { dict, typ } => {
            drop(obj);
            // Try calling __int__ method on the instance
            let args0 = &args[0];
            if let Ok(int_method) = args0.borrow().get_attribute("__int__") {
                let instance = args[0].clone();
                let result = builtin_call(&int_method, &[instance]);
                if let Ok(val) = result {
                    if let Some(n) = val.as_i64() {
                        return Ok(py_int(n));
                    }
                    // Maybe it returns a BigInt
                    let is_int = matches!(&*val.borrow(), PyObject::Int(_));
                    if is_int {
                        return Ok(val);
                    }
                }
            }
            Err(PyError::type_error(format!("int() argument must be a string or number, not '{}'", 
                args0.borrow().type_name())))
        }
        _ => Err(PyError::type_error(format!("int() argument must be a string or number, not '{}'", obj.type_name()))),
    }
}

/// int.from_bytes(bytes, byteorder, *, signed=False)
pub fn builtin_int_from_bytes(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("int.from_bytes() needs at least 2 arguments"));
    }
    let bytes_val = &args[0];
    let byteorder = &args[1];
    let order_str = byteorder.str();
    let big_endian = order_str == "big";
    let byte_data: Vec<u8> = match &*bytes_val.borrow() {
        PyObject::Bytes(b) => b.clone(),
        PyObject::List(items) => {
            items.iter().map(|x| x.as_i64().unwrap_or(0) as u8).collect()
        }
        _ => {
            let mut v = Vec::new();
            if let Ok(it) = builtin_iter(&[bytes_val.clone()]) {
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(x) => v.push(x.as_i64().unwrap_or(0) as u8),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            v
        }
    };
    let n = if big_endian {
        byte_data.iter().fold(0i64, |acc, &b| (acc << 8) | b as i64)
    } else {
        byte_data.iter().rev().fold(0i64, |acc, &b| (acc << 8) | b as i64)
    };
    Ok(py_int(n))
}

pub fn builtin_float(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Ok(py_float(0.0)); }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0))),
        PyObject::Float(f) => Ok(py_float(*f)),
        PyObject::Str(s) => {
            let f: f64 = s.trim().parse().map_err(|_| PyError::value_error(format!("could not convert string to float: '{}'", s)))?;
            Ok(py_float(f))
        }
        _ => Err(PyError::type_error(format!("float() argument must be a string or number, not '{}'", obj.type_name()))),
    }
}

pub fn builtin_str(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_str("")) }
    else {
        let f = {
            let obj_borrowed = args[0].borrow();
            if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                let typ_ref = typ.borrow();
                if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                    // Check own dict first
                    let own = type_dict.get_str("__str__").cloned();
                    if let Some(f) = own { Some(f) }
                    else {
                        // Walk MRO for inherited __str__
                        let mut found = None;
                        for base_type in mro {
                            if let PyObject::Type { dict: base_dict, .. } = &*base_type.borrow() {
                                if let Some(f) = base_dict.get_str("__str__") {
                                    found = Some(f.clone());
                                    break;
                                }
                            }
                        }
                        found
                    }
                } else { None }
            } else { None }
        };
        if let Some(f) = f {
            return call_bound_method(f, args[0].clone(), vec![]);
        }
        Ok(py_str(&args[0].str()))
    }
}

pub fn builtin_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("repr() takes exactly one argument"));
    }
    let f = {
        let obj_borrowed = args[0].borrow();
        match &*obj_borrowed {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__repr__").cloned(),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![]);
    }
    Ok(py_str(&args[0].repr()))
}

pub fn builtin_bool(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_bool(false)) }
    else { Ok(py_bool(args[0].truthy())) }
}

pub fn builtin_list(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_list(Vec::new())) }
    else {
        // Convert iterable to list
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => Ok(py_list(v.clone())),
            PyObject::Tuple(v) => Ok(py_list(v.clone())),
            PyObject::Str(s) => {
                let items: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                Ok(py_list(items))
            }
            PyObject::Set(s) => Ok(py_list(s.to_vec())),
            _ => {
                drop(obj);
                // Try general iteration protocol via iter() + next()
                let it = match builtin_iter(&[args[0].clone()]) {
                    Ok(it) => it,
                    Err(_) => return Err(PyError::type_error(format!("cannot convert '{}' object to list", args[0].borrow().type_name()))),
                };
                let mut collected = Vec::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(val) => collected.push(val),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(py_list(collected))
            }
        }
    }
}

pub fn builtin_tuple(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_tuple(Vec::new())) }
    else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => Ok(py_tuple(v.clone())),
            PyObject::Tuple(v) => Ok(py_tuple(v.clone())),
            PyObject::Str(s) => {
                let items: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                Ok(py_tuple(items))
            }
            _ => Err(PyError::type_error(format!("cannot convert '{}' to tuple", obj.type_name()))),
        }
    }
}

pub fn builtin_dict(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(py_dict())
}

pub fn builtin_set(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_set()) }
    else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => Ok(PyObjectRef::new(PyObject::Set(PySet::from_vec(v.clone())?))),
            PyObject::Tuple(v) => Ok(PyObjectRef::new(PyObject::Set(PySet::from_vec(v.clone())?))),
            PyObject::Range { start, stop, step } => {
                let mut elts = Vec::new();
                let mut i = *start;
                if *step > 0 {
                    while i < *stop { elts.push(py_int(i)); i += step; }
                } else {
                    while i > *stop { elts.push(py_int(i)); i += step; }
                }
                Ok(PyObjectRef::new(PyObject::Set(PySet::from_vec(elts)?)))
            }
            PyObject::Str(s) => {
                let elts: Vec<PyObjectRef> = s.chars().map(|c| py_int(c as i64)).collect();
                Ok(PyObjectRef::new(PyObject::Set(PySet::from_vec(elts)?)))
            }
            PyObject::Bytes(b) => {
                let elts: Vec<PyObjectRef> = b.iter().map(|&c| py_int(c as i64)).collect();
                Ok(PyObjectRef::new(PyObject::Set(PySet::from_vec(elts)?)))
            }
            PyObject::Set(s) => Ok(PyObjectRef::new(PyObject::Set(s.clone()))),
            PyObject::FrozenSet(s) => Ok(PyObjectRef::new(PyObject::Set(s.clone()))),
            _ => Err(PyError::type_error(format!("cannot convert '{}' to set", obj.type_name()))),
        }
    }
}

pub fn builtin_bytes(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new()))) }
    else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(i) => {
                let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                if n < 0 || n > 255 {
                    return Err(PyError::value_error("bytes() requires int in range 0-255"));
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(vec![n as u8])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::imm(PyObject::Bytes(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Set(items) => {
                let vec = items.to_vec();
                let mut result = Vec::new();
                for item in vec {
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be integer or iterable"));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::FrozenSet(items) => {
                let vec = items.to_vec();
                let mut result = Vec::new();
                for item in vec {
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be integer or iterable"));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            _ => Err(PyError::type_error(format!("cannot convert '{}' to bytes", obj.type_name()))),
        }
    }
}

/// bytes.fromhex(string) -> bytes
///
/// Create a bytes object from a string of hexadecimal digits.
pub fn builtin_bytes_fromhex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("bytes.fromhex() takes exactly 1 argument (0 given)"));
    }
    let s = args[0].str();
    // Remove spaces (CPython allows spaces in the hex string)
    let s = s.replace(' ', "");
    if s.len() % 2 != 0 {
        return Err(PyError::value_error("hex string must be of even length"));
    }
    let mut result = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hex_pair = std::str::from_utf8(chunk).map_err(|_| {
            PyError::value_error("non-hexadecimal number found")
        })?;
        let byte = u8::from_str_radix(hex_pair, 16).map_err(|_| {
            PyError::value_error(format!("non-hexadecimal number found in fromhex() arg at position {}", s.find(hex_pair).unwrap_or(0)))
        })?;
        result.push(byte);
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
}

pub fn builtin_bytearray(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(PyObjectRef::new(PyObject::ByteArray(Vec::new()))) }
    else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(i) => {
                let n = i.to_i64().ok_or_else(|| PyError::value_error("bytearray() requires int in range 0-255"))?;
                if n < 0 || n > 255 {
                    return Err(PyError::value_error("bytearray() requires int in range 0-255"));
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(vec![n as u8])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::new(PyObject::ByteArray(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytearray() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytearray() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytearray() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytearray() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytearray() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytearray() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            _ => Err(PyError::type_error(format!("cannot convert '{}' to bytearray", obj.type_name()))),
        }
    }
}

pub fn builtin_frozenset(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::imm(PyObject::FrozenSet(PySet::new())))
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Set(s) => Ok(PyObjectRef::imm(PyObject::FrozenSet(s.clone()))),
            PyObject::FrozenSet(s) => Ok(PyObjectRef::imm(PyObject::FrozenSet(s.clone()))),
            PyObject::List(v) => {
                let mut set = PySet::new();
                for item in v { set.add(item.clone())?; }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Tuple(v) => {
                let mut set = PySet::new();
                for item in v { set.add(item.clone())?; }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Str(s) => {
                let mut set = PySet::new();
                for ch in s.chars() {
                    set.add(py_str(&ch.to_string()))?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Bytes(b) => {
                let mut set = PySet::new();
                for &byte in b {
                    set.add(py_int(byte as i64))?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            _ => Err(PyError::type_error(format!("cannot convert '{}' to frozenset", obj.type_name()))),
        }
    }
}

pub fn builtin_format(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        0 => Err(PyError::type_error("format() requires at least 1 argument")),
        1 => Ok(py_str(&args[0].str())),
        2 => {
            let val = args[0].str();
            let spec = args[1].str();
            if spec.trim().is_empty() {
                return Ok(py_str(&val));
            }
            // Basic format: fill, align, width
            let spec = spec.trim();
            let s: Vec<char> = spec.chars().collect();
            let mut idx = 0;
            let fill_char = if idx < s.len() && !matches!(s[idx], '<' | '>' | '^' | '=') {
                let c = s[idx]; idx += 1; c
            } else { ' ' };
            let align = if idx < s.len() && matches!(s[idx], '<' | '>' | '^' | '=') {
                let c = s[idx]; idx += 1; Some(c)
            } else { None };
            let width_str: String = s[idx..].iter().take_while(|c| c.is_ascii_digit()).collect();
            let width: usize = width_str.parse().unwrap_or(0);
            if width > 0 {
                let padding = width.saturating_sub(val.len());
                match align {
                    Some('<') | None => Ok(py_str(&format!("{}{}", val, fill_char.to_string().repeat(padding)))),
                    Some('>') => Ok(py_str(&format!("{}{}", fill_char.to_string().repeat(padding), val))),
                    Some('^') => {
                        let left = padding / 2;
                        let right = padding - left;
                        Ok(py_str(&format!("{}{}{}", fill_char.to_string().repeat(left), val, fill_char.to_string().repeat(right))))
                    }
                    Some('=') => Ok(py_str(&val)),
                    _ => Ok(py_str(&val)),
                }
            } else {
                Ok(py_str(&val))
            }
        }
        _ => Err(PyError::type_error("format() takes at most 2 arguments")),
    }
}

pub fn py_format(val: &PyObjectRef, spec: &str) -> PyResult<PyObjectRef> {
    let args = [val.clone(), py_str(spec)];
    builtin_format(&args)
}

pub fn builtin_object(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Create a new bare object instance
    let object_type = PyObjectRef::new(PyObject::Type {
        name: "object".to_string(),
        dict: HashMap::new(),
        bases: vec![],
        mro: vec![],
    });
    Ok(PyObjectRef::new(PyObject::Instance {
        typ: object_type,
        dict: HashMap::new(),
    }))
}

pub fn builtin_hash(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hash() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(i) => {
            // Hash of int is the int itself (CPython behavior for small ints)
            let i64_val = i.to_i64().unwrap_or(0);
            Ok(py_int(i64_val))
        }
        PyObject::Float(f) => Ok(py_int(f.to_bits() as i64)),
        PyObject::Bool(b) => Ok(py_int(if *b { 1 } else { 0 })),
        PyObject::Str(s) => {
            // Simple FNV-1a hash
            let mut hash: u64 = 14695981039346656037;
            for byte in s.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(1099511628211);
            }
            Ok(py_int(hash as i64))
        }
        PyObject::Bytes(b) => {
            let mut hash: u64 = 14695981039346656037;
            for byte in b {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(1099511628211);
            }
            Ok(py_int(hash as i64))
        }
        PyObject::ByteArray(b) => {
            let mut hash: u64 = 14695981039346656037;
            for byte in b {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(1099511628211);
            }
            Ok(py_int(hash as i64))
        }
        PyObject::Tuple(v) => {
            let mut hash: u64 = 14695981039346656037;
            for item in v {
                let item_hash = builtin_hash(&[item.clone()])?;
                let h = item_hash.borrow().clone();
                if let PyObject::Int(i) = &h {
                    let i64_val = i.to_i64().unwrap_or(0);
                    hash ^= i64_val as u64;
                    hash = hash.wrapping_mul(1099511628211);
                }
            }
            Ok(py_int(hash as i64))
        }
        PyObject::None => Ok(py_int(123456789)),
        _ => {
            // For objects without a hash, use the pointer as hash
            let ptr = &*obj as *const PyObject as usize;
            Ok(py_int(ptr as i64))
        }
    }
}

pub fn builtin_slice(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        1 => {
            let stop = args[0].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start: none.clone(),
                stop,
                step: none,
            }))
        }
        2 => {
            let start = args[0].clone();
            let stop = args[1].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start,
                stop,
                step: none,
            }))
        }
        3 => {
            Ok(PyObjectRef::imm(PyObject::Slice {
                start: args[0].clone(),
                stop: args[1].clone(),
                step: args[2].clone(),
            }))
        }
        _ => Err(PyError::type_error("slice() takes at most 3 arguments")),
    }
}

pub fn builtin_dir(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_list(Vec::new()));
    }
    let obj = args[0].borrow();
    let mut names = Vec::new();
    match &*obj {
        PyObject::Instance { dict, .. } | PyObject::Module { dict, .. } => {
            for key in dict.keys() {
                names.push(py_str(key));
            }
        }
        PyObject::Type { dict, .. } => {
            for key in dict.keys() {
                names.push(py_str(key));
            }
        }
        _ => {}
    }
    // Add basic attributes for all types
    names.push(py_str("__class__"));
    names.push(py_str("__dir__"));
    names.sort_by(|a, b| {
        let a = a.borrow();
        let b = b.borrow();
        if let (PyObject::Str(a), PyObject::Str(b)) = (&*a, &*b) {
            a.cmp(b)
        } else { std::cmp::Ordering::Equal }
    });
    Ok(py_list(names))
}

pub fn builtin_globals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm.frames.last().ok_or_else(|| PyError::runtime_error("no frame"))?;
        let globals = frame.globals.borrow();
        let mut d = crate::object::PyDict::new();
        for (k, v) in globals.iter() {
            d.set(py_str(k), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(d)))
    })?
}

pub fn builtin_locals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm.frames.last().ok_or_else(|| PyError::runtime_error("no frame"))?;
        let mut d = crate::object::PyDict::new();
        for (k, v) in frame.locals.iter() {
            let name = crate::interner::lookup(k);
            d.set(py_str(&name), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(d)))
    })?
}

pub fn builtin_divmod(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 { return Err(PyError::type_error("divmod() takes exactly 2 arguments")); }
    let a = args[0].as_i64().ok_or_else(|| PyError::type_error("divmod() arg must be int"))?;
    let b = args[1].as_i64().ok_or_else(|| PyError::type_error("divmod() arg must be int"))?;
    if b == 0 { return Err(PyError::value_error("division by zero")); }
    Ok(PyObjectRef::new(PyObject::Tuple(vec![py_int(a / b), py_int(a % b)])))
}

pub fn builtin_round(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 || args.len() > 2 { return Err(PyError::type_error("round() takes 1 or 2 arguments")); }
    let val = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(i) => i.to_f64().unwrap_or(0.0),
            PyObject::Float(f) => *f,
            _ => return Err(PyError::type_error("round() arg must be numeric")),
        }
    };
    if args.len() == 2 {
        let n = args[1].as_i64().ok_or_else(|| PyError::type_error("ndigits must be int"))? as i32;
        Ok(py_float((val * 10_f64.powi(n)).round() / 10_f64.powi(n)))
    } else {
        Ok(py_int(val.round() as i64))
    }
}

pub fn builtin_abs(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("abs() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(i) => Ok(py_int(i.clone().abs())),
        PyObject::Float(f) => Ok(py_float(f.abs())),
        _ => Err(PyError::type_error(format!("bad operand type for abs(): '{}'", obj.type_name()))),
    }
}

pub fn builtin_hasattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("hasattr() takes exactly 2 arguments"));
    }
    let attr_name = args[1].str();
    match args[0].borrow().get_attribute(&attr_name) {
        Ok(_) => Ok(py_bool(true)),
        Err(_) => Ok(py_bool(false)),
    }
}

pub fn builtin_getattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("getattr() takes at least 2 arguments"));
    }
    let attr_name = args[1].str();
    match args[0].borrow().get_attribute(&attr_name) {
        Ok(val) => Ok(val),
        Err(_) if args.len() >= 3 => Ok(args[2].clone()),
        Err(e) => Err(e),
    }
}

pub fn builtin_setattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 3 {
        return Err(PyError::type_error("setattr() takes exactly 3 arguments"));
    }
    let attr_name = args[1].str();
    args[0].borrow_mut().set_attribute(&attr_name, args[2].clone())?;
    Ok(py_none())
}

pub fn builtin_delattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("delattr() takes exactly 2 arguments"));
    }
    let attr_name = args[1].str();
    // Check for __delattr__ on Instance types first
    let f = {
        let obj_borrowed = args[0].borrow();
        match &*obj_borrowed {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__delattr__").cloned(),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![args[1].clone()]);
    }
    args[0].borrow_mut().del_attribute(&attr_name)?;
    Ok(py_none())
}

pub fn builtin_ord(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("ord() takes exactly one argument"));
    }
    let s = args[0].str();
    let c = s.chars().next().ok_or_else(|| PyError::type_error("ord() expected a character, but string of length 0 found"))?;
    Ok(py_int(c as u32 as i64))
}

pub fn builtin_chr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("chr() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    let code = n.to_usize().ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
    let c = char::from_u32(code as u32).ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
    Ok(py_str(&c.to_string()))
}

pub fn builtin_hex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hex() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    Ok(py_str(&format!("0x{:x}", n)))
}

pub fn builtin_oct(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("oct() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    Ok(py_str(&format!("0o{:o}", n)))
}

pub fn builtin_bin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("bin() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    Ok(py_str(&format!("0b{:b}", n)))
}

pub fn builtin_ascii(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("ascii() takes exactly one argument"));
    }
    let s = args[0].repr();
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            result.push(c);
        } else {
            let code = c as u32;
            if code <= 0xFF {
                result.push_str(&format!("\\x{:02x}", code));
            } else if code <= 0xFFFF {
                result.push_str(&format!("\\u{:04x}", code));
            } else {
                result.push_str(&format!("\\U{:08x}", code));
            }
        }
    }
    Ok(py_str(&result))
}

pub fn builtin_memoryview(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("memoryview() takes exactly one argument"));
    }
    // Return a string representation since we don't have a full memoryview type
    Ok(py_str(&format!("<memory at 0x{:x}>", std::ptr::from_ref(&args[0]) as usize)))
}

pub fn builtin_input(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if !args.is_empty() {
        print!("{}", args[0].str());
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| PyError::runtime_error(e.to_string()))?;
    if line.ends_with('\n') {
        line.pop();
    }
    Ok(py_str(&line))
}

pub fn builtin_exit(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let code = if args.is_empty() { 0 }
    else if let PyObject::Int(i) = &*args[0].borrow() {
        i.to_i32().unwrap_or(0)
    } else { 0 };
    Err(PyError::SystemExit(code))
}

pub fn call_bound_method(func: PyObjectRef, self_obj: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
    match &*func.borrow() {
        PyObject::BuiltinMethod { func: f, self_obj: s, .. } => {
            let mut all_args = vec![s.clone()];
            all_args.push(self_obj);
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::BuiltinFunction { func: f, .. } => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::Closure(func) => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            func(&all_args)
        }
        PyObject::Function { code, globals: g, defaults, .. } => {
            let mut frame = super::vm::Frame::new(std::rc::Rc::new(code.clone()), g.clone(), std::rc::Rc::new(create_builtins()), None);
            let code = code.clone();
            let defaults = defaults.clone();
            // Set self at index 0
            if !code.varnames.is_empty() {
                frame.fast_locals[0] = Some(self_obj.clone());
                frame.insert_local(&code.varnames[0].clone(), self_obj);
            }
            let npos = args.len();
            let named_params = if code.vararg_name.is_some() || code.kwarg_name.is_some() {
                code.varnames.iter().position(|n| {
                    Some(n.clone()) == code.vararg_name || Some(n.clone()) == code.kwarg_name
                }).unwrap_or(code.varnames.len())
            } else {
                code.varnames.len()
            };
            for i in 0..npos.min(named_params.saturating_sub(1)) {
                let idx = i + 1;
                if idx < code.varnames.len() {
                    frame.fast_locals[idx] = Some(args[i].clone());
                    frame.insert_local(&code.varnames[idx].clone(), args[i].clone());
                }
            }
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in (named_params.saturating_sub(1))..npos {
                    extra.push(args[i].clone());
                }
                frame.insert_local(&vararg_name, py_tuple(extra));
            }
            if npos < named_params.saturating_sub(1) {
                let num_defaults = code.num_defaults;
                for i in npos..named_params.saturating_sub(1) {
                    let idx = i + 1;
                    if idx < code.varnames.len() {
                        let default_idx = num_defaults.saturating_sub(named_params.saturating_sub(1) - i);
                        if default_idx < defaults.len() {
                            let val = defaults[default_idx].clone();
                            frame.fast_locals[idx] = Some(val.clone());
                            frame.insert_local(&code.varnames[idx].clone(), val);
                        }
                    }
                }
            }
            let mut vm = super::vm::VirtualMachine::new();
            vm.frames.push(frame);
            vm.execute()
        }
        PyObject::BoundMethod { func, .. } => {
            let mut all_args = vec![self_obj.clone()];
            all_args.extend(args);
            call_bound_method(func.clone(), self_obj, all_args)
        }
        _ => Err(PyError::type_error("object is not callable")),
    }
}

pub fn builtin_sorted(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("sorted() takes at least 1 argument"));
    }
    // Check for key keyword argument (last arg could be a dict with "key")
    let key_fn: Option<PyObjectRef> = if args.len() >= 2 {
        // Check if last arg is a dict (keyword args container)
        let last = args.last().unwrap();
        let last_borrowed = last.borrow();
        if let PyObject::Dict(kwargs) = &*last_borrowed {
            kwargs.get(&py_str("key")).unwrap_or(None)
        } else {
            None
        }
    } else {
        None
    };
    let mut v = Vec::new();
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => v.push(val),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    // Use Rust's stable sort (O(n log n))
    let len = v.len();
    if len > 1 {
        // Sort with comparison, optionally applying key function
        let key_fn_ref = key_fn.clone();
        v.sort_by(|a, b| {
            let a_val = if let Some(ref kf) = key_fn_ref {
                call_bound_method(kf.clone(), a.clone(), vec![]).unwrap_or_else(|_| a.clone())
            } else {
                a.clone()
            };
            let b_val = if let Some(ref kf) = key_fn_ref {
                call_bound_method(kf.clone(), b.clone(), vec![]).unwrap_or_else(|_| b.clone())
            } else {
                b.clone()
            };
            let ordering = match a_val.borrow().gt(&b_val) {
                Ok(true) => std::cmp::Ordering::Greater,
                Ok(false) => match a_val.borrow().lt(&b_val) {
                    Ok(true) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                },
                Err(_) => std::cmp::Ordering::Equal,
            };
            ordering
        });
    }
    Ok(py_list(v))
}

pub fn builtin_enumerate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("enumerate() takes at least 1 argument"));
    }
    let start: usize = if args.len() > 1 {
        if let PyObject::Int(i) = &*args[1].borrow() {
            i.to_usize().unwrap_or(0)
        } else { 0 }
    } else { 0 };
    // Eagerly consume the iterable into a Vec, then wrap in lazy EnumerateIter
    let iterable = builtin_iter(&[args[0].clone()])?;
    let mut items = Vec::new();
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => items.push(val),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(PyObjectRef::new(PyObject::EnumerateIter { items, pos: 0, start }))
}

pub fn builtin_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("iter() takes exactly one argument"));
    }
    // Check for __iter__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__iter__").cloned(),
                    _ => None,
                }
            }
            PyObject::Generator { .. } => {
                // Generators are their own iterator (return self)
                return Ok(args[0].clone());
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![]);
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Tuple(v) => Ok(py_list(v.clone())),
        PyObject::Str(s) => Ok(py_list(s.chars().map(|c| py_str(&c.to_string())).collect())),
        PyObject::Set(s) => Ok(py_list(s.to_vec())),
        PyObject::FrozenSet(s) => Ok(py_list(s.to_vec())),
        PyObject::Range { start, stop, step } => {
            Ok(PyObjectRef::new(PyObject::RangeIter { current: *start, stop: *stop, step: *step }))
        }
        PyObject::List(v) => {
            Ok(PyObjectRef::new(PyObject::ListIter { list: v.clone(), index: 0 }))
        }
        _ => Ok(args[0].clone()),
    }
}

pub fn builtin_next(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("next() takes at least 1 argument"));
    }
    // Check for __next__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__next__").cloned(),
                    _ => None,
                }
            }
            PyObject::Generator { .. } => {
                drop(obj);
                let next_func = args[0].borrow().get_attribute("__next__")?;
                let (n, f) = {
                    let b = next_func.borrow();
                    if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                        (name.clone(), *func)
                    } else { return Err(PyError::runtime_error("expected __next__ method")) }
                };
                let result = f(&[args[0].clone()]);
                // Convert raise StopIteration into PyError::StopIteration for next() protocol
                if let Err(PyError::Exception(ref typ, _)) = result {
                    if typ == "StopIteration" {
                        return Err(PyError::StopIteration);
                    }
                }
                return result;
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        let result = call_bound_method(f, args[0].clone(), vec![]);
        // Convert raise StopIteration into PyError::StopIteration for next() protocol
        if let Err(PyError::Exception(_, ref exc)) = result {
            let is_stop = match &*exc.borrow() {
                PyObject::Exception { typ, .. } if typ == "StopIteration" => true,
                _ => false,
            };
            if is_stop {
                return Err(PyError::StopIteration);
            }
        }
        return result;
    }
    // Fallback to list-based iteration
    let mut obj = args[0].borrow_mut();
    match &mut *obj {
        PyObject::List(v) => {
            if v.is_empty() {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                // Convert to ListIter for O(1) iteration
                let list = std::mem::take(v);
                *obj = PyObject::ListIter { list, index: 0 };
                drop(obj);
                let mut obj = args[0].borrow_mut();
                if let PyObject::ListIter { list, index } = &mut *obj {
                    let v = list[*index].clone();
                    *index += 1;
                    Ok(v)
                } else { unreachable!() }
            }
        }
        PyObject::ListIter { list, index } => {
            if *index >= list.len() {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                let v = list[*index].clone();
                *index += 1;
                Ok(v)
            }
        }
        PyObject::EnumerateIter { items, pos, start } => {
            if *pos >= items.len() {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                let val = items[*pos].clone();
                let idx = *start + *pos;
                *pos += 1;
                Ok(py_tuple(vec![py_int(idx as i64), val]))
            }
        }
        PyObject::RangeIter { current, stop, step } => {
            if (*step > 0 && *current >= *stop) || (*step < 0 && *current <= *stop) {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                let v = py_int(*current);
                *current += *step;
                Ok(v)
            }
        }
        _ => Err(PyError::type_error(format!("'{}' is not an iterator", obj.type_name()))),
    }
}

pub fn builtin_sum(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("sum() takes at least 1 argument"));
    }
    let start = if args.len() >= 2 { args[1].clone() } else { py_int(0) };
    let mut total = start;
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => { total = py_add(&total, &val)?; }
            Err(PyError::StopIteration) => return Ok(total),
            Err(e) => return Err(e),
        }
    }
}

fn compare_gt(a: &PyObjectRef, b: &PyObjectRef) -> std::cmp::Ordering {
    match a.borrow().gt(b) {
        Ok(true) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Less,
    }
}

pub fn builtin_max(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("max() requires at least 1 argument")); }
    let items: Vec<PyObjectRef> = if args.len() == 1 {
        let mut v = Vec::new();
        let iterable = builtin_iter(&[args[0].clone()])?;
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(val) => v.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        v
    } else {
        args.to_vec()
    };
    items.into_iter().max_by(compare_gt).ok_or_else(|| PyError::value_error("max() arg is an empty sequence"))
}

pub fn builtin_min(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("min() requires at least 1 argument")); }
    let items: Vec<PyObjectRef> = if args.len() == 1 {
        let mut v = Vec::new();
        let iterable = builtin_iter(&[args[0].clone()])?;
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(val) => v.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        v
    } else {
        args.to_vec()
    };
    items.into_iter().min_by(compare_gt).ok_or_else(|| PyError::value_error("min() arg is an empty sequence"))
}

pub fn builtin_id(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("id() takes exactly one argument"));
    }
    Ok(py_int(args[0].get_id() as i64))
}

pub fn builtin_vars(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("vars() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Instance { dict, .. } => {
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(k), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(pd)))
        }
        PyObject::Module { dict, .. } => {
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(k), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(pd)))
        }
        _ => Err(PyError::type_error(format!("vars() argument must have __dict__ attribute"))),
    }
}

pub fn builtin_isinstance(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("isinstance() takes exactly 2 arguments"));
    }
    let obj = args[0].borrow();
    let class = args[1].borrow();
    // Handle tuple of types: isinstance(x, (type1, type2, ...))
    if let PyObject::Tuple(types) = &*class {
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    match (&*obj, &*class) {
        (PyObject::Instance { typ, .. }, PyObject::Type { mro, .. }) => {
            let typ_name = typ.borrow().type_name();
            for c in mro {
                if c.borrow().type_name() == typ_name {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_bool(false))
        }
        (PyObject::Instance { typ, .. }, _) => {
            let class_name = class.str();
            Ok(py_bool(typ.borrow().type_name() == class_name || class_name == "object"))
        }
        _ => {
            let obj_type = args[0].borrow().type_name();
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => class.str(),
            };
            Ok(py_bool(obj_type == class_name))
        }
    }
}

pub fn builtin_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("open() missing required argument 'file'"));
    }
    let filename = args[0].str();
    let mode = if args.len() > 1 { args[1].str() } else { "r".to_string() };
    let file = std::fs::File::options()
        .read(mode.contains('r'))
        .write(mode.contains('w') || mode.contains('a'))
        .append(mode.contains('a'))
        .create(mode.contains('w') || mode.contains('a'))
        .truncate(mode.contains('w'))
        .open(&filename)
        .map_err(|e| PyError::OsError(format!("{}", e)))?;
    Ok(PyObjectRef::new(PyObject::File { file: std::rc::Rc::new(std::cell::RefCell::new(file)), name: filename }))
}

pub fn builtin_any(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("any() requires at least 1 argument"));
    }
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => if val.truthy() { return Ok(py_bool(true)); },
            Err(PyError::StopIteration) => return Ok(py_bool(false)),
            Err(e) => return Err(e),
        }
    }
}

pub fn builtin_all(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("all() requires at least 1 argument"));
    }
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => if !val.truthy() { return Ok(py_bool(false)); },
            Err(PyError::StopIteration) => return Ok(py_bool(true)),
            Err(e) => return Err(e),
        }
    }
}

pub fn builtin_callable(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("callable() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    let is_callable = matches!(&*obj,
        PyObject::Function { .. } | PyObject::BuiltinFunction { .. } |
        PyObject::BuiltinMethod { .. } | PyObject::Type { .. } | PyObject::BuildClass |
        PyObject::BoundMethod { .. } | PyObject::Partial { .. } |
        PyObject::Generator { .. } | PyObject::Coroutine { .. } |
        // Instances may be callable if they have __call__
        PyObject::Instance { .. }
    );
    // For instances, check if the type has __call__
    if !is_callable {
        Ok(py_bool(false))
    } else if let PyObject::Instance { typ, .. } = &*obj {
        let typ_ref = typ.borrow();
        let has_call = if let PyObject::Type { dict, .. } = &*typ_ref {
            dict.contains_key("__call__")
        } else {
            false
        };
        Ok(py_bool(has_call))
    } else {
        Ok(py_bool(true))
    }
}

pub fn builtin_breakpoint(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if !args.is_empty() {
        eprintln!(
            "Breakpoint reached with {} argument(s) — debugger not available in this interpreter",
            args.len()
        );
        for (i, arg) in args.iter().enumerate() {
            eprintln!("  arg[{}]: {}", i, arg.str());
        }
    } else {
        eprintln!("Breakpoint reached — debugger not available in this interpreter");
    }
    Ok(py_none())
}

pub fn builtin_pow(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("pow() requires at least 2 arguments"));
    }
    let result = py_pow(&args[0], &args[1])?;
    if args.len() == 3 {
        py_mod(&result, &args[2])
    } else {
        Ok(result)
    }
}

pub fn builtin_reversed(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("reversed() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::List(v) => {
            let mut rev = v.clone(); rev.reverse();
            Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }))
        }
        PyObject::Tuple(v) => {
            let mut rev = v.clone(); rev.reverse();
            Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }))
        }
        PyObject::Str(s) => {
            let chars: Vec<PyObjectRef> = s.chars().rev().map(|c| py_str(&c.to_string())).collect();
            Ok(PyObjectRef::new(PyObject::ListIter { list: chars, index: 0 }))
        }
        _ => {
            let mut v = Vec::new();
            let iterable = builtin_iter(&[args[0].clone()])?;
            loop {
                match builtin_next(&[iterable.clone()]) {
                    Ok(val) => v.push(val),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
            v.reverse();
            Ok(PyObjectRef::new(PyObject::ListIter { list: v, index: 0 }))
        }
    }
}

pub fn builtin_issubclass(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("issubclass() takes exactly 2 arguments"));
    }
    // Handle tuple of types: issubclass(cls, (type1, type2, ...))
    let base = args[1].borrow();
    if let PyObject::Tuple(types) = &*base {
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let cls = args[0].borrow();
    drop(base);
    let base = args[1].borrow();
    match (&*cls, &*base) {
        (PyObject::Type { mro: cls_mro, .. }, PyObject::Type { .. }) => {
            let base_tn = base.type_name();
            for c in cls_mro {
                if c.borrow().type_name() == base_tn {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_bool(false))
        }
        (PyObject::Type { mro: cls_mro, .. }, _) => {
            // Non-Type second argument: compare by name
            let base_name = base.str();
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            for c in cls_mro {
                if c.borrow().type_name() == base_name {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_bool(false))
        }
        _ => Err(PyError::type_error("issubclass() arg 1 must be a class")),
    }
}

pub fn builtin_help(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        println!("Welcome to RustPython 0.1.0!");
        println!();
        println!("Available built-in functions:");
        println!("  abs()  all()  any()  ascii()  bin()  bool()  breakpoint()");
        println!("  bytearray()  bytes()  callable()  chr()  compile()  delattr()");
        println!("  dict()  dir()  divmod()  enumerate()  eval()  exec()  exit()");
        println!("  filter()  float()  format()  frozenset()  getattr()  globals()");
        println!("  hasattr()  hash()  help()  hex()  id()  input()  int()");
        println!("  isinstance()  issubclass()  iter()  len()  list()  locals()");
        println!("  map()  max()  memoryview()  min()  next()  object()  oct()");
        println!("  open()  ord()  pow()  print()  property()  range()  repr()");
        println!("  reversed()  round()  set()  setattr()  slice()  sorted()");
        println!("  staticmethod()  str()  sum()  super()  tuple()  type()  vars()");
        println!("  zip()");
        println!();
        println!("Available error types:");
        println!("  BaseException  Exception  TypeError  ValueError  ZeroDivisionError");
        println!("  NameError  AttributeError  IndexError  KeyError  RuntimeError");
        println!("  StopIteration  AssertionError  OSError  ImportError  LookupError");
        println!("  ArithmeticError  OverflowError  NotImplementedError  RecursionError");
        println!("  KeyboardInterrupt  SystemExit  ModuleNotFoundError  FileNotFoundError");
        println!("  PermissionError  UnicodeDecodeError  UnicodeEncodeError");
        println!();
        println!("Type help(object) for information about a specific object.");
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Type { name, dict, .. } => {
                println!("Help on class {}:", name);
                if let Some(doc) = dict.get("__doc__") {
                    println!("  {}", doc.str());
                }
                println!();
                println!("Methods:");
                for (key, val) in dict {
                    if matches!(&*val.borrow(), PyObject::Function { .. } | PyObject::BuiltinFunction { .. }) {
                        println!("  {}()", key);
                    }
                }
            }
            PyObject::Function { name, dict, .. } => {
                println!("Help on function {}:", name);
                if let Some(doc) = dict.get("__doc__") {
                    println!("  {}", doc.str());
                }
            }
            PyObject::BuiltinFunction { name, .. } => {
                println!("Help on built-in function {}:", name);
            }
            _ => {
                println!("Help on {}:", obj.type_name());
                println!("  Type: {}", obj.type_name());
            }
        }
    }
    Ok(py_none())
}

// ---- __import__ builtin ----

pub fn builtin_import(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__import__() requires at least 1 argument (module name)"));
    }
    let name = args[0].str();
    // Handle fromlist: if provided, return the rightmost submodule
    let fromlist = if args.len() > 3 {
        match &*args[3].borrow() {
            PyObject::List(items) => Some(items.clone()),
            PyObject::Tuple(items) => Some(items.iter().cloned().collect()),
            _ => None,
        }
    } else {
        None
    };
    let has_dots = name.contains('.');
    let has_fromlist = fromlist.as_ref().map_or(false, |fl| !fl.is_empty());

    // Use a raw pointer to avoid closure lifetime issues with long function
    let vm_ptr = match with_vm_mut(|vm| vm as *mut super::vm::VirtualMachine) {
        Ok(ptr) => ptr,
        Err(_) => return Err(PyError::runtime_error("__import__: no active VM")),
    };
    // SAFETY: VM_PTR is valid during builtin execution
    let vm = unsafe { &mut *vm_ptr };

    // With a non-empty fromlist and a dotted name, import the full module chain
    // and return the rightmost module. CPython behavior:
    //   __import__("certifi.core", ..., ["where"], 0)  -> imports certifi.core, returns certifi.core
    //   __import__("certifi.core", ..., [], 0)          -> imports certifi, returns certifi
    if has_dots && has_fromlist {
        // First, ensure the top-level package is imported (import_module_from_file
        // needs the parent in modules to resolve dotted names)
        let top_name = name.split('.').next().unwrap_or(&name).to_string();
        if !vm.modules.contains_key(&top_name) {
            match vm.import_module_from_file(&top_name) {
                Ok(module) => {
                    vm.modules.insert(top_name.clone(), module.clone());
                    if let Some(sys_mod) = vm.modules.get("sys") {
                        if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                            if let Some(mod_dict) = dict.get("modules") {
                                mod_dict.borrow_mut().set_attribute(&top_name, module.clone()).ok();
                            }
                        }
                    }
                }
                Err(e) => return Err(PyError::ImportError(format!("__import__ error: {}", e))),
            }
        }

        // Now import the full chain - import_module_from_file handles dotted
        // names when the parent is already in modules
        if let Some(module) = vm.modules.get(&name) {
            return Ok(module.clone());
        }
        return match vm.import_module_from_file(&name) {
            Ok(module) => {
                vm.modules.insert(name.to_string(), module.clone());
                if let Some(sys_mod) = vm.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get("modules") {
                            mod_dict.borrow_mut().set_attribute(&name, module.clone()).ok();
                        }
                    }
                }
                Ok(module)
            }
            Err(e) => Err(PyError::ImportError(format!("__import__ error: {}", e))),
        };
    }

    // Without fromlist (or non-dotted name), import only the top-level package
    let resolved_name = if has_dots {
        name.split('.').next().unwrap_or(&name).to_string()
    } else {
        name.clone()
    };

    // Check if already loaded
    if let Some(module) = vm.modules.get(&resolved_name) {
        return Ok(module.clone());
    }

    // Try to import the module from file
    match vm.import_module_from_file(&resolved_name) {
        Ok(module) => {
            vm.modules.insert(resolved_name.clone(), module.clone());
            // Also add to sys.modules
            if let Some(sys_mod) = vm.modules.get("sys") {
                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                    if let Some(mod_dict) = dict.get("modules") {
                        mod_dict.borrow_mut().set_attribute(&resolved_name, module.clone()).ok();
                    }
                }
            }
            Ok(module)
        }
        Err(e) => Err(PyError::ImportError(format!("__import__ error: {}", e))),
    }
}

pub fn builtin_eval(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("eval() requires at least 1 argument"));
    }
    let source = args[0].str();
    let mut parser = crate::parser::Parser::new(&source);
    let program = parser.parse_program().map_err(|e| PyError::type_error(format!("eval parse error: {}", e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler.compile(&program, "<eval>").map_err(|e| PyError::type_error(format!("eval compile error: {}", e)))?;
    let code2 = code.clone();
    // Use current VM if available via VM_PTR so exec() shares modules, sys.path, etc.
    match with_vm_mut(|vm| vm.run(code)) {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(e)) => Err(PyError::type_error(format!("eval error: {}", e))),
        Err(_) => {
            let mut new_vm = crate::vm::VirtualMachine::new();
            new_vm.run(code2).map_err(|e| PyError::type_error(format!("eval error: {}", e)))
        }
    }
}

pub fn builtin_exec(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("exec() requires at least 1 argument"));
    }
    // Check if first arg is a code object (compile() result)
    let code = match &*args[0].borrow() {
        PyObject::Code(c) => (*c).clone(),
        _ => Box::new(
            (|| -> Result<CodeObject, String> {
                let source = args[0].str();
                let mut parser = crate::parser::Parser::new(&source);
                let program = parser.parse_program()?;
                let mut compiler = crate::compiler::Compiler::new();
                compiler.compile(&program, "<exec>")
            })().map_err(|e| PyError::type_error(format!("exec error: {}", e)))?
        ),
    };
    let code2 = (*code).clone();
    // Use current VM if available via VM_PTR so exec() shares modules, sys.path, etc.
    match with_vm_mut(|vm| vm.run(*code)) {
        Ok(Ok(ref _val)) => Ok(py_none()),
        Ok(Err(e)) => Err(PyError::type_error(format!("exec error: {}", e))),
        Err(_) => {
            let mut new_vm = crate::vm::VirtualMachine::new();
            new_vm.run(code2).map_err(|e| PyError::type_error(format!("exec error: {}", e)))?;
            Ok(py_none())
        }
    }
}

pub fn builtin_compile(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error("compile() requires 3 arguments (source, filename, mode)"));
    }
    let source = args[0].str();
    let filename = args[1].str();
    let mode = args[2].str();
    let mut parser = crate::parser::Parser::new(&source);
    let program = parser.parse_program().map_err(|e| PyError::type_error(format!("SyntaxError: {}", e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler.compile(&program, &filename).map_err(|e| PyError::type_error(format!("compile error: {}", e)))?;
    Ok(PyObjectRef::new(PyObject::Code(Box::new(code))))
}

pub fn builtin_super(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // super() with no args or super(class, instance)
    if args.len() == 2 {
        let cls = args[0].clone();
        let obj = args[1].clone();
        Ok(PyObjectRef::new(PyObject::Super { cls, obj }))
    } else {
        Err(PyError::type_error("super() requires 2 arguments"))
    }
}

pub fn builtin_map(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("map() requires at least 2 arguments"));
    }
    let func = args[0].clone();
    let iter = builtin_iter(&[args[1].clone()])?;
    let mut results = Vec::new();
    loop {
        match builtin_next(&[iter.clone()]) {
            Ok(val) => {
                let mapped = builtin_call(&func, &[val])?;
                results.push(mapped);
            }
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(py_list(results))
}

pub fn builtin_filter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("filter() requires exactly 2 arguments"));
    }
    let func = args[0].clone();
    let iter = builtin_iter(&[args[1].clone()])?;
    let mut results = Vec::new();
    loop {
        match builtin_next(&[iter.clone()]) {
            Ok(val) => {
                let keep = if matches!(&*func.borrow(), PyObject::None) {
                    val.truthy()
                } else {
                    builtin_call(&func, &[val.clone()])?.truthy()
                };
                if keep { results.push(val); }
            }
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(py_list(results))
}

pub fn builtin_zip(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("zip() requires at least 1 argument"));
    }
    let mut iters: Vec<PyObjectRef> = args.iter().map(|a| builtin_iter(&[a.clone()])).collect::<PyResult<Vec<_>>>()?;
    let mut results = Vec::new();
    loop {
        let mut group = Vec::new();
        for it in iters.iter_mut() {
            match builtin_next(&[it.clone()]) {
                Ok(val) => group.push(val),
                Err(PyError::StopIteration) => return Ok(py_list(results)),
                Err(e) => return Err(e),
            }
        }
        results.push(py_tuple(group));
    }
}

pub fn builtin_call(func: &PyObjectRef, args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let f = func.clone();
    let a = args.to_vec();
    let type_name = f.get_type_name();
    let kind = {
        let obj = f.borrow();
        match &*obj {
            PyObject::BuiltinFunction { .. } => 0,
            PyObject::BuiltinMethod { .. } => 1,
            PyObject::Function { .. } => 2,
            PyObject::BoundMethod { .. } => 3,
            PyObject::Type { .. } => 4,
            PyObject::BuildClass => 5,
            PyObject::Partial { .. } => 6,
            _ => 7,
        }
    };
    match kind {
        0 => {
            if let PyObject::BuiltinFunction { func: bf, .. } = &*f.borrow() { bf(&a) } else { unreachable!() }
        }
        1 => {
            if let PyObject::BuiltinMethod { func: bf, self_obj: s, .. } = &*f.borrow() {
                let mut all_args = vec![s.clone()];
                all_args.extend(a);
                bf(&all_args)
            } else { unreachable!() }
        }
        2 => {
            if let PyObject::Function { code, globals: g, defaults, .. } = &*f.borrow() {
                let code = code.clone();
                let g = g.clone();
                let defaults = defaults.clone();
                let npos = a.len();
                let named_params = if code.vararg_name.is_some() || code.kwarg_name.is_some() {
                    code.varnames.iter().position(|n| {
                        Some(n.clone()) == code.vararg_name || Some(n.clone()) == code.kwarg_name
                    }).unwrap_or(code.varnames.len())
                } else {
                    code.varnames.len()
                };
                let mut frame = super::vm::Frame::new(std::rc::Rc::new(code.clone()), g.clone(), std::rc::Rc::new(create_builtins()), None);
                for i in 0..npos.min(named_params) {
                    if i < code.varnames.len() {
                        frame.fast_locals[i] = Some(a[i].clone());
                        frame.insert_local(&code.varnames[i].clone(), a[i].clone());
                    }
                }
                if let Some(vararg_name) = &code.vararg_name {
                    let mut extra = Vec::new();
                    for i in named_params..npos {
                        extra.push(a[i].clone());
                    }
                    frame.insert_local(&vararg_name, py_tuple(extra));
                }
                if npos < named_params {
                    let num_defaults = code.num_defaults;
                    for i in npos..named_params {
                        let default_idx = num_defaults.saturating_sub(named_params - i);
                        if default_idx < defaults.len() {
                            frame.insert_local(&code.varnames[i].clone(), defaults[default_idx].clone());
                        }
                    }
                }
                if let Some(kwarg_name) = &code.kwarg_name {
                    if !frame.contains_local(kwarg_name) {
                        frame.insert_local(&kwarg_name, py_dict());
                    }
                }
                let mut vm = super::vm::VirtualMachine::new();
                vm.frames.push(frame);
                vm.execute()
            } else { unreachable!() }
        }
        3 => {
            let (bf, self_obj) = {
                let obj = f.borrow();
                if let PyObject::BoundMethod { func: bf, self_obj: s, .. } = &*obj {
                    (bf.clone(), s.clone())
                } else { return Err(PyError::type_error("not a bound method")); }
            };
            let mut all_args = vec![self_obj];
            let a_len = a.len();
            all_args.extend(a);
            builtin_call(&bf, &all_args)
        }
        4 => {
            if let PyObject::Type { dict: type_dict, .. } = &*f.borrow() {
                let instance = PyObjectRef::new(PyObject::Instance {
                    typ: f.clone(),
                    dict: std::collections::HashMap::new(),
                });
                if let Some(init) = type_dict.get_str("__init__").cloned() {
                    call_bound_method(init, instance.clone(), a)?;
                }
                Ok(instance)
            } else { unreachable!() }
        }
        5 => {
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: f.clone(),
                dict: std::collections::HashMap::new(),
            });
            Ok(instance)
        }
        6 => {
            let (func, partial_args) = {
                let obj = f.borrow();
                if let PyObject::Partial { func: bf, args: pa } = &*obj {
                    (bf.clone(), pa.clone())
                } else { return Err(PyError::type_error("not a partial")); }
            };
            let mut all_args = partial_args.clone();
            all_args.extend(a);
            builtin_call(&func, &all_args)
        }
        _ => Err(PyError::type_error(format!("'{}' object is not callable", type_name))),
    }
}

// ---- Descriptor types ----

pub fn builtin_property(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let getter = if args.len() > 0 { Some(args[0].clone()) } else { None };
    let setter = if args.len() > 1 { Some(args[1].clone()) } else { None };
    let deleter = if args.len() > 2 { Some(args[2].clone()) } else { None };
    let doc = if args.len() > 3 { Some(args[3].str()) } else { None };
    Ok(PyObjectRef::new(PyObject::Property {
        getter,
        setter,
        deleter,
        doc,
    }))
}

/// Return a new Property with the given setter (used by @x.setter)
pub fn property_setter(prop: &PyObjectRef, new_setter: PyObjectRef) -> PyObjectRef {
    let (getter, deleter, doc) = {
        let b = prop.borrow();
        match &*b {
            PyObject::Property { getter, deleter, doc, .. } => (getter.clone(), deleter.clone(), doc.clone()),
            _ => return prop.clone(),
        }
    };
    PyObjectRef::new(PyObject::Property {
        getter,
        setter: Some(new_setter),
        deleter,
        doc,
    })
}

/// Return a new Property with the given deleter (used by @x.deleter)
pub fn property_deleter(prop: &PyObjectRef, new_deleter: PyObjectRef) -> PyObjectRef {
    let (getter, setter, doc) = {
        let b = prop.borrow();
        match &*b {
            PyObject::Property { getter, setter, doc, .. } => (getter.clone(), setter.clone(), doc.clone()),
            _ => return prop.clone(),
        }
    };
    PyObjectRef::new(PyObject::Property {
        getter,
        setter,
        deleter: Some(new_deleter),
        doc,
    })
}

/// Builtin for property.setter(func) — returns new Property with setter
pub fn builtin_property_setter_fn(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("setter() requires at least the setter function"));
    }
    Ok(property_setter(&args[0], args[1].clone()))
}

/// Builtin for property.deleter(func) — returns new Property with deleter
pub fn builtin_property_deleter_fn(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("deleter() requires at least the deleter function"));
    }
    Ok(property_deleter(&args[0], args[1].clone()))
}

pub fn builtin_staticmethod(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("staticmethod() requires at least 1 argument")); }
    Ok(PyObjectRef::new(PyObject::StaticMethod { func: args[0].clone() }))
}

pub fn builtin_classmethod(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("classmethod() requires at least 1 argument")); }
    Ok(PyObjectRef::new(PyObject::ClassMethod { func: args[0].clone() }))
}

// ---- __slots__ helpers ----

/// Extract slot names from a __slots__ value (can be str, tuple, list, or set)
fn extract_slots(slots_val: &PyObjectRef, result: &mut Vec<String>) {
    let borrowed = slots_val.borrow();
    match &*borrowed {
        PyObject::Str(s) => {
            if !result.iter().any(|x| x.as_str() == s.as_str()) {
                result.push(s.to_string());
            }
        }
        PyObject::Tuple(items) => {
            for item in items {
                if let PyObject::Str(s) = &*item.borrow() {
                    if !result.iter().any(|x| x.as_str() == s.as_str()) {
                        result.push(s.to_string());
                    }
                }
            }
        }
        PyObject::List(items) => {
            for item in items {
                if let PyObject::Str(s) = &*item.borrow() {
                    if !result.iter().any(|x| x.as_str() == s.as_str()) {
                        result.push(s.to_string());
                    }
                }
            }
        }
        PyObject::Set(set) => {
            for item in set.to_vec() {
                if let PyObject::Str(s) = &*item.borrow() {
                    if !result.iter().any(|x| x.as_str() == s.as_str()) {
                        result.push(s.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

/// Get the effective __slots__ for a type, checking the entire MRO.
/// Returns None if no __slots__ is defined anywhere in the hierarchy.
fn get_instance_slots(typ: &PyObjectRef) -> Option<Vec<String>> {
    let typ_ref = typ.borrow();
    if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
        let mut all_slots = Vec::new();

        // Check the type's own __slots__
        if let Some(slots_val) = type_dict.get_str("__slots__") {
            extract_slots(slots_val, &mut all_slots);
        }

        // Check bases' __slots__ (skip self at index 0)
        for base in mro.iter().skip(1) {
            let base_ref = base.borrow();
            if let PyObject::Type { dict: base_dict, .. } = &*base_ref {
                if let Some(slots_val) = base_dict.get_str("__slots__") {
                    extract_slots(slots_val, &mut all_slots);
                }
            }
        }

        if !all_slots.is_empty() {
            return Some(all_slots);
        }
    }
    None
}

/// Get the class name for an Instance's type, used for error messages.
fn get_type_name_for_instance(typ: &PyObjectRef) -> String {
    let typ_ref = typ.borrow();
    if let PyObject::Type { name, .. } = &*typ_ref {
        name.clone()
    } else {
        "object".to_string()
    }
}

// ---- Attribute access ----

pub trait ObjectAccess {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef>;
    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()>;
    fn del_attribute(&mut self, name: &str) -> PyResult<()>;
}

impl ObjectAccess for PyObject {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef> {
        match self {
            PyObject::Module { dict, name: mod_name } => {
                if name == "__dict__" {
                    // Convert module's HashMap to a PyDict
                    use crate::object::ObjectAccess;
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        let _ = pd.set(py_str(k), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(pd)));
                }
                if name == "__name__" {
                    return Ok(py_str(mod_name));
                }
                dict.get_str(&name).cloned().ok_or_else(|| PyError::attribute_error(format!(
                    "'module' object has no attribute '{}'", name
                )))
            }
            PyObject::Type { dict, mro, bases, name: type_name } => {
                if name == "__dict__" {
                    // Return type's dict as a PyDict
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        let _ = pd.set(py_str(k), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(pd)));
                }
                if name == "__mro__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(mro.clone())));
                }
                if name == "__bases__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(bases.clone())));
                }
                if name == "__name__" {
                    return Ok(py_str(type_name));
                }
                if name == "__qualname__" {
                    return Ok(py_str(type_name));
                }
                // Check own dict first
                if let Some(val) = dict.get_str(&name).cloned() {
                    // Unwrap staticmethod descriptor so type access returns the function directly
                    let b = val.borrow();
                    if let PyObject::StaticMethod { func } = &*b {
                        return Ok(func.clone());
                    }
                    drop(b);
                    return Ok(val);
                }
                // Check MRO (skip self)
                for base in mro.iter().skip(1) {
                    if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                        if let Some(val) = base_dict.get_str(&name) {
                            // Unwrap staticmethod descriptor from MRO bases
                            let b = val.borrow();
                            if let PyObject::StaticMethod { func } = &*b {
                                return Ok(func.clone());
                            }
                            drop(b);
                            return Ok(val.clone());
                        }
                    }
                }
                // Fallback: for dict-derived types, provide common dict methods
                if name == "__iter__" || name == "items" || name == "keys" || name == "values"
                {
                    static DICT_METHODS: std::sync::OnceLock<std::collections::HashMap<String, BuiltinFunc>> = std::sync::OnceLock::new();
                    let methods = DICT_METHODS.get_or_init(|| {
                        let mut m = std::collections::HashMap::new();
                        m.insert("__iter__".to_string(), dict_method_iter as BuiltinFunc);
                        m.insert("items".to_string(), dict_method_items as BuiltinFunc);
                        m.insert("keys".to_string(), dict_method_keys as BuiltinFunc);
                        m.insert("values".to_string(), dict_method_values as BuiltinFunc);
                        m
                    });
                    if let Some(func) = methods.get(name) {
                        let func = *func;
                        return Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                            name: name.to_string(),
                            func,
                            self_obj: py_none(),
                        }));
                    }
                }
                Err(PyError::attribute_error(format!("type has no attribute '{}'", name)))
            }
            PyObject::Instance { dict, typ } => {
                if name == "__dict__" {
                    // Return a copy of the instance's HashMap as a PyDict (no live view from here)
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        let _ = pd.set(py_str(k), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(pd)));
                }
                if name == "__weakref__" {
                    // __weakref__ slot exists but returns None by default
                    // A full implementation would return a WeakRef object if one exists
                    return Ok(py_none());
                }
                // If __slots__ is defined, verify the attribute is allowed
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        // Check if it's a class-level attribute (method, etc.) — those are always allowed
                        let typ_ref = typ.borrow();
                        let is_in_type = if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                            type_dict.contains_key_str(&name) || mro.iter().skip(1).any(|base| {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    base_dict.contains_key_str(&name)
                                } else { false }
                            })
                        } else { false };
                        if !is_in_type {
                            let type_name = get_type_name_for_instance(typ);
                            return Err(PyError::attribute_error(
                                format!("'{}' object has no attribute '{}'", type_name, name)
                            ));
                        }
                    }
                }
                dict.get_str(&name).cloned().or_else(|| {
                    let typ_ref = typ.borrow();
                    if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                        type_dict.get_str(&name).cloned().or_else(|| {
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if let Some(val) = base_dict.get_str(&name) {
                                        return Some(val.clone());
                                    }
                                }
                            }
                            // Fallback: provide common dict methods for dict-like instances
                            if name == "__iter__" || name == "items" || name == "keys" || name == "values" {
                                let dict_snapshot: Vec<(String, PyObjectRef)> = dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                                let result = instance_builtin_dict_method(name, dict_snapshot);
                                return result;
                            }
                            None
                        })
                    } else {
                        None
                    }
                }).ok_or_else(|| PyError::attribute_error(format!("instance has no attribute '{}'", name)))
            }
            PyObject::Property { getter, setter, deleter, doc, .. } => {
                match name {
                    "fget" => getter.clone().ok_or_else(|| PyError::attribute_error("property has no getter".to_string())),
                    "fset" => setter.clone().ok_or_else(|| PyError::attribute_error("property has no setter".to_string())),
                    "fdel" => deleter.clone().ok_or_else(|| PyError::attribute_error("property has no deleter".to_string())),
                    "doc" => Ok(doc.clone().map_or_else(py_none, |d| py_str(&d))),
                    "__get__" => {
                        if let Some(_) = getter {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: "__get__".to_string(),
                                func: |args| {
                                    if args.len() < 4 { return Err(PyError::type_error("__get__() takes 2 positional arguments")); }
                                    // args: [self_obj, descriptor, instance, owner]
                                    let g = args[1].borrow();
                                    if let PyObject::Property { getter: Some(getter_fn), .. } = &*g {
                                        call_bound_method(getter_fn.clone(), args[2].clone(), vec![])
                                    } else { Err(PyError::runtime_error("property has no getter")) }
                                },
                                self_obj: PyObjectRef::new(PyObject::None),
                            }))
                        } else { Err(PyError::attribute_error("property has no getter".to_string())) }
                    }
                    "__set__" => {
                        if let Some(_) = setter {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: "__set__".to_string(),
                                func: |args| {
                                    if args.len() < 4 { return Err(PyError::type_error("__set__() takes 2 positional arguments")); }
                                    // args: [self_obj, descriptor, instance, value]
                                    let s = args[1].borrow();
                                    if let PyObject::Property { setter: Some(setter_fn), .. } = &*s {
                                        call_bound_method(setter_fn.clone(), args[2].clone(), vec![args[3].clone()])
                                    } else { Err(PyError::runtime_error("property has no setter")) }
                                },
                                self_obj: PyObjectRef::new(PyObject::None),
                            }))
                        } else { Err(PyError::attribute_error("property has no setter".to_string())) }
                    }
                    "setter" | "deleter" | "getter" => {
                        let is_setter = name == "setter";
                        let prop_obj = PyObjectRef::new(match self {
                            PyObject::Property { getter, setter, deleter, doc } => PyObject::Property {
                                getter: getter.clone(),
                                setter: setter.clone(),
                                deleter: deleter.clone(),
                                doc: doc.clone(),
                            },
                            _ => unreachable!(),
                        });
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: name.to_string(),
                            func: if is_setter { builtin_property_setter_fn } else { builtin_property_deleter_fn },
                            self_obj: prop_obj,
                        }))
                    }
                    _ => Err(PyError::attribute_error(format!("'property' object has no attribute '{}'", name))),
                }
            }
            PyObject::Exception { typ, args, cause } => {
                match name {
                    "__name__" => Ok(py_str(typ)),
                    "args" => Ok(py_tuple(args.clone())),
                    "__cause__" => {
                        match cause {
                            Some(cause_exc) => Ok(cause_exc.clone()),
                            None => Ok(py_none()),
                        }
                    }
                    _ => Err(PyError::attribute_error(format!("'Exception' object has no attribute '{}'", name))),
                }
            }
            PyObject::List(_v) => {
                match name {
                    "append" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "append".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("append() takes exactly one argument")); }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.push(args[1].clone()); Ok(py_none()) }
                            else { Err(PyError::runtime_error("append on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                if args.len() > 1 {
                                    let idx = args[1].as_i64().ok_or_else(|| PyError::type_error("pop index must be an integer"))?;
                                    let len = list.len() as i64;
                                    let idx = if idx < 0 { len + idx } else { idx };
                                    if idx < 0 || idx >= len {
                                        return Err(PyError::index_error("pop index out of range"));
                                    }
                                    Ok(list.remove(idx as usize))
                                } else {
                                    list.pop().ok_or_else(|| PyError::runtime_error("pop from empty list"))
                                }
                            } else { Err(PyError::runtime_error("pop on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "extend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "extend".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("extend() takes exactly one argument")); }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                let it = builtin_iter(&[args[1].clone()])?;
                                loop { match builtin_next(&[it.clone()]) { Ok(v) => list.push(v), Err(PyError::StopIteration) => return Ok(py_none()), Err(e) => return Err(e) } }
                            } else { Err(PyError::runtime_error("extend on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.clear(); Ok(py_none()) }
                            else { Err(PyError::runtime_error("clear on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "reverse" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "reverse".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.reverse(); Ok(py_none()) }
                            else { Err(PyError::runtime_error("reverse on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "remove" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "remove".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("remove() takes exactly one argument")); }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                let pos = list.iter().position(|item| item.equals(&args[1]).unwrap_or(false))
                                    .ok_or_else(|| PyError::value_error(format!("{} is not in list", args[1].str())))?;
                                list.remove(pos);
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("remove on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "index".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("index() takes at least 1 argument")); }
                            if let PyObject::List(list) = &*args[0].borrow() {
                                for (i, item) in list.iter().enumerate() {
                                    if item.equals(&args[1])? { return Ok(py_int(i as i64)); }
                                }
                                Err(PyError::value_error(format!("{} is not in list", args[1].str())))
                            } else { Err(PyError::runtime_error("index on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("count() takes at least 1 argument")); }
                            if let PyObject::List(list) = &*args[0].borrow() {
                                let c = list.iter().filter(|item| item.equals(&args[1]).unwrap_or(false)).count();
                                Ok(py_int(c as i64))
                            } else { Err(PyError::runtime_error("count on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "sort" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "sort".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                list.sort_by(|a, b| {
                                    let a_int = a.borrow();
                                    let b_int = b.borrow();
                                    match (&*a_int, &*b_int) {
                                        (PyObject::Int(ai), PyObject::Int(bi)) => ai.cmp(bi),
                                        (PyObject::Float(af), PyObject::Float(bf)) => af.partial_cmp(bf).unwrap_or(std::cmp::Ordering::Equal),
                                        _ => a.str().cmp(&b.str()),
                                    }
                                });
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("sort on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "insert" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "insert".to_string(),
                        func: |args| {
                            if args.len() < 3 { return Err(PyError::type_error("insert() takes exactly 2 arguments")); }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                let idx = args[1].as_i64().unwrap_or(0) as usize;
                                let idx = idx.min(list.len());
                                list.insert(idx, args[2].clone());
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("insert on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &*args[0].borrow() { Ok(py_list(list.clone())) }
                            else { Err(PyError::runtime_error("copy on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__reversed__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__reversed__".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &*args[0].borrow() {
                                let mut rev = list.clone();
                                rev.reverse();
                                Ok(PyObjectRef::new(PyObject::List(rev)))
                            } else { Err(PyError::runtime_error("__reversed__ on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &*args[0].borrow() {
                                Ok(py_int(56 + (list.len() as i64) * 8))
                            } else { Err(PyError::runtime_error("__sizeof__ on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("__contains__() takes exactly one argument")); }
                            if let PyObject::List(list) = &*args[0].borrow() {
                                for item in list.iter() {
                                    if item.equals(&args[1])? { return Ok(py_bool(true)); }
                                }
                                Ok(py_bool(false))
                            } else { Err(PyError::runtime_error("__contains__ on non-list")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'list' object has no attribute '{}'", name))),
            }
            }
            PyObject::Tuple(_v) => {
                match name {
                    "__reversed__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__reversed__".to_string(),
                        func: |args| {
                            if let PyObject::Tuple(tuple) = &*args[0].borrow() {
                                let mut rev = tuple.clone();
                                rev.reverse();
                                Ok(PyObjectRef::imm(PyObject::Tuple(rev)))
                            } else { Err(PyError::runtime_error("__reversed__ on non-tuple")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Tuple(tuple) = &*args[0].borrow() {
                                Ok(py_int(40 + (tuple.len() as i64) * 8))
                            } else { Err(PyError::runtime_error("__sizeof__ on non-tuple")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'tuple' object has no attribute '{}'", name))),
                }
            }
            PyObject::Bytes(_v) => {
                match name {
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(bytes) = &*args[0].borrow() {
                                Ok(py_int(33 + bytes.len() as i64))
                            } else { Err(PyError::runtime_error("__sizeof__ on non-bytes")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__repr__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__repr__".to_string(),
                        func: |args| {
                            Ok(py_str(&args[0].repr()))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__str__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__str__".to_string(),
                        func: |args| {
                            Ok(py_str(&args[0].str()))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__len__".to_string(),
                        func: |args| {
                            let b = args[0].borrow();
                            if let PyObject::Bytes(bytes) = &*b {
                                Ok(py_int(bytes.len() as i64))
                            } else { Err(PyError::runtime_error("__len__ on non-bytes")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "hex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "hex".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(bytes) = &*args[0].borrow() {
                                let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                                Ok(py_str(&hex))
                            } else { Err(PyError::runtime_error("hex on non-bytes")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "decode" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "decode".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(bytes) = &*args[0].borrow() {
                                let encoding = if args.len() > 1 { args[1].str() } else { "utf-8".to_string() };
                                if encoding == "utf-8" || encoding == "utf8" {
                                    if let Ok(s) = String::from_utf8(bytes.clone()) {
                                        Ok(py_str(&s))
                                    } else {
                                        Err(PyError::value_error("invalid utf-8 sequence".to_string()))
                                    }
                                } else {
                                    let s = String::from_utf8_lossy(bytes).to_string();
                                    Ok(py_str(&s))
                                }
                            } else { Err(PyError::runtime_error("decode on non-bytes")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'bytes' object has no attribute '{}'", name))),
                }
            }
            PyObject::ByteArray(_b) => {
                match name {
                    "append" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "append".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("append() takes exactly one argument")); }
                            let val = args[1].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| PyError::value_error("byte value out of range"))?;
                                if n < 0 || n > 255 { return Err(PyError::value_error("byte must be in range(0, 256)")); }
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    bytes.push(n as u8);
                                    Ok(py_none())
                                } else { Err(PyError::runtime_error("append on non-bytearray")) }
                            } else { Err(PyError::type_error("argument must be an integer")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "extend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "extend".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("extend() takes exactly one argument")); }
                            let it = builtin_iter(&[args[1].clone()])?;
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => {
                                        let vv = v.borrow();
                                        if let PyObject::Int(i) = &*vv {
                                            let n = i.to_i64().ok_or_else(|| PyError::value_error("byte value out of range"))?;
                                            if n < 0 || n > 255 { return Err(PyError::value_error("byte must be in range(0, 256)")); }
                                            if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                                bytes.push(n as u8);
                                            } else { return Err(PyError::runtime_error("extend on non-bytearray")); }
                                        } else { return Err(PyError::type_error("argument must be iterable of integers")); }
                                    }
                                    Err(PyError::StopIteration) => return Ok(py_none()),
                                    Err(e) => return Err(e),
                                }
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "insert" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "insert".to_string(),
                        func: |args| {
                            if args.len() < 3 { return Err(PyError::type_error("insert() takes exactly 2 arguments")); }
                            let idx = args[1].as_i64().unwrap_or(0) as usize;
                            let val = args[2].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| PyError::value_error("byte value out of range"))?;
                                if n < 0 || n > 255 { return Err(PyError::value_error("byte must be in range(0, 256)")); }
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    let idx = idx.min(bytes.len());
                                    bytes.insert(idx, n as u8);
                                    Ok(py_none())
                                } else { Err(PyError::runtime_error("insert on non-bytearray")) }
                            } else { Err(PyError::type_error("argument must be an integer")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "remove" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "remove".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("remove() takes exactly one argument")); }
                            let val = args[1].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| PyError::value_error("byte value out of range"))? as u8;
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    let pos = bytes.iter().position(|&x| x == n)
                                        .ok_or_else(|| PyError::value_error(format!("value {} not found in bytearray", n)))?;
                                    bytes.remove(pos);
                                    Ok(py_none())
                                } else { Err(PyError::runtime_error("remove on non-bytearray")) }
                            } else { Err(PyError::type_error("argument must be an integer")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                let idx = if args.len() > 1 {
                                    let i = args[1].as_i64().ok_or_else(|| PyError::type_error("pop index must be an integer"))?;
                                    let len = bytes.len() as i64;
                                    if i < 0 { len + i } else { i }
                                } else {
                                    bytes.len() as i64 - 1
                                };
                                if idx < 0 || idx >= bytes.len() as i64 {
                                    return Err(PyError::index_error("pop index out of range"));
                                }
                                let val = bytes.remove(idx as usize);
                                Ok(PyObjectRef::imm(PyObject::Int(BigInt::from(val))))
                            } else { Err(PyError::runtime_error("pop on non-bytearray")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("__getitem__() requires an index")); }
                            py_getitem(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__setitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setitem__".to_string(),
                        func: |args| {
                            if args.len() < 3 { return Err(PyError::type_error("__setitem__() requires an index and value")); }
                            py_setitem(&args[0], &args[1], args[2].clone())?;
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__len__".to_string(),
                        func: |args| {
                            let b = args[0].borrow();
                            if let PyObject::ByteArray(bytes) = &*b {
                                Ok(py_int(bytes.len() as i64))
                            } else { Err(PyError::runtime_error("__len__ on non-bytearray")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__str__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__str__".to_string(),
                        func: |args| {
                            if let PyObject::ByteArray(bytes) = &*args[0].borrow() {
                                let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                                Ok(py_str(&hex))
                            } else { Err(PyError::runtime_error("__str__ on non-bytearray")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::ByteArray(bytes) = &*args[0].borrow() {
                                Ok(py_int(33 + bytes.len() as i64))
                            } else { Err(PyError::runtime_error("__sizeof__ on non-bytearray")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'bytearray' object has no attribute '{}'", name))),
                }
            }
            PyObject::Str(_s) => {
                match name {
                    "format" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "format".to_string(),
                        func: |args| {
                            if args.is_empty() { return Err(PyError::type_error("format() takes at least 1 argument")); }
                            let fmt = args[0].str();
                            let mut result = String::new();
                            let mut chars = fmt.chars();
                            let mut pos = 0usize;
                            let mut next_auto = 0usize;
                            while let Some(c) = chars.next() {
                                if c == '{' {
                                    // Check for {{ escape
                                    if chars.as_str().starts_with('{') {
                                        result.push('{');
                                        chars.next();
                                        continue;
                                    }
                                    // Parse field name: {name} or {0} or {}
                                    let mut field = String::new();
                                    loop {
                                        match chars.next() {
                                            Some('}') => break,
                                            Some(c) => field.push(c),
                                            None => return Err(PyError::value_error("unterminated format field")),
                                        }
                                    }
                                    // Determine the value
                                    let val = if field.is_empty() {
                                        // Auto-numbering: {}
                                        let idx = next_auto;
                                        next_auto += 1;
                                        if idx + 1 < args.len() { Some(args[idx + 1].clone()) } else { None }
                                    } else if let Ok(n) = field.parse::<usize>() {
                                        // Positional: {0}, {1}
                                        if n + 1 < args.len() { Some(args[n + 1].clone()) } else { None }
                                    } else {
                                        // Named: {name}
                                        None  // Named args not supported in this simplified version
                                    };
                                    match val {
                                        Some(v) => result.push_str(&v.str()),
                                        None => result.push_str(&field),
                                    }
                                } else if c == '}' {
                                    if chars.as_str().starts_with('}') {
                                        result.push('}');
                                        chars.next();
                                    }
                                } else {
                                    result.push(c);
                                }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "split" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "split".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            let sep = if args.len() > 1 { Some(args[1].str()) } else { None };
                            let parts: Vec<PyObjectRef> = if let Some(sep) = sep { s.split(&sep).map(|p| py_str(p)).collect() } else { s.split_whitespace().map(|p| py_str(p)).collect() };
                            Ok(py_list(parts))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("join() takes exactly one argument")); }
                            let sep = args[0].str();
                            let items = args[1].borrow();
                            let parts: Vec<String> = if let PyObject::List(v) = &*items { v.iter().map(|x| x.str()).collect() } else { return Err(PyError::type_error("join() argument must be a list")) };
                            Ok(py_str(&parts.join(&sep)))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "upper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "upper".to_string(),
                        func: |args| Ok(py_str(&args[0].str().to_uppercase())),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lower".to_string(),
                        func: |args| Ok(py_str(&args[0].str().to_lowercase())),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "strip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "strip".to_string(),
                        func: |args| {
                            let chars = if args.len() > 1 { args[1].str() } else { " \t\n\r".to_string() };
                            Ok(py_str(args[0].str().trim_matches(|c: char| chars.contains(c))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lstrip".to_string(),
                        func: |args| {
                            let chars = if args.len() > 1 { args[1].str() } else { " \t\n\r".to_string() };
                            Ok(py_str(args[0].str().trim_start_matches(|c: char| chars.contains(c))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rstrip".to_string(),
                        func: |args| {
                            let chars = if args.len() > 1 { args[1].str() } else { " \t\n\r".to_string() };
                            Ok(py_str(args[0].str().trim_end_matches(|c: char| chars.contains(c))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "startswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "startswith".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("startswith() takes exactly one argument")); }
                            Ok(py_bool(args[0].str().starts_with(&args[1].str())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "endswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "endswith".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("endswith() takes exactly one argument")); }
                            Ok(py_bool(args[0].str().ends_with(&args[1].str())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "find" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "find".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("find() takes at least 1 argument")); }
                            match args[0].str().find(&args[1].str()) {
                                Some(i) => Ok(py_int(i as i64)),
                                None => Ok(py_int(-1)),
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "replace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "replace".to_string(),
                        func: |args| {
                            if args.len() < 3 { return Err(PyError::type_error("replace() takes exactly 2 arguments")); }
                            Ok(py_str(&args[0].str().replace(&args[1].str(), &args[2].str())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdecimal" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isdecimal".to_string(), func: |a| Ok(py_bool(!a[0].str().is_empty() && a[0].str().chars().all(|c| c.is_ascii_digit() && !c.is_ascii_control()))), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isnumeric" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isnumeric".to_string(), func: |a| Ok(py_bool(!a[0].str().is_empty() && a[0].str().chars().any(|c| c.is_numeric()))), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isascii" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isascii".to_string(), func: |a| Ok(py_bool(a[0].str().is_ascii())), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isprintable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isprintable".to_string(), func: |a| Ok(py_bool(!a[0].str().is_empty() && a[0].str().chars().all(|c| c.is_ascii_graphic() || c == ' '))), self_obj: PyObjectRef::new(PyObject::None) })),
                    "casefold" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "casefold".to_string(), func: |a| Ok(py_str(&a[0].str().to_lowercase())), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isdigit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isdigit".to_string(), func: |a| Ok(py_bool(a[0].str().chars().all(|c| c.is_ascii_digit()))), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isalpha" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isalpha".to_string(), func: |a| Ok(py_bool(a[0].str().chars().all(|c| c.is_ascii_alphabetic()))), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isalnum" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isalnum".to_string(), func: |a| Ok(py_bool(a[0].str().chars().all(|c| c.is_ascii_alphanumeric()))), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isspace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isspace".to_string(), func: |a| Ok(py_bool(a[0].str().chars().all(|c| c.is_ascii_whitespace()))), self_obj: PyObjectRef::new(PyObject::None) })),
                    "islower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "islower".to_string(), func: |a| Ok(py_bool(a[0].str() == a[0].str().to_lowercase())), self_obj: PyObjectRef::new(PyObject::None) })),
                    "isupper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "isupper".to_string(), func: |a| Ok(py_bool(a[0].str() == a[0].str().to_uppercase())), self_obj: PyObjectRef::new(PyObject::None) })),
                    "istitle" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "istitle".to_string(), func: |a| { let s = a[0].str(); let mut prev_is_letter = false; let mut is_title = true; for c in s.chars() { if c.is_ascii_uppercase() { if prev_is_letter { is_title = false; break; } prev_is_letter = true; } else if c.is_ascii_lowercase() { if !prev_is_letter { is_title = false; break; } prev_is_letter = true; } else { prev_is_letter = false; } } Ok(py_bool(is_title && !s.is_empty())) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "title" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "title".to_string(), func: |a| { let s = a[0].str(); let mut result = String::with_capacity(s.len()); let mut prev_cased = false; for c in s.chars() { if c.is_uppercase() || c.is_lowercase() { if !prev_cased { result.extend(c.to_uppercase()); } else { result.extend(c.to_lowercase()); } prev_cased = true; } else { result.push(c); prev_cased = false; } } Ok(py_str(&result)) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "capitalize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "capitalize".to_string(), func: |a| { let s = a[0].str(); let mut c = s.chars(); Ok(py_str(&match c.next() { Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(), None => String::new() })) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "swapcase" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "swapcase".to_string(), func: |a| { let s = a[0].str(); let mut result = String::with_capacity(s.len()); for c in s.chars() { if c.is_uppercase() { result.extend(c.to_lowercase()); } else if c.is_lowercase() { result.extend(c.to_uppercase()); } else { result.push(c); } } Ok(py_str(&result)) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "zfill" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "zfill".to_string(), func: |a| { if a.len() < 2 { return Err(PyError::type_error("zfill() takes exactly 1 argument")); } let w = a[1].as_i64().unwrap_or(0) as usize; let s = a[0].str(); if w <= s.len() { return Ok(py_str(&s)); } let (sign, rest) = if let Some(stripped) = s.strip_prefix('+').or_else(|| s.strip_prefix('-')) { (&s[..1], stripped) } else { ("", s.as_str()) }; let padded = format!("{}{:0>width$}", sign, rest, width = w - sign.len()); Ok(py_str(&padded)) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "ljust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "ljust".to_string(), func: |a| if a.len() < 2 { return Err(PyError::type_error("ljust() takes exactly 1 argument")); } else { let w = a[1].as_i64().unwrap_or(0) as usize; let fill = if a.len() > 2 { let f = a[2].str(); f.chars().next().unwrap_or(' ') } else { ' ' }; let s = a[0].str(); let padding = if w > s.len() { fill.to_string().repeat(w - s.len()) } else { String::new() }; Ok(py_str(&(s.to_string() + &padding))) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "rjust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "rjust".to_string(), func: |a| { if a.len() < 2 { return Err(PyError::type_error("rjust() takes exactly 1 argument")); } let w = a[1].as_i64().unwrap_or(0) as usize; let fill = if a.len() > 2 { a[2].str().chars().next().unwrap_or(' ') } else { ' ' }; let s = a[0].str(); if w <= s.len() { Ok(py_str(&s)) } else { Ok(py_str(&(fill.to_string().repeat(w - s.len()) + &s))) } }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "center" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "center".to_string(), func: |a| { if a.len() < 2 { return Err(PyError::type_error("center() takes exactly 1 argument")); } let w = a[1].as_i64().unwrap_or(0) as usize; let fill = if a.len() > 2 { a[2].str().chars().next().unwrap_or(' ') } else { ' ' }; let s = a[0].str(); if w <= s.len() { Ok(py_str(&s)) } else { let pad = w - s.len(); let left = pad / 2; let right = pad - left; let fill_s = fill.to_string(); Ok(py_str(&(fill_s.repeat(left) + &s + &fill_s.repeat(right)))) } }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "removeprefix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "removeprefix".to_string(), func: |a| if a.len() < 2 { return Err(PyError::type_error("removeprefix() takes exactly 1 argument")); } else { let s = a[0].str(); let p = a[1].str(); Ok(py_str(if s.starts_with(&p) { &s[p.len()..] } else { &s })) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "removesuffix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "removesuffix".to_string(), func: |a| if a.len() < 2 { return Err(PyError::type_error("removesuffix() takes exactly 1 argument")); } else { let s = a[0].str(); let p = a[1].str(); Ok(py_str(if s.ends_with(&p) { &s[..s.len()-p.len()] } else { &s })) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "__mod__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mod__".to_string(),
                        func: |args| {
                            // args[0] = self_obj (py_none), args[1] = format string, args[2] = value
                            if args.len() < 3 { return Err(PyError::type_error("__mod__() too few args")); }
                            let fmt = args[1].str();
                            let result = string_interpolate(&fmt, &args[2]).map_err(|e| PyError::runtime_error(e))?;
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "partition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "partition".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("partition() takes exactly one argument")); }
                            let s = args[0].str();
                            let sep = args[1].str();
                            if let Some(pos) = s.find(&sep) {
                                Ok(py_tuple(vec![
                                    py_str(&s[..pos]),
                                    py_str(&sep),
                                    py_str(&s[pos + sep.len()..]),
                                ]))
                            } else {
                                Ok(py_tuple(vec![py_str(&s), py_str(""), py_str("")]))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rpartition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rpartition".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("rpartition() takes exactly one argument")); }
                            let s = args[0].str();
                            let sep = args[1].str();
                            if let Some(pos) = s.rfind(&sep) {
                                Ok(py_tuple(vec![
                                    py_str(&s[..pos]),
                                    py_str(&sep),
                                    py_str(&s[pos + sep.len()..]),
                                ]))
                            } else {
                                Ok(py_tuple(vec![py_str(""), py_str(""), py_str(&s)]))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "splitlines" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "splitlines".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            let keepends = if args.len() > 1 { args[1].truthy() } else { false };
                            let mut lines: Vec<PyObjectRef> = Vec::new();
                            let mut start = 0;
                            let chars: Vec<char> = s.chars().collect();
                            let len = chars.len();
                            let mut i = 0;
                            while i < len {
                                let mut end = i;
                                let mut line_end = i;
                                if chars[i] == '\r' {
                                    if i + 1 < len && chars[i + 1] == '\n' {
                                        line_end = i + 2;
                                    } else {
                                        line_end = i + 1;
                                    }
                                } else if chars[i] == '\n' {
                                    line_end = i + 1;
                                } else {
                                    i += 1;
                                    continue;
                                }
                                if keepends {
                                    end = line_end;
                                } else {
                                    end = i;
                                }
                                let line: String = chars[start..end].iter().collect();
                                lines.push(py_str(&line));
                                i = line_end;
                                start = i;
                            }
                            if start < len || s.ends_with('\n') || s.is_empty() || lines.is_empty() {
                                let line: String = chars[start..].iter().collect();
                                if !line.is_empty() || s.ends_with('\n') || (s.is_empty() && lines.is_empty()) {
                                    lines.push(py_str(&line));
                                }
                            }
                            Ok(py_list(lines))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "expandtabs" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "expandtabs".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            let tabsize = if args.len() > 1 { args[1].as_i64().unwrap_or(8) as usize } else { 8 };
                            let mut result = String::with_capacity(s.len());
                            let mut col = 0;
                            for c in s.chars() {
                                if c == '\t' {
                                    let spaces = tabsize - (col % tabsize);
                                    result.push_str(&" ".repeat(spaces));
                                    col += spaces;
                                } else if c == '\n' || c == '\r' {
                                    result.push(c);
                                    col = 0;
                                } else {
                                    result.push(c);
                                    col += 1;
                                }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "translate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "translate".to_string(), func: |a| { let s = a[0].str(); if a.len() > 1 { let _table = &a[1]; } Ok(py_str(&s)) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "encode" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: "encode".to_string(), func: |a| { let s = a[0].str(); if a.len() > 1 { let _encoding = a[1].str(); } Ok(PyObjectRef::imm(PyObject::Bytes(s.as_bytes().to_vec()))) }, self_obj: PyObjectRef::new(PyObject::None) })),
                    "isidentifier" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isidentifier".to_string(),
                        func: |a| {
                            let s = a[0].str();
                            if s.is_empty() { return Ok(py_bool(false)); }
                            let mut chars = s.chars();
                            let first = chars.next().unwrap();
                            let valid = (first == '_') || first.is_ascii_alphabetic();
                            if !valid { return Ok(py_bool(false)); }
                            Ok(py_bool(chars.all(|c| c == '_' || c.is_ascii_alphanumeric())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |a| {
                            let s = a[0].str();
                            Ok(py_int(49 + s.len() as i64))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'str' object has no attribute '{}'", name))),
                }
            }
            PyObject::Dict(_d) => {
                match name {
                    "keys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "keys".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d { Ok(py_list(dict.keys())) }
                            else { Err(PyError::runtime_error("keys on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "values" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "values".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d { Ok(py_list(dict.values())) }
                            else { Err(PyError::runtime_error("values on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "items" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "items".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                let items: Vec<PyObjectRef> = dict.items().iter().map(|(k, v)| py_tuple(vec![k.clone(), v.clone()])).collect();
                                Ok(py_list(items))
                            } else { Err(PyError::runtime_error("items on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("get() takes at least 1 argument")); }
                            let dict = &*args[0].borrow();
                            if let PyObject::Dict(d) = dict {
                                Ok(d.get(&args[1])?.unwrap_or_else(|| if args.len() > 2 { args[2].clone() } else { py_none() }))
                            } else { Err(PyError::runtime_error("get on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("pop() takes at least 1 argument")); }
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                match d.remove(&args[1]) {
                                    Ok(val) => Ok(val),
                                    Err(_) if args.len() > 2 => Ok(args[2].clone()),
                                    Err(e) => Err(e),
                                }
                            } else { Err(PyError::runtime_error("pop on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "popitem" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "popitem".to_string(),
                        func: |args| {
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                let items = d.items();
                                if items.is_empty() { return Err(PyError::key_error("popitem(): dictionary is empty")); }
                                let last = args.len() <= 2 || args[1].truthy();
                                let (k, v) = if last { items.into_iter().last().unwrap() }
                                    else { items.into_iter().next().unwrap() };
                                d.remove(&k)?;
                                Ok(py_tuple(vec![k, v]))
                            } else { Err(PyError::runtime_error("popitem on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() { d.clear(); Ok(py_none()) }
                            else { Err(PyError::runtime_error("clear on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "update".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("update() takes at least 1 argument")); }
                            let other = args[1].borrow();
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                if let PyObject::Dict(other_dict) = &*other {
                                    for (k, v) in other_dict.items() { d.set(k, v)?; }
                                    Ok(py_none())
                                } else { Err(PyError::type_error("update() argument must be a dict")) }
                            } else { Err(PyError::runtime_error("update on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "setdefault" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setdefault".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("setdefault() takes at least 1 argument")); }
                            let key = args[1].clone();
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                match d.get(&key)? {
                                    Some(val) => Ok(val.clone()),
                                    None => {
                                        let val = if args.len() > 2 { args[2].clone() } else { py_none() };
                                        d.set(key, val.clone())?; Ok(val)
                                    }
                                }
                            } else { Err(PyError::runtime_error("setdefault on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                let mut new_dict = PyDict::new();
                                for (k, v) in dict.items() { new_dict.set(k, v)?; }
                                Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
                            } else { Err(PyError::runtime_error("copy on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "fromkeys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "fromkeys".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("fromkeys() takes at least 1 argument")); }
                            let mut new_dict = PyDict::new();
                            let val = if args.len() > 2 { args[2].clone() } else { py_none() };
                            let it = builtin_iter(&[args[1].clone()])?;
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(k) => { new_dict.set(k, val.clone())?; }
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(py_int(72 + (d.len() as i64) * 16))
                            } else { Err(PyError::runtime_error("__sizeof__ on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("__contains__() takes exactly one argument")); }
                            if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(py_bool(d.contains(&args[1])?))
                            } else { Err(PyError::runtime_error("__contains__ on non-dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "move_to_end" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "move_to_end".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("move_to_end() needs a key argument")); }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__or__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__or__".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("__or__() takes exactly one argument")); }
                            let other = args[1].borrow();
                            if let PyObject::Dict(other_dict) = &*other {
                                let d = args[0].borrow();
                                if let PyObject::Dict(dict) = &*d {
                                    let mut new_dict = PyDict::new();
                                    for (k, v) in dict.items() { new_dict.set(k, v)?; }
                                    for (k, v) in other_dict.items() { new_dict.set(k, v)?; }
                                    Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
                                } else { Err(PyError::runtime_error("__or__ on non-dict")) }
                            } else { Err(PyError::type_error("__or__() argument must be a dict")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'dict' object has no attribute '{}'", name))),
                }
            }
            PyObject::Set(s) => {
                match name {
                    "add" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "add".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("add() takes exactly one argument")); }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() { set.add(args[1].clone())?; Ok(py_none()) }
                            else { Err(PyError::runtime_error("add on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "remove" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "remove".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("remove() takes exactly one argument")); }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() { set.remove(&args[1]) }
                            else { Err(PyError::runtime_error("remove on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "discard" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "discard".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("discard() takes exactly one argument")); }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                let _ = set.remove(&args[1]); Ok(py_none())
                            } else { Err(PyError::runtime_error("discard on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() { set.pop().ok_or_else(|| PyError::key_error("pop from an empty set")) }
                            else { Err(PyError::runtime_error("pop on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() { set.clear(); Ok(py_none()) }
                            else { Err(PyError::runtime_error("clear on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s { Ok(PyObjectRef::new(PyObject::Set(set.clone()))) }
                            else { Err(PyError::runtime_error("copy on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Set(set) = &*args[0].borrow() {
                                Ok(py_int(72 + (set.len() as i64) * 8))
                            } else { Err(PyError::runtime_error("__sizeof__ on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "union" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "union".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("union() takes at least 1 argument")); }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let mut result = set.clone();
                                for other_arg in &args[1..] {
                                    let other = other_arg.borrow();
                                    if let PyObject::Set(other_set) = &*other {
                                        for item in other_set.to_vec() { result.add(item)?; }
                                    }
                                }
                                Ok(PyObjectRef::new(PyObject::Set(result)))
                            } else { Err(PyError::runtime_error("union on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "intersection" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "intersection".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("intersection() takes at least 1 argument")); }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let mut result = PySet::new();
                                for item in set.to_vec() {
                                    let in_all = args[1..].iter().all(|other_arg| {
                                        let other = other_arg.borrow();
                                        if let PyObject::Set(other_set) = &*other { other_set.contains(&item).unwrap_or(false) }
                                        else { false }
                                    });
                                    if in_all { result.add(item)?; }
                                }
                                Ok(PyObjectRef::new(PyObject::Set(result)))
                            } else { Err(PyError::runtime_error("intersection on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "difference" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "difference".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("difference() takes at least 1 argument")); }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let mut result = PySet::new();
                                for item in set.to_vec() {
                                    let in_any = args[1..].iter().any(|other_arg| {
                                        let other = other_arg.borrow();
                                        if let PyObject::Set(other_set) = &*other { other_set.contains(&item).unwrap_or(false) }
                                        else { false }
                                    });
                                    if !in_any { result.add(item)?; }
                                }
                                Ok(PyObjectRef::new(PyObject::Set(result)))
                            } else { Err(PyError::runtime_error("difference on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "symmetric_difference" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "symmetric_difference".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("symmetric_difference() takes exactly one argument")); }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let other = args[1].borrow();
                                if let PyObject::Set(other_set) = &*other {
                                    let mut result = PySet::new();
                                    for item in set.to_vec() { if !other_set.contains(&item).unwrap_or(false) { result.add(item)?; } }
                                    for item in other_set.to_vec() { if !set.contains(&item).unwrap_or(false) { result.add(item)?; } }
                                    Ok(PyObjectRef::new(PyObject::Set(result)))
                                } else { Err(PyError::type_error("symmetric_difference() argument must be a set")) }
                            } else { Err(PyError::runtime_error("symmetric_difference on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "issubset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issubset".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("issubset() takes exactly one argument")); }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let other_set = convert_to_set(&args[1])?;
                                Ok(py_bool(set.to_vec().iter().all(|item| other_set.contains(item).unwrap_or(false))))
                            } else { Err(PyError::runtime_error("issubset on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "issuperset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issuperset".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("issuperset() takes exactly one argument")); }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let other_set = convert_to_set(&args[1])?;
                                Ok(py_bool(other_set.to_vec().iter().all(|item| set.contains(item).unwrap_or(false))))
                            } else { Err(PyError::runtime_error("issuperset on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdisjoint" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdisjoint".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("isdisjoint() takes exactly one argument")); }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let other = args[1].borrow();
                                if let PyObject::Set(other_set) = &*other {
                                    Ok(py_bool(!set.to_vec().iter().any(|item| other_set.contains(item).unwrap_or(false))))
                                } else { Err(PyError::type_error("isdisjoint() argument must be a set")) }
                            } else { Err(PyError::runtime_error("isdisjoint on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "update".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("update() takes at least 1 argument")); }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                for other_arg in &args[1..] {
                                    let other = other_arg.borrow();
                                    if let PyObject::Set(other_set) = &*other {
                                        for item in other_set.to_vec() { set.add(item)?; }
                                    }
                                }
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("update on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "intersection_update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "intersection_update".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("intersection_update() takes at least 1 argument")); }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                let items: Vec<PyObjectRef> = set.to_vec().iter().filter(|item| {
                                    args[1..].iter().all(|other_arg| {
                                        let other = other_arg.borrow();
                                        if let PyObject::Set(other_set) = &*other { other_set.contains(item).unwrap_or(false) }
                                        else { false }
                                    })
                                }).cloned().collect();
                                set.clear();
                                for item in items { set.add(item)?; }
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("intersection_update on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "difference_update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "difference_update".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("difference_update() takes at least 1 argument")); }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                let items: Vec<PyObjectRef> = set.to_vec().iter().filter(|item| {
                                    !args[1..].iter().any(|other_arg| {
                                        let other = other_arg.borrow();
                                        if let PyObject::Set(other_set) = &*other { other_set.contains(item).unwrap_or(false) }
                                        else { false }
                                    })
                                }).cloned().collect();
                                set.clear();
                                for item in items { set.add(item)?; }
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("difference_update on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "symmetric_difference_update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "symmetric_difference_update".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("symmetric_difference_update() takes exactly one argument")); }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                let other = args[1].borrow();
                                if let PyObject::Set(other_set) = &*other {
                                    for item in other_set.to_vec() {
                                        if set.contains(&item).unwrap_or(false) { set.remove(&item)?; }
                                        else { set.add(item)?; }
                                    }
                                    Ok(py_none())
                                } else { Err(PyError::type_error("symmetric_difference_update() argument must be a set")) }
                            } else { Err(PyError::runtime_error("symmetric_difference_update on non-set")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'set' object has no attribute '{}'", name))),
                }
            }
            PyObject::Function { name: func_name, dict, .. } => {
                match name {
                    "__name__" => Ok(dict.get("__name__").cloned().unwrap_or(py_str(func_name))),
                    "__qualname__" => Ok(dict.get("__qualname__").cloned().unwrap_or(py_str(func_name))),
                    "__doc__" => Ok(dict.get("__doc__").cloned().unwrap_or(py_none())),
                    "__code__" => Ok(dict.get("__code__").cloned().unwrap_or(py_none())),
                    "__globals__" => Ok(dict.get("__globals__").cloned().unwrap_or(py_none())),
                    "__defaults__" => Ok(dict.get("__defaults__").cloned().unwrap_or(py_none())),
                    "__closure__" => Ok(dict.get("__closure__").cloned().unwrap_or(py_none())),
                    "__module__" => Ok(dict.get("__module__").cloned().unwrap_or(py_none())),
                    "__annotations__" => Ok(dict.get("__annotations__").cloned().unwrap_or(py_none())),
                    _ => dict.get_str(&name).cloned().ok_or_else(|| PyError::attribute_error(format!(
                        "'function' object has no attribute '{}'", name
                    ))),
                }
            }
            PyObject::Generator { frame: gen_frame } => {
                match name {
                    "__next__" | "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: move |args| {
                            let gen = args[0].borrow();
                            if let PyObject::Generator { frame } = &*gen {
                                let mut frame_opt = frame.borrow_mut();
                                if let Some(f) = frame_opt.as_mut() {
                                    // Push the sent value onto the stack (None for __next__, value for send())
                                    if args.len() > 1 {
                                        f.stack.push(args[1].clone());
                                    } else {
                                        f.stack.push(crate::object::py_none());
                                    }
                                    let mut vm = super::vm::VirtualMachine::new();
                                    vm.frames.push(f.clone());
                                    match vm.execute() {
                                        Ok(val) => {
                                            let modified = vm.frames.pop().unwrap();
                                            if modified.ip > 0 && matches!(&modified.code.instructions[modified.ip - 1].op, crate::bytecode::Opcode::YIELD_VALUE) {
                                                *f = modified;
                                                Ok(val)
                                            } else {
                                                *frame_opt = None;
                                                // Propagate the return value via StopIteration for SEND
                                                Err(crate::object::PyError::Exception("StopIteration".to_string(), val))
                                            }
                                        }
                                        Err(e) => {
                                            *frame_opt = None;
                                            if matches!(&e, crate::object::PyError::StopIteration) {
                                                return Err(e);
                                            }
                                            // Wrapping other exceptions as StopIteration with value
                                            Err(crate::object::PyError::Exception("StopIteration".to_string(), crate::object::py_none()))
                                        }
                                    }
                                } else {
                                    Err(PyError::StopIteration)
                                }
                            } else {
                                Err(PyError::runtime_error("__next__ on non-generator"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "throw" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "throw".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("throw() needs at least 1 argument")); }
                            Err(PyError::runtime_error("generator throw not implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "close".to_string(),
                        func: |args| {
                            let gen = args[0].borrow();
                            if let PyObject::Generator { frame } = &*gen {
                                let mut frame_opt = frame.borrow_mut();
                                *frame_opt = None;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__await__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__await__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'generator' object has no attribute '{}'", name))),
                }
            }
            PyObject::Coroutine { frame: _coro_frame } => {
                match name {
                    "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send".to_string(),
                        func: move |args| {
                            let gen = args[0].borrow();
                            if let PyObject::Coroutine { frame } = &*gen {
                                let mut frame_opt = frame.borrow_mut();
                                if let Some(f) = frame_opt.as_mut() {
                                    if args.len() > 1 {
                                        f.stack.push(args[1].clone());
                                    } else {
                                        f.stack.push(crate::object::py_none());
                                    }
                                    let mut vm = super::vm::VirtualMachine::new();
                                    vm.frames.push(f.clone());
                                    match vm.execute() {
                                        Ok(val) => {
                                            let modified = vm.frames.pop().unwrap();
                                            if modified.ip > 0 && matches!(&modified.code.instructions[modified.ip - 1].op, crate::bytecode::Opcode::YIELD_VALUE) {
                                                *f = modified;
                                                Ok(val)
                                            } else {
                                                *frame_opt = None;
                                                // Propagate the return value via StopIteration for SEND
                                                Err(crate::object::PyError::Exception("StopIteration".to_string(), val))
                                            }
                                        }
                                        Err(e) => {
                                            *frame_opt = None;
                                            if matches!(&e, crate::object::PyError::StopIteration) {
                                                return Err(e);
                                            }
                                            // Wrap other exceptions as StopIteration with value
                                            Err(crate::object::PyError::Exception("StopIteration".to_string(), crate::object::py_none()))
                                        }
                                    }
                                } else {
                                    Err(PyError::StopIteration)
                                }
                            } else {
                                Err(PyError::runtime_error("send on non-coroutine"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "throw" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "throw".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("throw() needs at least 1 argument")); }
                            Err(PyError::runtime_error("coroutine throw not implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "close".to_string(),
                        func: |args| {
                            let gen = args[0].borrow();
                            if let PyObject::Coroutine { frame } = &*gen {
                                let mut frame_opt = frame.borrow_mut();
                                *frame_opt = None;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__await__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__await__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'coroutine' object has no attribute '{}'", name))),
                }
            }
            PyObject::File { file: _, .. } => {
                match name {
                    "name" => {
                        if let PyObject::File { name: fname, .. } = &*self {
                            Ok(py_str(fname))
                        } else {
                            Err(PyError::runtime_error("name access on non-file"))
                        }
                    }
                    "read" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "read".to_string(),
                        func: |args| {
                            use std::io::Read;
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                let mut buf = String::new();
                                file.borrow_mut().read_to_string(&mut buf).map_err(|e| PyError::OsError(format!("{}", e)))?;
                                Ok(py_str(&buf))
                            } else { Err(PyError::runtime_error("read on non-file")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "write" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "write".to_string(),
                        func: |args| {
                            use std::io::Write;
                            if args.len() < 2 { return Err(PyError::type_error("write() takes exactly one argument")); }
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                let text = args[1].str();
                                file.borrow_mut().write_all(text.as_bytes()).map_err(|e| PyError::OsError(format!("{}", e)))?;
                                Ok(py_int(text.len() as i64))
                            } else { Err(PyError::runtime_error("write on non-file")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "close".to_string(),
                        func: |args| {
                            if let PyObject::File { file, .. } = &mut *args[0].borrow_mut() {
                                // Flush and drop by replacing with a closed file
                                let _ = std::mem::replace(&mut *file.borrow_mut(), std::fs::File::create("/dev/null").unwrap_or(std::fs::File::open("/dev/null").unwrap_or_else(|_| panic!())));
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("close on non-file")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            // args[0] = file_obj (normal path via LOAD_ATTR) or py_none (exception path via WITH_EXIT)
                            // args[1] = exc_type (normal) or file_obj (exception via BoundMethod wrapper)
                            // Find the file object: check args[0], then args[1]
                            let file_obj_idx = if args.len() > 0 && matches!(&*args[0].borrow(), PyObject::File { .. }) { 0 }
                                              else if args.len() > 1 && matches!(&*args[1].borrow(), PyObject::File { .. }) { 1 }
                                              else { return Ok(py_none()) };
                            // Sync and flush data to disk
                            if let PyObject::File { file, .. } = &*args[file_obj_idx].borrow() {
                                let _ = file.borrow().sync_all();
                            }
                            // Replace with /dev/null to close the actual file descriptor
                            if let PyObject::File { file, .. } = &mut *args[file_obj_idx].borrow_mut() {
                                let _ = std::mem::replace(&mut *file.borrow_mut(), std::fs::File::open("/dev/null").unwrap_or_else(|_| {
                                    std::fs::File::create("/dev/null").unwrap()
                                }));
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'file' object has no attribute '{}'", name))),
                }
            }
            PyObject::Socket { inner } => {
                match name {
                    "bind" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "bind".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("bind() takes exactly 1 argument")); }
                            let addr = socket_addr_to_string(&args[1])?;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                match &*inner {
                                    SocketInner::Uninitialized => {
                                        let listener = std::net::TcpListener::bind(&addr)
                                            .map_err(|e| PyError::OsError(format!("{}", e)))?;
                                        listener.set_nonblocking(true).ok();
                                        *inner = SocketInner::TcpListener(listener);
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::runtime_error("socket already bound or connected")),
                                }
                            } else { Err(PyError::runtime_error("bind on non-socket")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "listen" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "listen".to_string(),
                        func: |args| {
                            let backlog = if args.len() > 1 { args[1].as_i64().unwrap_or(5) as i32 } else { 5 };
                            let _ = backlog;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let inner = inner.borrow();
                                match &*inner {
                                    SocketInner::TcpListener(_listener) => {
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::runtime_error("listen on non-listener")),
                                }
                            } else { Err(PyError::runtime_error("listen on non-socket")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "accept" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "accept".to_string(),
                        func: |args| {
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                let old = std::mem::replace(&mut *inner, SocketInner::Uninitialized);
                                match old {
                                    SocketInner::TcpListener(listener) => {
                                        match listener.accept() {
                                            Ok((stream, addr)) => {
                                                *inner = SocketInner::TcpListener(listener);
                                                let client = PyObjectRef::new(PyObject::Socket {
                                                    inner: std::rc::Rc::new(std::cell::RefCell::new(SocketInner::TcpStream(stream))),
                                                });
                                                Ok(py_tuple(vec![client, py_str(&addr.to_string())]))
                                            }
                                            Err(e) => {
                                                *inner = SocketInner::TcpListener(listener);
                                                Err(PyError::OsError(format!("{}", e)))
                                            }
                                        }
                                    }
                                    other => {
                                        *inner = other;
                                        Err(PyError::runtime_error("accept on non-listener"))
                                    }
                                }
                            } else { Err(PyError::runtime_error("accept on non-socket")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "connect" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "connect".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("connect() takes exactly 1 argument")); }
                            let addr = socket_addr_to_string(&args[1])?;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                match &*inner {
                                    SocketInner::Uninitialized => {
                                        match std::net::TcpStream::connect(&addr) {
                                            Ok(stream) => {
                                                stream.set_nonblocking(true).ok();
                                                *inner = SocketInner::TcpStream(stream);
                                                Ok(py_none())
                                            }
                                            Err(e) => Err(PyError::OsError(format!("{}", e))),
                                        }
                                    }
                                    _ => Err(PyError::runtime_error("socket already connected or listening")),
                                }
                            } else { Err(PyError::runtime_error("connect on non-socket")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("send() takes exactly 1 argument")); }
                            let data = args[1].str();
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                match &mut *inner {
                                    SocketInner::TcpStream(stream) => {
                                        use std::io::Write;
                                        match stream.write_all(data.as_bytes()) {
                                            Ok(()) => Ok(py_int(data.len() as i64)),
                                            Err(e) => Err(PyError::OsError(format!("{}", e))),
                                        }
                                    }
                                    _ => Err(PyError::runtime_error("send on non-stream")),
                                }
                            } else { Err(PyError::runtime_error("send on non-socket")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "recv" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "recv".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("recv() takes exactly 1 argument")); }
                            let bufsize = args[1].as_i64().unwrap_or(4096) as usize;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                match &mut *inner {
                                    SocketInner::TcpStream(stream) => {
                                        use std::io::Read;
                                        let mut buf = vec![0u8; bufsize.min(65536)];
                                        match stream.read(&mut buf) {
                                            Ok(0) => Ok(py_str("")),
                                            Ok(n) => {
                                                buf.truncate(n);
                                                match String::from_utf8(buf) {
                                                    Ok(s) => Ok(py_str(&s)),
                                                    Err(_) => Ok(py_str("<binary>")),
                                                }
                                            }
                                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                                Ok(py_none())
                                            }
                                            Err(e) => Err(PyError::OsError(format!("{}", e))),
                                        }
                                    }
                                    _ => Err(PyError::runtime_error("recv on non-stream")),
                                }
                            } else { Err(PyError::runtime_error("recv on non-socket")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "close".to_string(),
                        func: |args| {
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                let old = std::mem::replace(&mut *inner, SocketInner::Uninitialized);
                                drop(old);
                                Ok(py_none())
                            } else { Err(PyError::runtime_error("close on non-socket")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "setsockopt" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setsockopt".to_string(),
                        func: |_| Ok(py_none()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'socket' object has no attribute '{}'", name))),
                }
            }
            PyObject::Thread(inner_arc) => {
                let inner_arc = inner_arc.clone();
                match name {
                    "start" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "start".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Thread(inner_arc) = &*obj {
                                let mut locked = inner_arc.lock().unwrap();
                                if locked.handle.is_some() {
                                    return Err(PyError::runtime_error("thread already started"));
                                }
                                let target = locked.target.clone();
                                let thread_args = locked.args.clone();
                                // Don't create a real thread (PyObjectRef is !Send)
                                // Thread runs synchronously instead
                                let result = locked.result.clone();
                                let call_result = crate::object::builtin_call(&target, &thread_args);
                                match call_result {
                                    Ok(val) => {
                                        *result.lock().unwrap() = Some(val);
                                    }
                                    Err(e) => {
                                        eprintln!("Thread raised: {}", e);
                                    }
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Thread(inner_arc) = &*obj {
                                let mut locked = inner_arc.lock().unwrap();
                                if let Some(handle) = locked.handle.take() {
                                    handle.join().map_err(|_| PyError::runtime_error("thread panicked"))?;
                                    return Ok(locked.result.lock().unwrap().clone().unwrap_or_else(|| py_none()));
                                }
                            }
                            Err(PyError::runtime_error("thread not started"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "is_alive" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "is_alive".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Thread(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                return Ok(py_bool(locked.handle.is_some()));
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'Thread' object has no attribute '{}'", name))),
                }
            }
            PyObject::Lock(inner_arc) => {
                let inner_arc = inner_arc.clone();
                match name {
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                while locked.lock.load(std::sync::atomic::Ordering::SeqCst) {
                                    std::thread::yield_now();
                                }
                                locked.lock.store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                locked.lock.store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "acquire".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                while locked.lock.load(std::sync::atomic::Ordering::SeqCst) {
                                    std::thread::yield_now();
                                }
                                locked.lock.store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "release".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                locked.lock.store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'lock' object has no attribute '{}'", name))),
                }
            }
            PyObject::RLock(inner_arc) => {
                let inner_arc = inner_arc.clone();
                match name {
                    "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "acquire".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if let Some(owner) = inner.owner {
                                    if owner == current_id {
                                        inner.count += 1;
                                        return Ok(py_bool(true));
                                    }
                                }
                                // Spin waiting for lock
                                while inner.owner.is_some() {
                                    drop(inner);
                                    std::thread::yield_now();
                                    inner = inner_arc.lock().unwrap();
                                }
                                inner.owner = Some(current_id);
                                inner.count = 1;
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "release".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if inner.owner != Some(current_id) {
                                    return Err(PyError::runtime_error("cannot release un-acquired lock"));
                                }
                                inner.count -= 1;
                                if inner.count == 0 {
                                    inner.owner = None;
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if let Some(owner) = inner.owner {
                                    if owner == current_id {
                                        inner.count += 1;
                                        return Ok(args[0].clone());
                                    }
                                }
                                while inner.owner.is_some() {
                                    drop(inner);
                                    std::thread::yield_now();
                                    inner = inner_arc.lock().unwrap();
                                }
                                inner.owner = Some(current_id);
                                inner.count = 1;
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if inner.owner != Some(current_id) {
                                    return Err(PyError::runtime_error("cannot release un-acquired lock"));
                                }
                                inner.count -= 1;
                                if inner.count == 0 {
                                    inner.owner = None;
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'RLock' object has no attribute '{}'", name))),
                }
            }
            PyObject::Event(inner_arc) => {
                let inner_arc = inner_arc.clone();
                match name {
                    "is_set" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "is_set".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let flag = inner_arc.flag.lock().unwrap();
                                return Ok(py_bool(*flag));
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "set" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "set".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let mut flag = inner_arc.flag.lock().unwrap();
                                *flag = true;
                                inner_arc.condvar.notify_all();
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let mut flag = inner_arc.flag.lock().unwrap();
                                *flag = false;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "wait" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "wait".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let mut flag = inner_arc.flag.lock().unwrap();
                                while !*flag {
                                    flag = inner_arc.condvar.wait(flag).unwrap();
                                }
                                return Ok(py_bool(true));
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'Event' object has no attribute '{}'", name))),
                }
            }
            PyObject::Queue(inner_arc) => {
                let inner_arc = inner_arc.clone();
                match name {
                    "put" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "put".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let item = args.get(1).cloned().ok_or_else(|| PyError::type_error("put() missing argument"))?;
                                let mut q = inner_arc.lock().unwrap();
                                q.queue.push_back(item);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let mut q = inner_arc.lock().unwrap();
                                return q.queue.pop_front().ok_or_else(|| PyError::runtime_error("empty queue"));
                            }
                            Err(PyError::runtime_error("not a Queue"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "qsize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "qsize".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let q = inner_arc.lock().unwrap();
                                return Ok(py_int(q.queue.len() as i64));
                            }
                            Ok(py_int(0))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'Queue' object has no attribute '{}'", name))),
                }
            }
            PyObject::Exception { typ, args, .. } => {
                match name {
                    "__name__" => Ok(py_str(typ)),
                    "args" => Ok(py_tuple(args.clone())),
                    _ => Err(PyError::attribute_error(format!("'Exception' object has no attribute '{}'", name))),
                }
            }
            PyObject::Int(_i) => {
                match name {
                    "__bool__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__bool__".to_string(),
                        func: |args| {
                            if let PyObject::Int(v) = &*args[0].borrow() {
                                Ok(py_bool(!v.is_zero()))
                            } else { Err(PyError::runtime_error("__bool__ on non-int")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__float__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__float__".to_string(),
                        func: |args| {
                            if let PyObject::Int(v) = &*args[0].borrow() {
                                Ok(py_float(v.to_f64().unwrap_or(0.0)))
                            } else { Err(PyError::runtime_error("__float__ on non-int")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "bit_length" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "bit_length".to_string(),
                        func: |args| {
                            if let PyObject::Int(v) = &*args[0].borrow() {
                                Ok(py_int(v.bits() as i64))
                            } else { Err(PyError::runtime_error("bit_length on non-int")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "to_bytes" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "to_bytes".to_string(),
                        func: |args| {
                            if args.len() < 3 { return Err(PyError::type_error("to_bytes() takes at least 2 arguments (1 given)")); }
                            if let PyObject::Int(val) = &*args[0].borrow() {
                                let length = args[1].as_i64().ok_or_else(|| PyError::type_error("length must be int"))?;
                                let byteorder = args[2].str();
                                let signed = if args.len() > 3 { args[3].truthy() } else { false };
                                if length <= 0 {
                                    return Err(PyError::type_error("length must be positive"));
                                }
                                let len = length as usize;
                                let (_, mut bytes) = if byteorder == "little" {
                                    val.to_bytes_le()
                                } else {
                                    val.to_bytes_be()
                                };
                                // Handle negative numbers for signed=True
                                if signed && val.sign() == num_bigint::Sign::Minus {
                                    // For signed negative, compute two's complement
                                    let abs_val = -val.clone();
                                    let (_, abs_bytes) = if byteorder == "little" {
                                        abs_val.to_bytes_le()
                                    } else {
                                        abs_val.to_bytes_be()
                                    };
                                    // Create two's complement
                                    let mut result = vec![0u8; len];
                                    for i in 0..abs_bytes.len().min(len) {
                                        result[if byteorder == "little" { i } else { len - 1 - i }] = abs_bytes[i];
                                    }
                                    // Two's complement: invert bits and add 1
                                    for b in result.iter_mut() {
                                        *b = !*b;
                                    }
                                    // Add 1
                                    let mut carry = 1u16;
                                    if byteorder == "little" {
                                        for b in result.iter_mut() {
                                            let v = *b as u16 + carry;
                                            *b = v as u8;
                                            carry = v >> 8;
                                        }
                                    } else {
                                        for b in result.iter_mut().rev() {
                                            let v = *b as u16 + carry;
                                            *b = v as u8;
                                            carry = v >> 8;
                                        }
                                    }
                                    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                                } else {
                                    // Pad or truncate to fit length
                                    if bytes.len() > len {
                                        return Err(PyError::type_error("int too big to convert"));
                                    }
                                    let mut result = vec![0u8; len];
                                    if byteorder == "little" {
                                        for i in 0..bytes.len() {
                                            result[i] = bytes[i];
                                        }
                                    } else {
                                        let offset = len - bytes.len();
                                        for i in 0..bytes.len() {
                                            result[offset + i] = bytes[i];
                                        }
                                    }
                                    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                                }
                            } else { Err(PyError::runtime_error("to_bytes on non-int")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'int' object has no attribute '{}'", name))),
                }
            }
            PyObject::Float(_f) => {
                match name {
                    "__int__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__int__".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                Ok(py_int(*v as i64))
                            } else { Err(PyError::runtime_error("__int__ on non-float")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "as_integer_ratio" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "as_integer_ratio".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                let f = *v;
                                if f.is_nan() || f.is_infinite() {
                                    return Err(PyError::value_error(format!("cannot convert {:?} to integer ratio", f)));
                                }
                                // Decompose f64 into a reduced fraction
                                fn float_to_ratio(x: f64) -> (BigInt, BigInt) {
                                    if x == 0.0 { return (BigInt::from(0), BigInt::from(1)); }
                                    let bits = x.to_bits();
                                    let sign = if (bits >> 63) == 0 { 1i64 } else { -1i64 };
                                    let biased_exp = ((bits >> 52) & 0x7ff) as i64;
                                    let mantissa = bits & 0x000f_ffff_ffff_ffff;
                                    if biased_exp == 0 {
                                        if mantissa == 0 { return (BigInt::from(0), BigInt::from(1)); }
                                        // Subnormal: value = mantissa * 2^(-1074)
                                        let num = BigInt::from(sign) * BigInt::from(mantissa);
                                        let den = BigInt::from(1i64) << 1074;
                                        let g = gcd_bigint(&num, &den);
                                        (num / &g, den / g)
                                    } else {
                                        // Normal: add implicit leading 1
                                        let full_mantissa = 0x0010_0000_0000_0000 | mantissa;
                                        let exp = biased_exp - 1023 - 52;
                                        if exp >= 0 {
                                            (BigInt::from(sign) * BigInt::from(full_mantissa) * (BigInt::from(1i64) << (exp as u32)), BigInt::from(1))
                                        } else {
                                            let num = BigInt::from(sign) * BigInt::from(full_mantissa);
                                            let den = BigInt::from(1i64) << ((-exp) as u32);
                                            let g = gcd_bigint(&num, &den);
                                            (num / &g, den / g)
                                        }
                                    }
                                }
                                fn gcd_bigint(a: &BigInt, b: &BigInt) -> BigInt {
                                    let mut a = a.clone();
                                    let mut b = b.clone();
                                    while !b.is_zero() {
                                        let t = b.clone();
                                        b = a % &t;
                                        a = t;
                                    }
                                    a.abs()
                                }
                                let (num, den) = float_to_ratio(f);
                                Ok(py_tuple(vec![py_int(num), py_int(den)]))
                            } else { Err(PyError::runtime_error("as_integer_ratio on non-float")) }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'float' object has no attribute '{}'", name))),
                }
            }
            PyObject::CompiledRegex { regex, pattern, flags } => {
                let re = regex.clone();
                let pat = pattern.clone();
                let fl = *flags;
                match name {
                    "pattern" => Ok(py_str(&pat)),
                    "flags" => Ok(py_int(fl as i64)),
                    "match" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        if args.len() < 1 {
                            return Err(PyError::type_error("match() takes at least 1 argument"));
                        }
                        let string = args[0].str();
                        match re.find_at(&string, 0) {
                            Some(m) if m.start() == 0 => {
                                Ok(PyObjectRef::new(PyObject::Tuple(vec![
                                    py_int(m.start() as i64),
                                    py_int(m.end() as i64),
                                    py_str(m.as_str()),
                                ])))
                            }
                            _ => Ok(py_none()),
                        }
                    })))),
                    "search" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        if args.len() < 1 {
                            return Err(PyError::type_error("search() takes at least 1 argument"));
                        }
                        let string = args[0].str();
                        match re.find(&string) {
                            Some(m) => {
                                Ok(PyObjectRef::new(PyObject::Tuple(vec![
                                    py_int(m.start() as i64),
                                    py_int(m.end() as i64),
                                    py_str(m.as_str()),
                                ])))
                            }
                            None => Ok(py_none()),
                        }
                    })))),
                    "findall" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        if args.len() < 1 {
                            return Err(PyError::type_error("findall() takes at least 1 argument"));
                        }
                        let string = args[0].str();
                        let results: Vec<PyObjectRef> = re.find_iter(&string)
                            .map(|m| py_str(m.as_str()))
                            .collect();
                        Ok(py_list(results))
                    })))),
                    "sub" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        if args.len() < 2 {
                            return Err(PyError::type_error("sub() takes at least 2 arguments"));
                        }
                        let repl = args[0].str();
                        let string = args[1].str();
                        let result = re.replace_all(&string, repl.as_str());
                        Ok(py_str(&result))
                    })))),
                    "split" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        if args.len() < 1 {
                            return Err(PyError::type_error("split() takes at least 1 argument"));
                        }
                        let string = args[0].str();
                        let limit = if args.len() > 1 { args[1].as_i64().unwrap_or(0) as usize } else { 0 };
                        let parts: Vec<PyObjectRef> = if limit > 0 {
                            re.splitn(&string, limit).map(|s| py_str(s)).collect()
                        } else {
                            re.split(&string).map(|s| py_str(s)).collect()
                        };
                        Ok(py_list(parts))
                    })))),
                    "fullmatch" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        if args.len() < 1 {
                            return Err(PyError::type_error("fullmatch() takes at least 1 argument"));
                        }
                        let string = args[0].str();
                        match re.find(&string) {
                            Some(m) if m.start() == 0 && m.end() == string.len() => {
                                Ok(PyObjectRef::new(PyObject::Tuple(vec![
                                    py_int(m.start() as i64),
                                    py_int(m.end() as i64),
                                    py_str(m.as_str()),
                                ])))
                            }
                            _ => Ok(py_none()),
                        }
                    })))),
                    _ => Err(PyError::attribute_error(format!("'re.Pattern' object has no attribute '{}'", name))),
                }
            }
            PyObject::Super { cls, obj } => {
                // super(cls, obj).attr: walk MRO of obj's type, starting after cls
                let obj_type = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    Some(typ.clone())
                } else {
                    obj.borrow().get_attribute("__class__").ok()
                };
                if let Some(obj_type) = obj_type {
                    if let PyObject::Type { mro, .. } = &*obj_type.borrow() {
                        // Find cls in MRO, start search from the next class
                        let start_idx = mro.iter().position(|m| {
                            if let (PyObjectRef::Mut(a), PyObjectRef::Mut(b)) = (cls, m) {
                                std::ptr::eq(a.as_ptr(), b.as_ptr())
                            } else {
                                false
                            }
                        }).unwrap_or(0) + 1;
                        if start_idx < mro.len() {
                            let mut found = None;
                            for base in mro.iter().skip(start_idx) {
                                if let PyObject::Type { dict, .. } = &*base.borrow() {
                                    if let Some(val) = dict.get_str(&name) {
                                        let val_borrowed = val.borrow();
                                        match &*val_borrowed {
                                            PyObject::Function { .. } | PyObject::BuiltinFunction { .. } => {
                                                found = Some(PyObjectRef::new(PyObject::BoundMethod {
                                                    func: val.clone(),
                                                    self_obj: obj.clone(),
                                                }));
                                                break;
                                            }
                                            PyObject::Property { getter: Some(g), .. } => {
                                                found = Some(builtin_call(g, &[obj.clone()]).unwrap_or_else(|_| val.clone()));
                                                break;
                                            }
                                            _ => {
                                                found = Some(val.clone());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(found) = found {
                                return Ok(found);
                            }
                        }
                    }
                }
                Err(PyError::attribute_error(
                    format!("'super' object has no attribute '{}'", name)
                ))
            }
            PyObject::FutureAwaitIterator { future, yielded } => {
                match name {
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__next__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__next__".to_string(),
                        func: |args| {
                            if args.is_empty() { return Err(PyError::type_error("__next__ needs self")); }
                            let self_ref = args[0].borrow();
                            let (done, result) = match &*self_ref {
                                PyObject::FutureAwaitIterator { future, yielded } => {
                                    if *yielded {
                                        let done = future.borrow().get_attribute("_done")
                                            .ok().map(|d| d.truthy()).unwrap_or(false);
                                        let result = future.borrow().get_attribute("_result")
                                            .unwrap_or_else(|_| py_none());
                                        (Some(done), Some(result))
                                    } else {
                                        let f = future.clone();
                                        drop(self_ref);
                                        return Ok(f);
                                    }
                                }
                                _ => return Err(PyError::runtime_error("__next__ on non-FutureAwaitIterator")),
                            };
                            drop(self_ref);
                            if let Some(true) = done {
                                Err(PyError::Exception("StopIteration".to_string(), result.unwrap_or_else(|| py_none())))
                            } else {
                                Ok(py_none())
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send".to_string(),
                        func: |args| {
                            if args.is_empty() { return Err(PyError::type_error("send needs self")); }
                            let (is_first, future_clone) = match &*args[0].borrow() {
                                PyObject::FutureAwaitIterator { future, yielded } => {
                                    (!*yielded, future.clone())
                                }
                                _ => return Err(PyError::runtime_error("send on non-FutureAwaitIterator")),
                            };
                            if is_first {
                                let mut obj = args[0].borrow_mut();
                                if let PyObject::FutureAwaitIterator { yielded, .. } = &mut *obj {
                                    *yielded = true;
                                }
                                drop(obj);
                                // Return the future as the yielded value
                                Ok(future_clone)
                            } else {
                                // Second send: check if future is done
                                let done = future_clone.borrow().get_attribute("_done")
                                    .ok().map(|d| d.truthy()).unwrap_or(false);
                                let result = future_clone.borrow().get_attribute("_result")
                                    .unwrap_or_else(|_| py_none());
                                if done {
                                    Err(PyError::Exception("StopIteration".to_string(), result))
                                } else {
                                    Ok(future_clone)
                                }
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'future_await_iterator' object has no attribute '{}'", name))),
                }
            }
            PyObject::BuiltinFunction { name: bf_name, .. } => {
                if bf_name == "bytes" && name == "fromhex" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "fromhex".to_string(),
                        func: builtin_bytes_fromhex,
                    }));
                }
                if bf_name == "int" && name == "from_bytes" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "from_bytes".to_string(),
                        func: builtin_int_from_bytes,
                    }));
                }
                if bf_name == "dict" && (name == "__setitem__" || name == "__getitem__") {
                    let method_name = name.to_string();
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: method_name.clone(),
                        func: if method_name == "__setitem__" {
                            builtin_dict_setitem as BuiltinFunc
                        } else {
                            builtin_dict_getitem as BuiltinFunc
                        },
                        self_obj: py_none(),
                    }));
                }
                Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", self.type_name(), name)))
            }
            PyObject::FrozenSet(items) | PyObject::Set(items) => {
                match name {
                    "issuperset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issuperset".to_string(),
                        func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            let self_set = if let PyObject::FrozenSet(s) = &*args[0].borrow() { s.clone() }
                                else if let PyObject::Set(s) = &*args[0].borrow() { s.clone() }
                                else { return Err(PyError::type_error("issuperset requires a set/frozenset")) };
                            let other = if args.len() < 2 { return Err(PyError::type_error("issuperset requires 1 argument")) }
                                else { &args[1] };
                            let other_set = convert_to_set(other)?;
                            Ok(py_bool(self_set.is_superset(&other_set)))
                        },
                        self_obj: py_none(),
                    })),
                    "issubset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issubset".to_string(),
                        func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            let self_set = if let PyObject::FrozenSet(s) = &*args[0].borrow() { s.clone() }
                                else if let PyObject::Set(s) = &*args[0].borrow() { s.clone() }
                                else { return Err(PyError::type_error("issubset requires a set/frozenset")) };
                            let other = if args.len() < 2 { return Err(PyError::type_error("issubset requires 1 argument")) }
                                else { &args[1] };
                            let other_set = convert_to_set(other)?;
                            Ok(py_bool(self_set.is_subset(&other_set)))
                        },
                        self_obj: py_none(),
                    })),
                    _ => Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", self.type_name(), name))),
                }
            }
            PyObject::Code(c) => {
                match name {
                    "co_filename" => Ok(py_str(&c.filename)),
                    "co_name" => Ok(py_str(&c.name)),
                    "co_argcount" => Ok(py_int(c.arg_count as i64)),
                    "co_nlocals" => Ok(py_int(c.nlocals as i64)),
                    "co_varnames" => Ok(py_tuple(c.varnames.iter().map(|v| py_str(v)).collect())),
                    "co_flags" => Ok(py_int(c.flags as i64)),
                    _ => Err(PyError::attribute_error(format!("'code' object has no attribute '{}'", name))),
                }
            }
            _ => Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", self.type_name(), name))),
        }
    }

    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, typ } => {
                // Check __slots__ restriction if defined on the type or its MRO
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        let type_name = get_type_name_for_instance(typ);
                        return Err(PyError::attribute_error(
                            format!("'{}' object has no attribute '{}'", type_name, name)
                        ));
                    }
                }
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Module { dict, .. } => {
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Type { dict, .. } => {
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Function { dict, .. } => {
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Dict(_) | PyObject::List(_) | PyObject::Tuple(_) | PyObject::Set(_) | PyObject::FrozenSet(_) => {
                // Store attributes in a side dict (instance-like) for these built-in types
                let mut pd = match self {
                    PyObject::Dict(d) => Some(d.clone()),
                    _ => None,
                };
                Err(PyError::attribute_error(format!("cannot set attribute '{}' on '{}'", name, self.type_name())))
            }
            _ => Err(PyError::attribute_error(format!("cannot set attribute '{}' on '{}'", name, self.type_name()))),
        }
    }

    fn del_attribute(&mut self, name: &str) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, typ } => {
                // Check __slots__ restriction if defined on the type or its MRO
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        let type_name = get_type_name_for_instance(typ);
                        return Err(PyError::attribute_error(
                            format!("'{}' object has no attribute '{}'", type_name, name)
                        ));
                    }
                }
                dict.remove(name).ok_or_else(|| PyError::attribute_error(format!(
                    "'{}' object has no attribute '{}'", self.type_name(), name
                )))?;
                Ok(())
            }
            PyObject::Module { dict, .. } => {
                dict.remove(name).ok_or_else(|| PyError::attribute_error(format!("module has no attribute '{}'", name)))?;
                Ok(())
            }
            PyObject::Type { dict, .. } => {
                dict.remove(name).ok_or_else(|| PyError::attribute_error(format!("type has no attribute '{}'", name)))?;
                Ok(())
            }
            _ => Err(PyError::attribute_error(format!(
                "'{}' object has no attribute '{}'", self.type_name(), name
            ))),
        }
    }
}

// ---- Subscript access ----

pub fn to_index(obj: &PyObjectRef) -> PyResult<BigInt> {
    let type_name = obj.get_type_name();
    let is_instance = matches!(&*obj.borrow(), PyObject::Instance { .. });
    if is_instance {
        let f = {
            let o = obj.borrow();
            match &*o {
                PyObject::Instance { typ, .. } => {
                    let typ_ref = typ.borrow();
                    match &*typ_ref {
                        PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__index__").cloned(),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        if let Some(f) = f {
            let result = call_bound_method(f, obj.clone(), vec![])?;
            let r = result.borrow();
            if let PyObject::Int(i) = &*r { Ok(i.clone()) }
            else { Err(PyError::type_error("__index__ must return int")) }
        } else {
            Err(PyError::type_error(format!("'{}' object cannot be interpreted as an integer", type_name)))
        }
    } else {
        let o = obj.borrow();
        match &*o {
            PyObject::Int(i) => Ok(i.clone()),
            _ => Err(PyError::type_error(format!("'{}' object cannot be interpreted as an integer", type_name))),
        }
    }
}

pub fn py_getitem(obj: &PyObjectRef, index: &PyObjectRef) -> PyResult<PyObjectRef> {
    // Check for __getitem__ on custom classes and __class_getitem__ on types (PEP 560)
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Type { dict: type_dict, mro, .. } => {
                // PEP 560: cls[args] checks for __class_getitem__ on the type and its MRO
                type_dict.get_str("__class_getitem__").cloned().or_else(|| {
                    for base in mro.iter().skip(1) {
                        if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                            if let Some(val) = base_dict.get_str("__class_getitem__") {
                                return Some(val.clone());
                            }
                        }
                    }
                    None
                })
            }
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, mro, .. } => {
                        type_dict.get_str("__getitem__").cloned().or_else(|| {
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if let Some(val) = base_dict.get_str("__getitem__") {
                                        return Some(val.clone());
                                    }
                                }
                            }
                            // Fallback: use builtin_dict_getitem for dict-derived instances
                            None
                        }).or_else(|| {
                            // Check if any base is the builtin 'dict' function
                            let is_dict_subclass = mro.iter().skip(1).any(|base| {
                                let b = base.borrow();
                                matches!(&*b, PyObject::BuiltinFunction { name, .. } if name == "dict")
                                    || b.type_name() == "dict"
                            });
                            if is_dict_subclass {
                                Some(PyObjectRef::new(PyObject::BuiltinFunction {
                                    name: "__getitem__".to_string(),
                                    func: builtin_dict_getitem,
                                }))
                            } else {
                                None
                            }
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, obj.clone(), vec![index.clone()]);
    }
    let o = obj.borrow();
    match &*o {
        PyObject::List(items) => {
            let idx = index.borrow();
            match &*idx {
                PyObject::Int(i) => {
                    let i = i.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                    let len = items.len() as isize;
                    let i = if i < 0 { len + i } else { i };
                    if i < 0 || i >= len {
                        return Err(PyError::index_error("list index out of range"));
                    }
                    Ok(items[i as usize].clone())
                }
                PyObject::Slice { start, stop, step } => {
                    let mut result = Vec::new();
                    let len = items.len();
                    let s = start.borrow();
                    let e = stop.borrow();
                    let st = step.borrow();
                    let step_val = if let PyObject::Int(i) = &*st { i.to_isize().unwrap_or(1) } else { 1 };
                    if step_val > 0 {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or(0) } else { 0 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(len as isize) } else { len as isize };
                        let mut i = start_val;
                        while i < stop_val && i < len as isize {
                            result.push(items[i as usize].clone());
                            i += step_val;
                        }
                    } else {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or((len as isize) - 1) } else { (len as isize) - 1 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(-1) } else { -1 };
                        let mut i = start_val;
                        while i > stop_val && i >= 0 {
                            result.push(items[i as usize].clone());
                            i += step_val;
                        }
                    }
                    Ok(py_list(result))
                }
                _ => Err(PyError::type_error("list indices must be integers or slices")),
            }
        }
        PyObject::Tuple(items) => {
            let idx = index.borrow();
            match &*idx {
                PyObject::Int(i) => {
                    let i = i.to_isize().ok_or_else(|| PyError::index_error("tuple index out of range"))?;
                    let len = items.len() as isize;
                    let i = if i < 0 { len + i } else { i };
                    if i < 0 || i >= len {
                        return Err(PyError::index_error("tuple index out of range"));
                    }
                    Ok(items[i as usize].clone())
                }
                PyObject::Slice { start, stop, step } => {
                    let mut result = Vec::new();
                    let len = items.len();
                    let s = start.borrow();
                    let e = stop.borrow();
                    let st = step.borrow();
                    let step_val = if let PyObject::Int(i) = &*st { i.to_isize().unwrap_or(1) } else { 1 };
                    if step_val > 0 {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or(0) } else { 0 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(len as isize) } else { len as isize };
                        let mut i = start_val;
                        while i < stop_val && i < len as isize {
                            result.push(items[i as usize].clone());
                            i += step_val;
                        }
                    } else {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or((len as isize) - 1) } else { (len as isize) - 1 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(-1) } else { -1 };
                        let mut i = start_val;
                        while i > stop_val && i >= 0 {
                            result.push(items[i as usize].clone());
                            i += step_val;
                        }
                    }
                    Ok(py_tuple(result))
                }
                _ => {
                    Err(PyError::type_error("tuple indices must be integers or slices"))
                }
            }
        }
        PyObject::Str(s) => {
            let idx = index.borrow();
            match &*idx {
                PyObject::Int(i) => {
                    let chars: Vec<char> = s.chars().collect();
                    let i = i.to_isize().ok_or_else(|| PyError::index_error("string index out of range"))?;
                    let len = chars.len() as isize;
                    let i = if i < 0 { len + i } else { i };
                    if i < 0 || i >= len {
                        return Err(PyError::index_error("string index out of range"));
                    }
                    Ok(py_str(&chars[i as usize].to_string()))
                }
                PyObject::Slice { start, stop, step } => {
                    let chars: Vec<char> = s.chars().collect();
                    let len = chars.len();
                    let s = start.borrow();
                    let e = stop.borrow();
                    let st = step.borrow();
                    let step_val = if let PyObject::Int(i) = &*st { i.to_isize().unwrap_or(1) } else { 1 };
                    let mut result = String::new();
                    if step_val > 0 {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or(0) } else { 0 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(len as isize) } else { len as isize };
                        let mut i = start_val;
                        while i < stop_val && i < len as isize {
                            result.push(chars[i as usize]);
                            i += step_val;
                        }
                    } else {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or((len as isize) - 1) } else { (len as isize) - 1 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(-1) } else { -1 };
                        let mut i = start_val;
                        while i > stop_val && i >= 0 {
                            result.push(chars[i as usize]);
                            i += step_val;
                        }
                    }
                    Ok(py_str(&result))
                }
                _ => Err(PyError::type_error("string indices must be integers or slices")),
            }
        }
        PyObject::Dict(d) => {
            match d.get(index)? {
                Some(val) => Ok(val),
                None => Err(PyError::key_error(index.str())),
            }
        }
        PyObject::Bytes(b) => {
            let idx = index.borrow();
            match &*idx {
                PyObject::Int(i) => {
                    let i = i.to_isize().ok_or_else(|| PyError::index_error("bytes index out of range"))?;
                    let len = b.len() as isize;
                    let i = if i < 0 { len + i } else { i };
                    if i < 0 || i >= len {
                        return Err(PyError::index_error("bytes index out of range"));
                    }
                    Ok(PyObjectRef::imm(PyObject::Bytes(vec![b[i as usize]])))
                }
                PyObject::Slice { start, stop, step } => {
                    let len = b.len();
                    let s = start.borrow();
                    let e = stop.borrow();
                    let st = step.borrow();
                    let step_val = if let PyObject::Int(i) = &*st { i.to_isize().unwrap_or(1) } else { 1 };
                    let mut result = Vec::new();
                    if step_val > 0 {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or(0) } else { 0 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(len as isize) } else { len as isize };
                        let mut i = start_val;
                        while i < stop_val && i < len as isize {
                            result.push(b[i as usize]);
                            i += step_val;
                        }
                    } else {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or((len as isize) - 1) } else { (len as isize) - 1 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(-1) } else { -1 };
                        let mut i = start_val;
                        while i > stop_val && i >= 0 {
                            result.push(b[i as usize]);
                            i += step_val;
                        }
                    }
                    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                }
                _ => Err(PyError::type_error("bytes indices must be integers or slices")),
            }
        }
        PyObject::ByteArray(b) => {
            let idx = index.borrow();
            match &*idx {
                PyObject::Int(i) => {
                    let i = i.to_isize().ok_or_else(|| PyError::index_error("bytearray index out of range"))?;
                    let len = b.len() as isize;
                    let i = if i < 0 { len + i } else { i };
                    if i < 0 || i >= len {
                        return Err(PyError::index_error("bytearray index out of range"));
                    }
                    Ok(PyObjectRef::new(PyObject::ByteArray(vec![b[i as usize]])))
                }
                PyObject::Slice { start, stop, step } => {
                    let len = b.len();
                    let s = start.borrow();
                    let e = stop.borrow();
                    let st = step.borrow();
                    let step_val = if let PyObject::Int(i) = &*st { i.to_isize().unwrap_or(1) } else { 1 };
                    let mut result = Vec::new();
                    if step_val > 0 {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or(0) } else { 0 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(len as isize) } else { len as isize };
                        let mut i = start_val;
                        while i < stop_val && i < len as isize {
                            result.push(b[i as usize]);
                            i += step_val;
                        }
                    } else {
                        let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or((len as isize) - 1) } else { (len as isize) - 1 };
                        let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(-1) } else { -1 };
                        let mut i = start_val;
                        while i > stop_val && i >= 0 {
                            result.push(b[i as usize]);
                            i += step_val;
                        }
                    }
                    Ok(PyObjectRef::new(PyObject::ByteArray(result)))
                }
                _ => Err(PyError::type_error("bytearray indices must be integers or slices")),
            }
        }
        PyObject::Array(arr) => {
            let idx = index.borrow();
            if let PyObject::Int(i) = &*idx {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("array index out of range"))?;
                let len = arr.data.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("array index out of range"));
                }
                let v = arr.data[i as usize];
                if arr.typecode == 'i' {
                    Ok(py_int(v as i64))
                } else {
                    Ok(py_float(v))
                }
            } else {
                Err(PyError::type_error("array indices must be integers"))
            }
        }
        PyObject::Instance { dict, .. } => {
            let key = index.str();
            let val = dict.get(&key).cloned();
            drop(o);
            if let Some(v) = val {
                Ok(v)
            } else {
                // Check for __missing__ (dict subclass support, e.g. Counter)
                let missing = obj.borrow().get_attribute("__missing__").ok()
                    .and_then(|m| crate::object::call_function(&m, vec![obj.clone(), index.clone()]).ok());
                match missing {
                    Some(v) => Ok(v),
                    None => Err(PyError::key_error(index.str())),
                }
            }
        }
        _ => Err(PyError::type_error(format!("'{}' object is not subscriptable", o.type_name()))),
    }
}

pub fn py_setitem(obj: &PyObjectRef, index: &PyObjectRef, value: PyObjectRef) -> PyResult<()> {
    // Check for __setitem__ on custom classes
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, mro, .. } => {
                        type_dict.get_str("__setitem__").cloned().or_else(|| {
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if let Some(val) = base_dict.get_str("__setitem__") {
                                        return Some(val.clone());
                                    }
                                }
                            }
                            None
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        call_bound_method(f, obj.clone(), vec![index.clone(), value])?;
        return Ok(());
    }
    // Default Instance __setitem__: store key/value in the instance dict (HashMap)
    {
        let o = obj.borrow();
        if let PyObject::Instance { dict, .. } = &*o {
            let key = index.str();
            drop(o);
            let mut o = obj.borrow_mut();
            if let PyObject::Instance { dict, .. } = &mut *o {
                dict.insert(key, value);
                return Ok(());
            }
        }
    }
    let mut o = obj.borrow_mut();
    match &mut *o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let PyObject::Int(i) = &*idx {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("list assignment index out of range"));
                }
                items[i as usize] = value;
                Ok(())
            } else {
                Err(PyError::type_error("list indices must be integers"))
            }
        }
        PyObject::Dict(d) => {
            d.set(index.clone(), value)
        }
        _ => Err(PyError::type_error(format!("'{}' object does not support item assignment", o.type_name()))),
    }
}

pub fn py_delitem(obj: &PyObjectRef, index: &PyObjectRef) -> PyResult<()> {
    // Check for __delitem__ on custom classes
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get_str("__delitem__").cloned(),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        call_bound_method(f, obj.clone(), vec![index.clone()])?;
        return Ok(());
    }
    let mut o = obj.borrow_mut();
    match &mut *o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let PyObject::Int(i) = &*idx {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("list index out of range"));
                }
                items.remove(i as usize);
                Ok(())
            } else {
                Err(PyError::type_error("list indices must be integers"))
            }
        }
        PyObject::Dict(d) => {
            d.remove(index)?;
            Ok(())
        }
        _ => Err(PyError::type_error(format!("'{}' object does not support item deletion", o.type_name()))),
    }
}

// ---- String operations ----

pub fn py_str_join(strings: &[PyObjectRef], sep: &str) -> PyResult<PyObjectRef> {
    let parts: Vec<String> = strings.iter().map(|s| s.str()).collect();
    Ok(py_str(&parts.join(sep)))
}

pub fn py_str_split(s: &str, sep: Option<&str>) -> PyResult<PyObjectRef> {
    let parts: Vec<PyObjectRef> = if let Some(sep) = sep {
        s.split(sep).map(|p| py_str(p)).collect()
    } else {
        s.split_whitespace().map(|p| py_str(p)).collect()
    };
    Ok(py_list(parts))
}

// ---- Exception constructor functions ----

macro_rules! make_exception_func {
    ($name:ident, $typ:expr) => {
        pub fn $name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            Ok(PyObjectRef::new(PyObject::Exception { typ: $typ.to_string(), args: args.to_vec(), cause: None }))
        }
    };
}

make_exception_func!(builtin_make_exception_baseexception, "BaseException");
make_exception_func!(builtin_make_exception_exception, "Exception");
make_exception_func!(builtin_make_exception_typeerror, "TypeError");
make_exception_func!(builtin_make_exception_valueerror, "ValueError");
make_exception_func!(builtin_make_exception_zerodivisionerror, "ZeroDivisionError");
make_exception_func!(builtin_make_exception_nameerror, "NameError");
make_exception_func!(builtin_make_exception_attributeerror, "AttributeError");
make_exception_func!(builtin_make_exception_indexerror, "IndexError");
make_exception_func!(builtin_make_exception_keyerror, "KeyError");
make_exception_func!(builtin_make_exception_runtimeerror, "RuntimeError");
make_exception_func!(builtin_make_exception_stopiteration, "StopIteration");
make_exception_func!(builtin_make_exception_assertionerror, "AssertionError");
make_exception_func!(builtin_make_exception_oserror, "OSError");
make_exception_func!(builtin_make_exception_importerror, "ImportError");
// Additional exception types for full CPython hierarchy
make_exception_func!(builtin_make_exception_lookuperror, "LookupError");
make_exception_func!(builtin_make_exception_arithmeticerror, "ArithmeticError");
make_exception_func!(builtin_make_exception_floatingpointerror, "FloatingPointError");
make_exception_func!(builtin_make_exception_overflowerror, "OverflowError");
make_exception_func!(builtin_make_exception_environmenterror, "EnvironmentError");
make_exception_func!(builtin_make_exception_ioerror, "IOError");
make_exception_func!(builtin_make_exception_filenotfounderror, "FileNotFoundError");
make_exception_func!(builtin_make_exception_permissionerror, "PermissionError");
make_exception_func!(builtin_make_exception_notimplementederror, "NotImplementedError");
make_exception_func!(builtin_make_exception_recursionerror, "RecursionError");
make_exception_func!(builtin_make_exception_keyboardinterrupt, "KeyboardInterrupt");
make_exception_func!(builtin_make_exception_generatorexit, "GeneratorExit");
make_exception_func!(builtin_make_exception_systemexit, "SystemExit");
make_exception_func!(builtin_make_exception_modulenotfounderror, "ModuleNotFoundError");
make_exception_func!(builtin_make_exception_stopasynciteration, "StopAsyncIteration");
make_exception_func!(builtin_make_exception_eoferror, "EOFError");
make_exception_func!(builtin_make_exception_syntaxerror, "SyntaxError");
make_exception_func!(builtin_make_exception_connectionerror, "ConnectionError");
make_exception_func!(builtin_make_exception_brokenpipeerror, "BrokenPipeError");
make_exception_func!(builtin_make_exception_connectionrefusederror, "ConnectionRefusedError");
make_exception_func!(builtin_make_exception_blockingioerror, "BlockingIOError");
make_exception_func!(builtin_make_exception_childprocesserror, "ChildProcessError");
make_exception_func!(builtin_make_exception_interruptederror, "InterruptedError");
make_exception_func!(builtin_make_exception_timeouterror, "TimeoutError");
make_exception_func!(builtin_make_exception_unicodedecodeerror, "UnicodeDecodeError");
make_exception_func!(builtin_make_exception_unicodeencodeerror, "UnicodeEncodeError");
make_exception_func!(builtin_make_exception_systemerror, "SystemError");

// ExceptionGroup and BaseExceptionGroup factory functions (PEP 654)
pub fn builtin_make_exception_exceptiongroup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let message = if !args.is_empty() { args[0].str() } else { "".to_string() };
    let exceptions = if args.len() > 1 {
        match &*args[1].borrow() {
            PyObject::List(items) => items.clone(),
            PyObject::Tuple(items) => items.clone(),
            _ => vec![],
        }
    } else {
        vec![]
    };
    Ok(PyObjectRef::new(PyObject::ExceptionGroup {
        typ: "ExceptionGroup".to_string(),
        args: args.to_vec(),
        exceptions,
    }))
}

pub fn builtin_make_exception_baseexceptiongroup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let message = if !args.is_empty() { args[0].str() } else { "".to_string() };
    let exceptions = if args.len() > 1 {
        match &*args[1].borrow() {
            PyObject::List(items) => items.clone(),
            PyObject::Tuple(items) => items.clone(),
            _ => vec![],
        }
    } else {
        vec![]
    };
    Ok(PyObjectRef::new(PyObject::ExceptionGroup {
        typ: "BaseExceptionGroup".to_string(),
        args: args.to_vec(),
        exceptions,
    }))
}

fn json_encode(val: &PyObjectRef) -> PyResult<PyObjectRef> {
    match &*val.borrow() {
        PyObject::None => Ok(py_str("null")),
        PyObject::Bool(b) => Ok(py_str(if *b { "true" } else { "false" })),
        PyObject::Int(i) => Ok(py_str(&i.to_string())),
        PyObject::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                return Err(PyError::ValueError("Out of range float values are not JSON compliant".to_string()));
            }
            Ok(py_str(&f.to_string()))
        }
        PyObject::Str(s) => Ok(py_str(&json_escape_string(s))),
        PyObject::List(items) | PyObject::Tuple(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                let encoded = json_encode(item)?;
                parts.push(encoded.str());
            }
            Ok(py_str(&format!("[{}]", parts.join(", "))))
        }
        PyObject::Dict(d) => {
            let mut pairs = Vec::new();
            for (key, val) in d.items() {
                let k = json_encode(&key)?;
                let v = json_encode(&val)?;
                pairs.push(format!("{}: {}", k.str(), v.str()));
            }
            Ok(py_str(&format!("{{{}}}", pairs.join(", "))))
        }
        _ => Err(PyError::type_error(format!("Object of type '{}' is not JSON serializable", val.borrow().type_name()))),
    }
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn json_decode(s: &str) -> PyResult<PyObjectRef> {
    let s = s.trim();
    let mut chars = s.chars().peekable();
    json_parse_value(&mut chars)
}

fn json_parse_value<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    json_skip_ws(chars);
    match chars.peek() {
        None => Err(PyError::ValueError("Unexpected end of JSON input".to_string())),
        Some('"') => json_parse_string(chars),
        Some('t') | Some('f') => json_parse_bool(chars),
        Some('n') => json_parse_null(chars),
        Some('[') => json_parse_array(chars),
        Some('{') => json_parse_object(chars),
        Some(c) if c.is_ascii_digit() || *c == '-' => json_parse_number(chars),
        Some(c) => Err(PyError::ValueError(format!("Unexpected character '{}'", c))),
    }
}

fn json_skip_ws<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) {
    while let Some(&c) = chars.peek() {
        if c.is_ascii_whitespace() { chars.next(); }
        else { break; }
    }
}

fn json_parse_string<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    chars.next();
    let mut s = String::new();
    loop {
        match chars.next() {
            None => return Err(PyError::ValueError("Unterminated JSON string".to_string())),
            Some('"') => return Ok(py_str(&s)),
            Some('\\') => {
                match chars.next() {
                    None => return Err(PyError::ValueError("Unexpected end of JSON string".to_string())),
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('n') => s.push('\n'),
                    Some('r') => s.push('\r'),
                    Some('t') => s.push('\t'),
                    Some('b') => s.push('\x08'),
                    Some('f') => s.push('\x0c'),
                    Some('u') => {
                        let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                        if hex.len() < 4 { return Err(PyError::ValueError("Invalid Unicode escape".to_string())); }
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(code) {
                                s.push(c);
                            }
                        }
                    }
                    Some(c) => s.push(c),
                }
            }
            Some(c) => s.push(c),
        }
    }
}

fn json_parse_bool<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    let s: String = chars.by_ref().take(5).collect();
    if s.starts_with("true") { Ok(py_bool(true)) }
    else if s.starts_with("false") { Ok(py_bool(false)) }
    else { Err(PyError::ValueError(format!("Unexpected token '{}'", s))) }
}

fn json_parse_null<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    let s: String = chars.by_ref().take(4).collect();
    if s.starts_with("null") { Ok(py_none()) }
    else { Err(PyError::ValueError(format!("Unexpected token '{}'", s))) }
}

fn json_parse_number<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    let mut num = String::new();
    if let Some(&'-') = chars.peek() { num.push(chars.next().unwrap()); }
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() { num.push(chars.next().unwrap()); }
        else { break; }
    }
    if let Some(&'.') = chars.peek() {
        num.push(chars.next().unwrap());
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() { num.push(chars.next().unwrap()); }
            else { break; }
        }
        let peek_lower = chars.peek().map(|c| c.to_ascii_lowercase());
        if peek_lower == Some('e') {
            num.push(chars.next().unwrap());
            if let Some(&'+') | Some(&'-') = chars.peek() { num.push(chars.next().unwrap()); }
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() { num.push(chars.next().unwrap()); }
                else { break; }
            }
        }
        Ok(py_float(num.parse::<f64>().map_err(|_| PyError::ValueError(format!("Invalid number: {}", num)))?))
    } else {
        Ok(py_int(num.parse::<i64>().map_err(|_| PyError::ValueError(format!("Invalid integer: {}", num)))?))
    }
}

fn json_parse_array<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    chars.next();
    let mut items = Vec::new();
    loop {
        json_skip_ws(chars);
        match chars.peek() {
            None => return Err(PyError::ValueError("Unterminated JSON array".to_string())),
            Some(&']') => { chars.next(); return Ok(py_list(items)); }
            Some(&',') => { chars.next(); continue; }
            _ => { items.push(json_parse_value(chars)?); }
        }
    }
}

fn json_parse_object<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> PyResult<PyObjectRef> {
    chars.next();
    let dict = py_dict();
    loop {
        json_skip_ws(chars);
        match chars.peek() {
            None => return Err(PyError::ValueError("Unterminated JSON object".to_string())),
            Some(&'}') => { chars.next(); return Ok(dict); }
            Some(&',') => { chars.next(); continue; }
            Some(&'"') => {
                let key = json_parse_string(chars)?;
                json_skip_ws(chars);
                match chars.next() {
                    Some(':') => {}
                    Some(c) => return Err(PyError::ValueError(format!("Expected ':' got '{}'", c))),
                    None => return Err(PyError::ValueError("Unexpected end of JSON object".to_string())),
                }
                let val = json_parse_value(chars)?;
                if let PyObject::Dict(d) = &mut *dict.borrow_mut() {
                    d.set(key, val)?;
                }
            }
            Some(c) => return Err(PyError::ValueError(format!("Unexpected token '{}' in object", c))),
        }
    }
}

pub fn json_encode_full(val: &PyObjectRef, indent: Option<usize>, sort_keys: bool, level: usize) -> PyResult<PyObjectRef> {
    match &*val.borrow() {
        PyObject::None => Ok(py_str("null")),
        PyObject::Bool(b) => Ok(py_str(if *b { "true" } else { "false" })),
        PyObject::Int(i) => Ok(py_str(&i.to_string())),
        PyObject::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                return Err(PyError::ValueError("Out of range float values are not JSON compliant".to_string()));
            }
            Ok(py_str(&f.to_string()))
        }
        PyObject::Str(s) => Ok(py_str(&json_escape_string(s))),
        PyObject::List(items) | PyObject::Tuple(items) => {
            if indent.is_some() {
                let inner_indent = indent.unwrap_or(4);
                let pad = " ".repeat(inner_indent * (level + 1));
                let close_pad = " ".repeat(inner_indent * level);
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    let encoded = json_encode_full(item, indent, sort_keys, level + 1)?;
                    parts.push(format!("\n{}{}", pad, encoded.str()));
                }
                if parts.is_empty() {
                    Ok(py_str("[]"))
                } else {
                    parts.push(format!("\n{}", close_pad));
                    Ok(py_str(&format!("[{}]", parts.join(","))))
                }
            } else {
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    let encoded = json_encode_full(item, indent, sort_keys, level + 1)?;
                    parts.push(encoded.str());
                }
                Ok(py_str(&format!("[{}]", parts.join(", "))))
            }
        }
        PyObject::Dict(d) => {
            let pairs: Vec<(String, String)> = if sort_keys {
                let mut sorted: Vec<(String, String)> = d.items().iter()
                    .map(|(k, v)| {
                        let key_obj = json_encode_full(k, indent, sort_keys, level + 1)
                            .unwrap_or_else(|_| py_str("\"?\""));
                        let val_obj = json_encode_full(v, indent, sort_keys, level + 1)
                            .unwrap_or_else(|_| py_str("null"));
                        (k.str(), format!("{}: {}", key_obj.str(), val_obj.str()))
                    })
                    .collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                sorted
            } else {
                d.items().iter().map(|(k, v)| {
                    let key_obj = json_encode_full(k, indent, sort_keys, level + 1)
                        .unwrap_or_else(|_| py_str("\"?\""));
                    let val_obj = json_encode_full(v, indent, sort_keys, level + 1)
                        .unwrap_or_else(|_| py_str("null"));
                    (String::new(), format!("{}: {}", key_obj.str(), val_obj.str()))
                }).collect()
            };
            if indent.is_some() {
                let inner_indent = indent.unwrap_or(4);
                let pad = " ".repeat(inner_indent * (level + 1));
                let close_pad = " ".repeat(inner_indent * level);
                let items: Vec<String> = pairs.iter().map(|(_, v)| format!("\n{}{}", pad, v)).collect();
                if items.is_empty() {
                    Ok(py_str("{}"))
                } else {
                    Ok(py_str(&format!("{{{},{}\n{}}}", items.join(","), "", close_pad)))
                }
            } else {
                let items: Vec<String> = pairs.iter().map(|(_, v)| v.clone()).collect();
                Ok(py_str(&format!("{{{}}}", items.join(", "))))
            }
        }
        _ => Err(PyError::type_error(format!("Object of type '{}' is not JSON serializable", val.borrow().type_name()))),
    }
}

pub fn call_function(func: &PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
    let f = func.borrow();
    match &*f {
        PyObject::BuiltinFunction { func: bf, .. } => {
            return bf(&args);
        }
        PyObject::Closure(func) => {
            return func(&args);
        }
        _ => {}
    }
    drop(f);
    Err(PyError::type_error(format!("'{}' object is not callable", func.borrow().type_name())))
}

static RNG_STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn fast_random_u64() -> u64 {
    RNG_STATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

pub fn fast_random_f64() -> f64 {
    (fast_random_u64() >> 11) as f64 * (1.0 / 9007199254740992.0)
}

fn socket_addr_to_string(addr: &PyObjectRef) -> PyResult<String> {
    let borrowed = addr.borrow();
    match &*borrowed {
        PyObject::Tuple(items) if items.len() == 2 => {
            let host = items[0].str();
            let port = items[1].as_i64().ok_or_else(|| PyError::type_error("port must be int"))?;
            Ok(format!("{}:{}", host, port))
        }
        PyObject::Str(s) => Ok(s.to_string()),
        _ => {
            // Fallback: use str representation
            Ok(addr.str())
        }
    }
}

pub struct ThreadInner {
    pub handle: Option<std::thread::JoinHandle<()>>,
    pub result: std::sync::Arc<std::sync::Mutex<Option<PyObjectRef>>>,
    pub target: PyObjectRef,
    pub args: Vec<PyObjectRef>,
}

#[derive(Clone)]
pub struct PyArray {
    pub typecode: char,
    pub data: Vec<f64>,
}

pub struct LockInner {
    pub lock: std::sync::atomic::AtomicBool,
}

pub struct RLockInner {
    pub owner: Option<std::thread::ThreadId>,
    pub count: u32,
}

pub struct EventInner {
    pub flag: std::sync::Mutex<bool>,
    pub condvar: std::sync::Condvar,
}

pub struct QueueInner {
    pub queue: std::collections::VecDeque<PyObjectRef>,
}

pub fn create_module(name: &str, dict: HashMap<String, PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Module {
        name: name.to_string(),
        dict,
    })
}

/// Helper: deep-copy a single object with memo support
pub fn deepcopy_one(obj: &PyObjectRef, memo: &PyObjectRef) -> Result<PyObjectRef, PyError> {
    // Check memo first using identity
    if let PyObject::Dict(memo_dict) = &*memo.borrow() {
        if let Some(cached) = memo_dict.get_by_identity(obj) {
            return Ok(cached);
        }
    }
    // Deep-copy based on type
    let result = {
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) | PyObject::Bool(_) | PyObject::None | PyObject::Bytes(_) => obj.clone(),
            PyObject::List(items) => {
                let mut new_items = Vec::new();
                for item in items {
                    new_items.push(deepcopy_one(item, memo)?);
                }
                py_list(new_items)
            }
            PyObject::Tuple(items) => {
                let mut new_items = Vec::new();
                for item in items {
                    new_items.push(deepcopy_one(item, memo)?);
                }
                PyObjectRef::imm(PyObject::Tuple(new_items))
            }
            PyObject::Dict(dict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.items() {
                    let new_k = deepcopy_one(&k, memo)?;
                    let new_v = deepcopy_one(&v, memo)?;
                    let _ = new_dict.set(new_k, new_v);
                }
                PyObjectRef::new(PyObject::Dict(new_dict))
            }
            _ => obj.clone(),
        }
    };
    // Store in memo for cycle detection
    if let PyObject::Dict(memo_dict) = &mut *memo.borrow_mut() {
        let _ = memo_dict.set(obj.clone(), result.clone());
    }
    Ok(result)
}

use std::sync::atomic::{AtomicI64, Ordering};
pub static ENUM_AUTO_COUNTER: AtomicI64 = AtomicI64::new(1);

// === IO MODULE ===
pub fn io_stringio_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("read() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        if let Some(buf) = dict.get("_buffer") {
            let buf_str = buf.str();
            let pos = dict.get("_pos").and_then(|p| p.as_i64()).unwrap_or(0) as usize;
            let result = buf_str[pos..].to_string();
            // Update position to end (full read consumed everything)
            drop(inst);
            if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                dict.insert("_pos".to_string(), py_int(buf_str.len() as i64));
            }
            Ok(py_str(&result))
        } else {
            Ok(py_str(""))
        }
    } else {
        Err(PyError::type_error("StringIO instance required"))
    }
}

pub fn io_stringio_readline(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("readline() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        if let Some(buf) = dict.get("_buffer") {
            let buf_str = buf.str();
            let pos = dict.get("_pos").and_then(|p| p.as_i64()).unwrap_or(0) as usize;
            if pos >= buf_str.len() {
                return Ok(py_str(""));
            }
            let remaining = &buf_str[pos..];
            let end = remaining.find('\n').map(|i| i + 1).unwrap_or(remaining.len());
            let result = &remaining[..end];
            let new_pos = pos + end;
            drop(inst);
            if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                dict.insert("_pos".to_string(), py_int(new_pos as i64));
            }
            Ok(py_str(result))
        } else {
            Ok(py_str(""))
        }
    } else {
        Err(PyError::type_error("StringIO instance required"))
    }
}

pub fn io_stringio_write(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Err(PyError::type_error("write() missing required argument")); }
    let self_obj = &args[0];
    let text = args[1].str();
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        let pos = dict.get("_pos").and_then(|p| p.as_i64()).unwrap_or(0) as usize;
        drop(inst);
        if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
            let buf = dict.entry("_buffer".to_string()).or_insert_with(|| py_str(""));
            let mut buf_str = buf.str();
            // Insert at position
            if pos <= buf_str.len() {
                buf_str.insert_str(pos, &text);
                *buf = py_str(&buf_str);
            }
            dict.insert("_pos".to_string(), py_int((pos + text.len()) as i64));
        }
        Ok(py_int(text.len() as i64))
    } else {
        Err(PyError::type_error("StringIO instance required"))
    }
}

pub fn io_stringio_seek(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Err(PyError::type_error("seek() missing required argument")); }
    let self_obj = &args[0];
    let pos = args[1].as_i64().unwrap_or(0);
    if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
        dict.insert("_pos".to_string(), py_int(pos));
    }
    Ok(py_int(pos))
}

pub fn io_stringio_tell(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("tell() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        Ok(dict.get("_pos").cloned().unwrap_or(py_int(0)))
    } else {
        Err(PyError::type_error("StringIO instance required"))
    }
}

pub fn io_stringio_getvalue(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("getvalue() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        Ok(dict.get("_buffer").cloned().unwrap_or(py_str("")))
    } else {
        Err(PyError::type_error("StringIO instance required"))
    }
}

pub fn io_bytesio_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("read() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        if let Some(buf) = dict.get("_buffer") {
            let buf_bytes = match &*buf.borrow() {
                PyObject::Bytes(b) => b.clone(),
                _ => vec![],
            };
            let pos = dict.get("_pos").and_then(|p| p.as_i64()).unwrap_or(0) as usize;
            let result = buf_bytes[pos..].to_vec();
            drop(inst);
            if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                dict.insert("_pos".to_string(), py_int(buf_bytes.len() as i64));
            }
            Ok(PyObjectRef::imm(PyObject::Bytes(result)))
        } else {
            Ok(PyObjectRef::imm(PyObject::Bytes(vec![])))
        }
    } else {
        Err(PyError::type_error("BytesIO instance required"))
    }
}

pub fn io_bytesio_readline(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("readline() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        if let Some(buf) = dict.get("_buffer") {
            let buf_bytes = match &*buf.borrow() {
                PyObject::Bytes(b) => b.clone(),
                _ => vec![],
            };
            let pos = dict.get("_pos").and_then(|p| p.as_i64()).unwrap_or(0) as usize;
            if pos >= buf_bytes.len() {
                return Ok(PyObjectRef::imm(PyObject::Bytes(vec![])));
            }
            let remaining = &buf_bytes[pos..];
            let end = remaining.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(remaining.len());
            let result = remaining[..end].to_vec();
            let new_pos = pos + end;
            drop(inst);
            if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                dict.insert("_pos".to_string(), py_int(new_pos as i64));
            }
            Ok(PyObjectRef::imm(PyObject::Bytes(result)))
        } else {
            Ok(PyObjectRef::imm(PyObject::Bytes(vec![])))
        }
    } else {
        Err(PyError::type_error("BytesIO instance required"))
    }
}

pub fn io_bytesio_write(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Err(PyError::type_error("write() missing required argument")); }
    let self_obj = &args[0];
    let data = args[1].borrow();
    let bytes = match &*data {
        PyObject::Bytes(b) => b.clone(),
        PyObject::Str(s) => s.as_bytes().to_vec(),
        _ => return Err(PyError::type_error("write() argument must be bytes or str")),
    };
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        let pos = dict.get("_pos").and_then(|p| p.as_i64()).unwrap_or(0) as usize;
        drop(inst);
        if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
            let buf = dict.entry("_buffer".to_string()).or_insert_with(|| PyObjectRef::imm(PyObject::Bytes(vec![])));
            let mut buf_bytes = match &*buf.borrow() {
                PyObject::Bytes(b) => b.clone(),
                _ => vec![],
            };
            // Insert at position (overwrite)
            if pos <= buf_bytes.len() {
                buf_bytes.splice(pos..pos, bytes.iter().cloned());
            } else {
                buf_bytes.extend(&bytes);
            }
            *buf = PyObjectRef::imm(PyObject::Bytes(buf_bytes));
            dict.insert("_pos".to_string(), py_int((pos + bytes.len()) as i64));
        }
        Ok(py_int(bytes.len() as i64))
    } else {
        Err(PyError::type_error("BytesIO instance required"))
    }
}

pub fn io_bytesio_seek(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Err(PyError::type_error("seek() missing required argument")); }
    let self_obj = &args[0];
    let pos = args[1].as_i64().unwrap_or(0);
    if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
        dict.insert("_pos".to_string(), py_int(pos));
    }
    Ok(py_int(pos))
}

pub fn io_bytesio_tell(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("tell() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        Ok(dict.get("_pos").cloned().unwrap_or(py_int(0)))
    } else {
        Err(PyError::type_error("BytesIO instance required"))
    }
}

pub fn io_bytesio_getvalue(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("getvalue() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        Ok(dict.get("_buffer").cloned().unwrap_or_else(|| PyObjectRef::imm(PyObject::Bytes(vec![]))))
    } else {
        Err(PyError::type_error("BytesIO instance required"))
    }
}

// ---- logging module ----
// basicConfig(level) stores level; getLogger(name) returns dict-like with .info/.debug/.warning/.error methods.
thread_local! {
    pub static LOG_LEVEL: std::cell::RefCell<String> = std::cell::RefCell::new("WARNING".to_string());
}

pub fn logging_debug(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Ok(py_none()); }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "DEBUG" && level != "INFO" && level != "WARNING" && level != "ERROR" && level != "CRITICAL" {
        return Ok(py_none());
    }
    let msg = args[1].str();
    let logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    Ok(py_none())
}

pub fn logging_info(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Ok(py_none()); }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "INFO" && level != "WARNING" && level != "ERROR" && level != "CRITICAL" {
        return Ok(py_none());
    }
    let msg = args[1].str();
    let logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    eprintln!("INFO:{}:{}", logger_name, msg);
    Ok(py_none())
}

pub fn logging_warning(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Ok(py_none()); }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "WARNING" && level != "ERROR" && level != "CRITICAL" {
        return Ok(py_none());
    }
    let msg = args[1].str();
    let logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    eprintln!("WARNING:{}:{}", logger_name, msg);
    Ok(py_none())
}

pub fn logging_error(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Ok(py_none()); }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "ERROR" && level != "CRITICAL" {
        return Ok(py_none());
    }
    let msg = args[1].str();
    let logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    eprintln!("ERROR:{}:{}", logger_name, msg);
    Ok(py_none())
}

/// Bit-by-bit CRC32 computation for gzip compress.
pub fn gzip_crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = 0xedb88320 ^ (crc >> 1);
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

// ---- pathlib module ----
thread_local! {
    pub static PATH_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

// ---- zipfile module ----
// Helper: extract ZIP entry data from an Instance's dict
fn zipfile_get_entry(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = &args[0];
    let name = args[1].borrow().str();
    let (entries, data) = match &*self_obj.borrow() {
        PyObject::Instance { dict, .. } => {
            let entries = dict.get("_entries").ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted: missing _entries"))?.clone();
            let data = dict.get("_data").ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted: missing _data"))?.clone();
            (entries, data)
        }
        _ => return Err(PyError::runtime_error("ZipFile method called on non-instance")),
    };

    let entries_list = match &*entries.borrow() {
        PyObject::List(items) => items.clone(),
        _ => return Err(PyError::runtime_error("ZipFile entries corrupted")),
    };

    let data_bytes = match &*data.borrow() {
        PyObject::Bytes(b) => b.clone(),
        _ => return Err(PyError::runtime_error("ZipFile data corrupted")),
    };

    for entry in &entries_list {
        let entry_borrow = entry.borrow();
        let entry_list = match &*entry_borrow {
            PyObject::List(items) => items,
            _ => continue,
        };
        if entry_list.len() < 5 { continue; }
        let entry_name = entry_list[0].borrow().str();
        if entry_name != name {
            continue;
        }
        let data_offset = match entry_list[1].as_i64() { Some(n) => n as usize, None => continue };
        let compressed_size = match entry_list[2].as_i64() { Some(n) => n as usize, None => continue };
        if data_offset + compressed_size > data_bytes.len() {
            return Err(PyError::runtime_error("ZipFile: data truncated in archive"));
        }
        let raw = data_bytes[data_offset..data_offset + compressed_size].to_vec();
        return Ok(PyObjectRef::new(PyObject::Bytes(raw)));
    }

    Err(PyError::key_error(format!("File not found in zip: '{}'", name)))
}

fn zipfile_namelist(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("namelist() requires self"));
    }
    match &*args[0].borrow() {
        PyObject::Instance { dict, .. } => {
            if let Some(names) = dict.get("_names") {
                return Ok(names.clone());
            }
            Err(PyError::runtime_error("ZipFile instance corrupted: missing _names"))
        }
        _ => Err(PyError::runtime_error("namelist() called on non-instance")),
    }
}

fn zipfile_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("read() takes exactly one argument (name)"));
    }
    zipfile_get_entry(args)
}

fn zipfile_extract(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("extract() takes exactly one argument (name)"));
    }
    zipfile_get_entry(args)
}

fn zipfile_infolist(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = &args[0];
    let entries = match &*self_obj.borrow() {
        PyObject::Instance { dict, .. } => {
            dict.get("_entries").ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted"))?.clone()
        }
        _ => return Err(PyError::runtime_error("infolist() called on non-instance")),
    };

    let entries_list = match &*entries.borrow() {
        PyObject::List(items) => items.clone(),
        _ => return Err(PyError::runtime_error("ZipFile entries corrupted")),
    };

    let infos: Vec<PyObjectRef> = entries_list.iter().map(|entry| {
        let entry_borrow = entry.borrow();
        let entry_list = match &*entry_borrow {
            PyObject::List(items) => items,
            _ => return py_none(),
        };
        let mut info_dict = HashMap::new();
        if entry_list.len() >= 1 {
            info_dict.insert("filename".to_string(), entry_list[0].clone());
        }
        if entry_list.len() >= 4 {
            info_dict.insert("file_size".to_string(), entry_list[3].clone());
        }
        if entry_list.len() >= 3 {
            info_dict.insert("compress_size".to_string(), entry_list[2].clone());
        }
        PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "zipfile.ZipInfo".to_string(),
                dict: HashMap::new(),
            }),
            dict: info_dict,
        })
    }).collect();

    Ok(py_list(infos))
}

pub fn zipfile_constructor(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 || args.len() > 2 {
        return Err(PyError::type_error("ZipFile() takes 1-2 arguments (filename, [mode])"));
    }
    let filename = args[0].borrow().str();
    let mode = if args.len() > 1 { args[1].borrow().str() } else { "r".to_string() };
    if mode != "r" {
        return Err(PyError::value_error("ZipFile only supports mode='r'"));
    }

    // Read entire file into memory
    let archive = match std::fs::read(&filename) {
        Ok(d) => d,
        Err(e) => return Err(PyError::runtime_error(format!("Cannot open zip file '{}': {}", filename, e))),
    };

    // Scan for local file headers (signature 0x04034b50)
    let archive_len = archive.len();
    let mut offset = 0usize;
    // entries stored as Vec of Python lists: [name, data_offset, compressed_size, uncompressed_size, compress_method]
    let mut names: Vec<PyObjectRef> = Vec::new();
    let mut entries: Vec<PyObjectRef> = Vec::new();

    loop {
        if offset + 30 > archive_len {
            break;
        }
        let sig = u32::from_le_bytes([
            archive[offset],
            archive[offset + 1],
            archive[offset + 2],
            archive[offset + 3],
        ]);
        if sig != 0x04034b50 {
            // Not a local file header — reached central directory or end
            break;
        }

        let compressed_size = u32::from_le_bytes([
            archive[offset + 18], archive[offset + 19],
            archive[offset + 20], archive[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            archive[offset + 22], archive[offset + 23],
            archive[offset + 24], archive[offset + 25],
        ]) as usize;
        let filename_length = u16::from_le_bytes([archive[offset + 26], archive[offset + 27]]) as usize;
        let extra_field_length = u16::from_le_bytes([archive[offset + 28], archive[offset + 29]]) as usize;

        let name_start = offset + 30;
        let data_start = name_start + filename_length + extra_field_length;

        let name = if filename_length > 0 && name_start + filename_length <= archive_len {
            String::from_utf8_lossy(&archive[name_start..name_start + filename_length]).to_string()
        } else {
            String::new()
        };

        names.push(py_str(&name));
        entries.push(PyObjectRef::new(PyObject::List(vec![
            py_str(&name),
            py_int(data_start as i64),
            py_int(compressed_size as i64),
            py_int(uncompressed_size as i64),
            // compress_method stored separately in entries_meta if needed
        ])));

        offset = data_start + compressed_size;
    }

    let mut inst_dict = HashMap::new();
    inst_dict.insert("filename".to_string(), py_str(&filename));
    inst_dict.insert("_data".to_string(), PyObjectRef::new(PyObject::Bytes(archive)));
    inst_dict.insert("_names".to_string(), py_list(names));
    inst_dict.insert("_entries".to_string(), py_list(entries));

    // Attach methods as BuiltinFunctions (will be wrapped as BuiltinMethod with self_obj)
    inst_dict.insert("namelist".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "namelist".to_string(),
        func: zipfile_namelist,
    }));
    inst_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "read".to_string(),
        func: zipfile_read,
    }));
    inst_dict.insert("extract".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "extract".to_string(),
        func: zipfile_extract,
    }));
    inst_dict.insert("infolist".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "infolist".to_string(),
        func: zipfile_infolist,
    }));

    Ok(PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Module {
            name: "zipfile.ZipFile".to_string(),
            dict: HashMap::new(),
        }),
        dict: inst_dict,
    }))
}

// === MIMETYPES MODULE ===
use once_cell::sync::Lazy;

// Static MIME type database: extension -> (type, encoding)
static KNOWN_TYPES: Lazy<HashMap<String, (String, String)>> = Lazy::new(|| {
    HashMap::from([
        (".html".to_string(), ("text/html".to_string(), "".to_string())),
        (".htm".to_string(), ("text/html".to_string(), "".to_string())),
        (".css".to_string(), ("text/css".to_string(), "".to_string())),
        (".js".to_string(), ("application/javascript".to_string(), "".to_string())),
        (".json".to_string(), ("application/json".to_string(), "".to_string())),
        (".xml".to_string(), ("application/xml".to_string(), "".to_string())),
        (".txt".to_string(), ("text/plain".to_string(), "".to_string())),
        (".csv".to_string(), ("text/csv".to_string(), "".to_string())),
        (".md".to_string(), ("text/markdown".to_string(), "".to_string())),
        (".py".to_string(), ("text/x-python".to_string(), "".to_string())),
        (".png".to_string(), ("image/png".to_string(), "".to_string())),
        (".jpg".to_string(), ("image/jpeg".to_string(), "".to_string())),
        (".jpeg".to_string(), ("image/jpeg".to_string(), "".to_string())),
        (".gif".to_string(), ("image/gif".to_string(), "".to_string())),
        (".bmp".to_string(), ("image/bmp".to_string(), "".to_string())),
        (".ico".to_string(), ("image/x-icon".to_string(), "".to_string())),
        (".svg".to_string(), ("image/svg+xml".to_string(), "".to_string())),
        (".webp".to_string(), ("image/webp".to_string(), "".to_string())),
        (".mp3".to_string(), ("audio/mpeg".to_string(), "".to_string())),
        (".wav".to_string(), ("audio/wav".to_string(), "".to_string())),
        (".ogg".to_string(), ("audio/ogg".to_string(), "".to_string())),
        (".mp4".to_string(), ("video/mp4".to_string(), "".to_string())),
        (".webm".to_string(), ("video/webm".to_string(), "".to_string())),
        (".avi".to_string(), ("video/x-msvideo".to_string(), "".to_string())),
        (".mov".to_string(), ("video/quicktime".to_string(), "".to_string())),
        (".pdf".to_string(), ("application/pdf".to_string(), "".to_string())),
        (".zip".to_string(), ("application/zip".to_string(), "".to_string())),
        (".gz".to_string(), ("application/gzip".to_string(), "".to_string())),
        (".tar".to_string(), ("application/x-tar".to_string(), "".to_string())),
        (".rar".to_string(), ("application/vnd.rar".to_string(), "".to_string())),
        (".7z".to_string(), ("application/x-7z-compressed".to_string(), "".to_string())),
        (".exe".to_string(), ("application/x-msdownload".to_string(), "".to_string())),
        (".bin".to_string(), ("application/octet-stream".to_string(), "".to_string())),
        (".wasm".to_string(), ("application/wasm".to_string(), "".to_string())),
        (".woff".to_string(), ("font/woff".to_string(), "".to_string())),
        (".woff2".to_string(), ("font/woff2".to_string(), "".to_string())),
        (".ttf".to_string(), ("font/ttf".to_string(), "".to_string())),
        (".otf".to_string(), ("font/otf".to_string(), "".to_string())),
        (".yaml".to_string(), ("text/yaml".to_string(), "".to_string())),
        (".yml".to_string(), ("text/yaml".to_string(), "".to_string())),
        (".toml".to_string(), ("application/toml".to_string(), "".to_string())),
        (".doc".to_string(), ("application/msword".to_string(), "".to_string())),
        (".docx".to_string(), ("application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string(), "".to_string())),
        (".xls".to_string(), ("application/vnd.ms-excel".to_string(), "".to_string())),
        (".xlsx".to_string(), ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(), "".to_string())),
        (".ppt".to_string(), ("application/vnd.ms-powerpoint".to_string(), "".to_string())),
        (".pptx".to_string(), ("application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string(), "".to_string())),
        (".rtf".to_string(), ("application/rtf".to_string(), "".to_string())),
    ])
});

// Static reverse mapping: type -> extension
static KNOWN_EXTS: Lazy<HashMap<String, String>> = Lazy::new(|| {
    HashMap::from([
        ("text/html".to_string(), ".html".to_string()),
        ("text/css".to_string(), ".css".to_string()),
        ("application/javascript".to_string(), ".js".to_string()),
        ("application/json".to_string(), ".json".to_string()),
        ("application/xml".to_string(), ".xml".to_string()),
        ("text/plain".to_string(), ".txt".to_string()),
        ("text/csv".to_string(), ".csv".to_string()),
        ("text/markdown".to_string(), ".md".to_string()),
        ("text/x-python".to_string(), ".py".to_string()),
        ("image/png".to_string(), ".png".to_string()),
        ("image/jpeg".to_string(), ".jpg".to_string()),
        ("image/gif".to_string(), ".gif".to_string()),
        ("image/bmp".to_string(), ".bmp".to_string()),
        ("image/x-icon".to_string(), ".ico".to_string()),
        ("image/svg+xml".to_string(), ".svg".to_string()),
        ("image/webp".to_string(), ".webp".to_string()),
        ("audio/mpeg".to_string(), ".mp3".to_string()),
        ("audio/wav".to_string(), ".wav".to_string()),
        ("audio/ogg".to_string(), ".ogg".to_string()),
        ("video/mp4".to_string(), ".mp4".to_string()),
        ("video/webm".to_string(), ".webm".to_string()),
        ("video/x-msvideo".to_string(), ".avi".to_string()),
        ("video/quicktime".to_string(), ".mov".to_string()),
        ("application/pdf".to_string(), ".pdf".to_string()),
        ("application/zip".to_string(), ".zip".to_string()),
        ("application/gzip".to_string(), ".gz".to_string()),
        ("application/x-tar".to_string(), ".tar".to_string()),
        ("application/vnd.rar".to_string(), ".rar".to_string()),
        ("application/x-7z-compressed".to_string(), ".7z".to_string()),
        ("application/x-msdownload".to_string(), ".exe".to_string()),
        ("application/octet-stream".to_string(), ".bin".to_string()),
        ("application/wasm".to_string(), ".wasm".to_string()),
        ("font/woff".to_string(), ".woff".to_string()),
        ("font/woff2".to_string(), ".woff2".to_string()),
        ("font/ttf".to_string(), ".ttf".to_string()),
        ("font/otf".to_string(), ".otf".to_string()),
        ("text/yaml".to_string(), ".yaml".to_string()),
        ("application/toml".to_string(), ".toml".to_string()),
        ("application/msword".to_string(), ".doc".to_string()),
        ("application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string(), ".docx".to_string()),
        ("application/vnd.ms-excel".to_string(), ".xls".to_string()),
        ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(), ".xlsx".to_string()),
        ("application/vnd.ms-powerpoint".to_string(), ".ppt".to_string()),
        ("application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string(), ".pptx".to_string()),
        ("application/rtf".to_string(), ".rtf".to_string()),
    ])
});

pub fn mime_guess_type(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("guess_type() takes at least 1 argument"));
    }
    let url = args[0].str();
    // Strip query string and fragment
    let path = url.split('?').next().unwrap_or("").split('#').next().unwrap_or("");
    let ext = {
        let p = path.rfind('.').map(|i| &path[i..]).unwrap_or("");
        p.to_lowercase()
    };
    let (mime_type, encoding) = KNOWN_TYPES.get(&ext).cloned().unwrap_or_else(|| {
        ("application/octet-stream".to_string(), "".to_string())
    });
    let encoding = if encoding.is_empty() { py_none() } else { py_str(&encoding) };
    let result = PyObjectRef::new(PyObject::Tuple(vec![py_str(&mime_type), encoding]));
    Ok(result)
}

pub fn mime_guess_extension(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("guess_extension() takes at least 1 argument"));
    }
    let mime_type = args[0].str();
    let ext = KNOWN_EXTS.get(&mime_type);
    match ext {
        Some(e) => Ok(py_str(e)),
        None => Ok(py_none()),
    }
}

pub fn mime_add_type(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("add_type() takes at least 2 arguments (type, ext)"));
    }
    let _ = args;
    Ok(py_none())
}

// === SHELVE MODULE ===
// Shelf class backed by a dict. open(filename) -> Shelf instance.

/// Extract the _data dict from a Shelf Instance (args[0]).
fn shelf_get_data(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("method requires self"));
    }
    match &*args[0].borrow() {
        PyObject::Instance { dict, .. } => {
            match dict.get("_data") {
                Some(data) => Ok(data.clone()),
                None => Err(PyError::runtime_error("Shelf instance corrupted: missing _data")),
            }
        }
        _ => Err(PyError::type_error("expected Shelf instance")),
    }
}

fn shelf_close(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _ = args;
    Ok(py_none())
}

fn shelf_sync(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _ = args;
    Ok(py_none())
}

fn shelf_get(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // args[0] = self, args[1] = key, args[2] = default (optional)
    if args.len() < 2 {
        return Err(PyError::type_error("get() takes at least 2 arguments (self, key)"));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        match d.get(&py_key)? {
            Some(val) => Ok(val),
            None => {
                if args.len() > 2 {
                    Ok(args[2].clone())
                } else {
                    Ok(py_none())
                }
            }
        }
    } else {
        Ok(py_none())
    }
}

fn shelf_keys(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let ks = d.keys();
        Ok(PyObjectRef::new(PyObject::List(ks)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

fn shelf_values(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let vs = d.values();
        Ok(PyObjectRef::new(PyObject::List(vs)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

fn shelf_items(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let pairs: Vec<PyObjectRef> = d.items().into_iter().map(|(k, v)| {
            PyObjectRef::new(PyObject::Tuple(vec![k, v]))
        }).collect();
        Ok(PyObjectRef::new(PyObject::List(pairs)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

// __len__(self) -> int (for len())
fn shelf_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        Ok(py_int(d.len() as i64))
    } else {
        Ok(py_int(0))
    }
}

// __contains__(self, key) -> bool (for 'key in shelf')
fn shelf_contains(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("__contains__() takes at least 2 arguments (self, key)"));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        Ok(py_bool(d.contains(&py_key)?))
    } else {
        Ok(py_bool(false))
    }
}

// __repr__(self) -> str
fn shelf_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        Ok(py_str(&format!("Shelf({} items)", d.len())))
    } else {
        Ok(py_str("Shelf(0 items)"))
    }
}

// __getitem__(self, key) -> value (for shelf[key])
fn shelf_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("__getitem__() takes at least 2 arguments (self, key)"));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        match d.get(&py_key)? {
            Some(val) => Ok(val),
            None => Err(PyError::key_error(format!("'{}'", key))),
        }
    } else {
        Err(PyError::key_error(format!("'{}'", key)))
    }
}

// __setitem__(self, key, value) (for shelf[key] = value)
fn shelf_setitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error("__setitem__() takes at least 3 arguments (self, key, value)"));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    {
        let mut data_mut = data.borrow_mut();
        if let PyObject::Dict(ref mut d) = &mut *data_mut {
            d.set(py_str(&key), args[2].clone())?;
        }
    }
    Ok(py_none())
}

// __delitem__(self, key) (for del shelf[key])
fn shelf_delitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("__delitem__() takes at least 2 arguments (self, key)"));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    {
        let mut data_mut = data.borrow_mut();
        if let PyObject::Dict(ref mut d) = &mut *data_mut {
            let py_key = py_str(&key);
            d.remove(&py_key)?;
        }
    }
    Ok(py_none())
}

pub fn shelf_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("open() takes at least 1 argument (filename)"));
    }
    let filename = args[0].str();

    // Internal data dict
    let data_dict = py_dict();

    // Instance dict with field and methods
    let mut inst_dict = HashMap::new();
    inst_dict.insert("_data".to_string(), data_dict);
    inst_dict.insert("filename".to_string(), py_str(&filename));

    inst_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "close".to_string(), func: shelf_close }));
    inst_dict.insert("sync".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "sync".to_string(), func: shelf_sync }));
    inst_dict.insert("get".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "get".to_string(), func: shelf_get }));
    inst_dict.insert("keys".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "keys".to_string(), func: shelf_keys }));
    inst_dict.insert("values".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "values".to_string(), func: shelf_values }));
    inst_dict.insert("items".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "items".to_string(), func: shelf_items }));

    // Type dict with dunder methods (used by py_getitem/py_setitem dispatch)
    let mut type_dict = HashMap::new();
    type_dict.insert("__getitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__getitem__".to_string(), func: shelf_getitem }));
    type_dict.insert("__setitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__setitem__".to_string(), func: shelf_setitem }));
    type_dict.insert("__delitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__delitem__".to_string(), func: shelf_delitem }));
    type_dict.insert("__len__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__len__".to_string(), func: shelf_len }));
    type_dict.insert("__contains__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__contains__".to_string(), func: shelf_contains }));
    type_dict.insert("__repr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__repr__".to_string(), func: shelf_repr }));

    // Build Shelf type
    let shelf_type = PyObjectRef::new(PyObject::Type {
        name: "Shelf".to_string(),
        dict: type_dict,
        bases: vec![],
        // MRO includes self so __getitem__ lookup works
        mro: vec![],
    });

    let instance = PyObjectRef::new(PyObject::Instance {
        typ: shelf_type,
        dict: inst_dict,
    });

    Ok(instance)
}

// ---- urllib module ----

/// Create a response object for urlopen with a read() method.
/// The response body bytes are stored in the instance dict under "_body".
fn create_urlopen_response(body: Vec<u8>) -> PyObjectRef {
    use std::collections::HashMap;

    // Create the response type with a read() method
    let mut type_dict = HashMap::new();
    type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "read".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("read() missing argument"));
            }
            let body = args[0].borrow();
            if let PyObject::Instance { dict, .. } = &*body {
                if let Some(body_val) = dict.get("_body") {
                    return Ok(body_val.clone());
                }
            }
            Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())))
        },
    }));

    let resp_type = PyObjectRef::new(PyObject::Type {
        name: "HTTPResponse".to_string(),
        dict: type_dict,
        bases: vec![],
        mro: vec![],
    });

    let mut instance_dict = HashMap::new();
    instance_dict.insert("_body".to_string(), PyObjectRef::imm(PyObject::Bytes(body)));
    PyObjectRef::new(PyObject::Instance {
        typ: resp_type,
        dict: instance_dict,
    })
}

pub fn create_urllib_request_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! request_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    request_func!("urlopen", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("urlopen() missing required argument 'url'"));
        }
        let url_str = args[0].str();

        // Only support http:// URLs with a simple GET
        if !url_str.starts_with("http://") {
            return Err(PyError::type_error(format!("urlopen() only supports http:// URLs, got: {}", url_str)));
        }

        let rest = url_str.trim_start_matches("http://");
        let (host_port, path) = match rest.find('/') {
            Some(pos) => (&rest[..pos], &rest[pos..]),
            None => (rest, "/"),
        };

        let (host, port) = if let Some(colon_pos) = host_port.find(':') {
            (&host_port[..colon_pos], host_port[colon_pos+1..].parse::<u16>().unwrap_or(80))
        } else {
            (host_port, 80u16)
        };

        if host.is_empty() {
            return Err(PyError::type_error("urlopen() invalid URL: empty host"));
        }

        // Connect via TcpStream
        let addr = format!("{}:{}", host, port);
        let stream = match std::net::TcpStream::connect(&addr) {
            Ok(s) => s,
            Err(e) => return Err(PyError::runtime_error(format!("urlopen() failed to connect: {}", e))),
        };

        // Send HTTP GET request
        let request = format!("GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n", path, host);
        {
            use std::io::Write;
            if let Err(e) = (&stream).write_all(request.as_bytes()) {
                return Err(PyError::runtime_error(format!("urlopen() write error: {}", e)));
            }
        }

        // Read response
        let mut response = Vec::new();
        {
            use std::io::Read;
            if let Err(e) = (&stream).read_to_end(&mut response) {
                return Err(PyError::runtime_error(format!("urlopen() read error: {}", e)));
            }
        }

        // Parse HTTP response
        let response_str = String::from_utf8_lossy(&response);
        let body = if let Some(body_start) = response_str.find("\r\n\r\n") {
            let header_end = body_start + 4;
            if header_end < response.len() {
                response[header_end..].to_vec()
            } else {
                Vec::new()
            }
        } else {
            // No headers found, return raw response as body
            response.clone()
        };

        Ok(create_urlopen_response(body))
    });

    d
}

/// Percent-encode a character (for quote)
fn percent_encode_byte(byte: u8) -> String {
    format!("%{:02X}", byte)
}

/// Percent-decode a string (for unquote)
fn percent_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            // Invalid percent encoding, preserve original
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if a byte should be encoded in URL (for quote)
fn needs_percent_encode(byte: u8, safe: &str) -> bool {
    // Always safe: unreserved characters per RFC 3986
    if byte.is_ascii_alphanumeric() {
        return false;
    }
    // Also safe: these unreserved chars
    if matches!(byte, b'_' | b'-' | b'.' | b'~') {
        return false;
    }
    // Check user-provided safe chars
    if safe.as_bytes().contains(&byte) {
        return false;
    }
    true
}

pub fn create_urllib_parse_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! parse_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // urlparse(url, scheme='', allow_fragments=True)
    parse_func!("urlparse", |args| {
        let url = if args.len() > 0 { args[0].str() } else { return Err(PyError::type_error("urlparse() missing required argument 'url'")); };
        let scheme_default = if args.len() > 1 { args[1].str() } else { String::new() };

        let mut scheme = scheme_default;
        let mut netloc = String::new();
        let mut params = String::new();
        let mut query = String::new();
        let mut fragment = String::new();
        let mut path = String::new();

        // Split fragment (allow_fragments defaults to true)
        let allow_fragments = if args.len() > 2 { args[2].truthy() } else { true };
        let remaining = if allow_fragments {
            if let Some(pos) = url.find('#') {
                fragment = url[pos+1..].to_string();
                url[..pos].to_string()
            } else {
                url.clone()
            }
        } else {
            url.clone()
        };

        // Split query
        let remaining = if let Some(pos) = remaining.find('?') {
            query = remaining[pos+1..].to_string();
            remaining[..pos].to_string()
        } else {
            remaining
        };

        // Extract scheme
        if let Some(pos) = remaining.find("://") {
            scheme = remaining[..pos].to_string();
            let after_scheme = &remaining[pos+3..];
            // Extract netloc (host:port or host)
            if let Some(slash_pos) = after_scheme.find('/') {
                netloc = after_scheme[..slash_pos].to_string();
                path = after_scheme[slash_pos..].to_string();
            } else {
                netloc = after_scheme.to_string();
            }
        } else {
            path = remaining;
        }

        // Split params from path (last semicolon in path segment)
        if let Some(pos) = path.rfind(';') {
            params = path[pos+1..].to_string();
            path = path[..pos].to_string();
        }

        // Create result type with scheme, netloc, path, params, query, fragment attributes
        let type_dict = HashMap::new();
        let parse_type = PyObjectRef::new(PyObject::Type {
            name: "ParseResult".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });

        let mut instance_dict = HashMap::new();
        instance_dict.insert("scheme".to_string(), py_str(&scheme));
        instance_dict.insert("netloc".to_string(), py_str(&netloc));
        instance_dict.insert("path".to_string(), py_str(&path));
        instance_dict.insert("params".to_string(), py_str(&params));
        instance_dict.insert("query".to_string(), py_str(&query));
        instance_dict.insert("fragment".to_string(), py_str(&fragment));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: parse_type,
            dict: instance_dict,
        }))
    });

    // urlencode(query, doseq=False)
    parse_func!("urlencode", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("urlencode() missing required argument 'query'"));
        }
        let _doseq = if args.len() > 1 { args[1].truthy() } else { false };

        let obj = args[0].borrow();
        let mut pairs: Vec<(String, String)> = Vec::new();

        match &*obj {
            PyObject::Dict(dict) => {
                for (k, v) in dict.items() {
                    let key = k.str();
                    let val = v.str();
                    pairs.push((key, val));
                }
            }
            PyObject::List(items) | PyObject::Tuple(items) => {
                for item in items {
                    let item_ref = item.borrow();
                    if let PyObject::Tuple(pair) = &*item_ref {
                        if pair.len() >= 2 {
                            let key = pair[0].str();
                            let val = pair[1].str();
                            pairs.push((key, val));
                        }
                    } else if let PyObject::List(pair) = &*item_ref {
                        if pair.len() >= 2 {
                            let key = pair[0].str();
                            let val = pair[1].str();
                            pairs.push((key, val));
                        }
                    } else {
                        // Try to iterate
                        let key = item.str();
                        pairs.push((key, String::new()));
                    }
                }
            }
            _ => {
                return Err(PyError::type_error("urlencode() argument must be dict, list of tuples, or list of lists"));
            }
        }

        // Percent-encode both keys and values
        let encoded: Vec<String> = pairs.into_iter().map(|(k, v)| {
            let enc_key: String = k.bytes().map(|b| {
                if needs_percent_encode(b, "") { percent_encode_byte(b) }
                else { (b as char).to_string() }
            }).collect::<Vec<_>>().concat();
            let enc_val: String = v.bytes().map(|b| {
                if needs_percent_encode(b, "") { percent_encode_byte(b) }
                else { (b as char).to_string() }
            }).collect::<Vec<_>>().concat();
            format!("{}={}", enc_key, enc_val)
        }).collect();

        Ok(py_str(&encoded.join("&")))
    });

    // quote(string, safe='/')
    parse_func!("quote", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("quote() missing required argument 'string'"));
        }
        let s = args[0].str();
        let safe = if args.len() > 1 { args[1].str() } else { "/".to_string() };

        let encoded: String = s.bytes().map(|b| {
            if needs_percent_encode(b, &safe) { percent_encode_byte(b) }
            else { (b as char).to_string() }
        }).collect::<Vec<_>>().concat();

        Ok(py_str(&encoded))
    });

    // unquote(string, encoding='utf-8', errors='replace')
    parse_func!("unquote", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unquote() missing required argument 'string'"));
        }
        let s = args[0].str();
        Ok(py_str(&percent_decode(&s)))
    });

    d
}

/// Helper: provide dict methods (items, keys, values, __iter__) for Instance objects
/// that inherit from dict but can't access the built-in dict methods.
fn instance_builtin_dict_method(name: &str, dict_snapshot: Vec<(String, PyObjectRef)>) -> Option<PyObjectRef> {
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
    // Directly insert into the Instance's dict, bypassing py_setitem (which would recurse)
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
