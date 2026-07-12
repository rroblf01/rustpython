use std::rc::Rc;
use std::cell::RefCell;
use std::fmt;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign};
use num_traits::{Zero, One, ToPrimitive, float::FloatCore, Signed};
use crate::bytecode::CodeObject;

pub type BuiltinFunc = fn(&[PyObjectRef]) -> PyResult<PyObjectRef>;

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
        // Safe: we only store valid UTF-8
        unsafe { std::str::from_utf8_unchecked(&self.data[..self.len as usize]) }
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
    Imm(Rc<PyObject>),           // Immutable: Int, Str, Float, Tuple, Bytes, ByteArray, Range, Slice, Code, Function
}

impl PyObjectRef {
    /// Create a MUTABLE PyObjectRef (for List, Dict, Set, Instance)
    pub fn new(obj: PyObject) -> Self {
        PyObjectRef::Mut(Rc::new(RefCell::new(obj)))
    }

    /// Create an IMMUTABLE PyObjectRef (for Int, Str, Float, etc.)
    pub fn imm(obj: PyObject) -> Self {
        PyObjectRef::Imm(Rc::new(obj))
    }

    pub fn borrow(&self) -> RefOrOwned<'_> {
        match self {
            PyObjectRef::SmallInt(n) => RefOrOwned::Owned(PyObject::Int(BigInt::from(*n))),
            PyObjectRef::SmallBool(b) => RefOrOwned::Owned(PyObject::Bool(*b)),
            PyObjectRef::SmallFloat(f) => RefOrOwned::Owned(PyObject::Float(*f)),
            PyObjectRef::SmallStr(s) => RefOrOwned::Owned(PyObject::Str(s.to_string())),
            PyObjectRef::None => RefOrOwned::Owned(PyObject::None),
            PyObjectRef::Mut(rc) => RefOrOwned::Ref(rc.borrow()),
            PyObjectRef::Imm(rc) => RefOrOwned::RcRef(Rc::clone(rc)),
        }
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, PyObject> {
        match self { PyObjectRef::Mut(rc) => rc.borrow_mut(), _ => panic!("borrow_mut on non-mutable value") }
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
    pub fn str(&self) -> String { self.borrow().str() }
    pub fn truthy(&self) -> bool {
        match self {
            PyObjectRef::SmallInt(n) => *n != 0,
            PyObjectRef::SmallBool(b) => *b,
            PyObjectRef::SmallFloat(f) => *f != 0.0,
            PyObjectRef::SmallStr(s) => !s.as_str().is_empty(),
            PyObjectRef::None => false,
            PyObjectRef::Mut(rc) => rc.borrow().truthy(),
            PyObjectRef::Imm(rc) => (**rc).truthy(),
        }
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
            PyObjectRef::Imm(rc) => (**rc).hash(),
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
            PyObjectRef::Imm(rc) => Rc::as_ptr(rc) as usize,
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
}

// ---- PyDict: hash-based dict with arbitrary hashable keys ----

#[derive(Clone)]
pub struct PyDict {
    buckets: std::collections::HashMap<usize, Vec<(PyObjectRef, PyObjectRef)>>,
    size: usize,
}

impl PyDict {
    pub fn new() -> Self { PyDict { buckets: std::collections::HashMap::new(), size: 0 } }
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
            Some(i) => bucket[i].1 = value,
            None => { bucket.push((key, value)); self.size += 1; }
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
    Str(String),
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
    },
    Socket {
        inner: std::rc::Rc<std::cell::RefCell<SocketInner>>,
    },
    Thread(std::sync::Arc<std::sync::Mutex<ThreadInner>>),
    Lock(std::sync::Arc<std::sync::Mutex<LockInner>>),
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
            PyObject::BuildClass => "builtin_function_or_method",
            PyObject::BoundMethod { .. } => "method",
            PyObject::Partial { .. } => "partial",
            PyObject::File { .. } => "file",
            PyObject::Socket { .. } => "socket",
            PyObject::Thread(_) => "Thread",
            PyObject::Lock(_) => "lock",
            PyObject::Super { .. } => "super",
            PyObject::Property { .. } => "property",
            PyObject::StaticMethod { .. } => "staticmethod",
            PyObject::ClassMethod { .. } => "classmethod",
            PyObject::Generator { .. } => "generator",
            PyObject::Coroutine { .. } => "coroutine",
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
            PyObject::BuildClass => "<builtin function __build_class__>".to_string(),
            PyObject::BoundMethod { func, .. } => format!("<bound method {}>", func.borrow().type_name()),
            PyObject::Partial { func, .. } => format!("<partial {}>", func.borrow().type_name()),
            PyObject::File { .. } => format!("<_io.FileIO '...'>"),
            PyObject::Socket { .. } => format!("<socket object>"),
            PyObject::Thread(_) => "<Thread>".to_string(),
            PyObject::Lock(_) => "<lock>".to_string(),
            PyObject::Super { .. } => format!("<super object>"),
            PyObject::Property { .. } => format!("<property object>"),
            PyObject::StaticMethod { .. } => format!("<staticmethod object>"),
            PyObject::ClassMethod { .. } => format!("<classmethod object>"),
            PyObject::Generator { .. } => format!("<generator object>"),
            PyObject::Coroutine { .. } => format!("<coroutine object>"),
        }
    }

    pub fn str(&self) -> String {
        match self {
            PyObject::Str(s) => s.clone(),
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
                        PyObject::Type { dict: type_dict, .. } => type_dict.get("__bool__").cloned(),
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
                        PyObject::Type { dict: type_dict, .. } => type_dict.get("__hash__").cloned(),
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

pub fn py_int(i: impl Into<BigInt>) -> PyObjectRef {
    let big = i.into();
    if let Some(n) = big.to_i64() {
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

pub fn py_float(f: f64) -> PyObjectRef {
    // Use inline SmallFloat to avoid Rc + heap alloc
    PyObjectRef::SmallFloat(f)
}

pub fn py_str(s: &str) -> PyObjectRef {
    // Use inline SmallStr for strings < 16 bytes to avoid Rc + heap alloc
    if let Some(small) = SmallStr::new(s) {
        return PyObjectRef::SmallStr(small);
    }
    PyObjectRef::imm(PyObject::Str(s.to_string()))
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

    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.next() {
                None => return Err("incomplete format: trailing %".to_string()),
                Some('%') => result.push('%'),
                Some('s') => {
                    let val = arg.str();
                    result.push_str(&val);
                }
                Some('d') | Some('i') => {
                    let val = if let Some(i) = arg.as_i64() {
                        i.to_string()
                    } else {
                        arg.str()
                    };
                    result.push_str(&val);
                }
                Some('r') => {
                    let val = format!("'{}'", arg.str());
                    result.push_str(&val);
                }
                Some('f') => {
                    let val = arg.str();
                    result.push_str(&val);
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
                PyObject::Type { dict: type_dict, .. } => type_dict.get(name).cloned(),
                _ => None,
            }
        }
        _ => None,
    }
}

trait Compare {
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
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__contains__").cloned(),
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
        PyObject::Instance { typ, .. } => {
            let f = {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__len__").cloned(),
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
                let n = n.to_usize().unwrap_or(0);
                let items: Vec<PyObjectRef> = (0..n).map(|i| py_int(i as i64)).collect();
                Ok(py_list(items))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        2 => {
            let start = args[0].borrow();
            let stop = args[1].borrow();
            if let (PyObject::Int(a), PyObject::Int(b)) = (&*start, &*stop) {
                let a = a.to_i64().unwrap_or(0);
                let b = b.to_i64().unwrap_or(0);
                let items: Vec<PyObjectRef> = (a..b).map(|i| py_int(i)).collect();
                Ok(py_list(items))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        3 => {
            let start = args[0].borrow();
            let stop = args[1].borrow();
            let step = args[2].borrow();
            if let (PyObject::Int(a), PyObject::Int(b), PyObject::Int(s)) = (&*start, &*stop, &*step) {
                let a = a.to_i64().unwrap_or(0);
                let b = b.to_i64().unwrap_or(0);
                let s = s.to_i64().unwrap_or(1);
                if s == 0 { return Err(PyError::value_error("range() arg 3 must not be zero")); }
                let mut items = Vec::new();
                let mut i = a;
                if s > 0 {
                    while i < b {
                        items.push(py_int(i));
                        i += s;
                    }
                } else {
                    while i > b {
                        items.push(py_int(i));
                        i += s;
                    }
                }
                Ok(py_list(items))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        _ => Err(PyError::type_error("range() takes at most 3 arguments")),
    }
}

pub fn builtin_type_of(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("type() takes exactly one argument"));
    }
    let name = args[0].borrow().type_name();
    Ok(PyObjectRef::imm(PyObject::Str(name)))
}

pub fn builtin_int(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Ok(py_int(0)); }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(_) => Ok(args[0].clone()),
        PyObject::Float(f) => Ok(py_int(*f as i64)),
        PyObject::Str(s) => {
            let n: i64 = s.trim().parse().map_err(|_| PyError::value_error(format!("invalid literal for int(): '{}'", s)))?;
            Ok(py_int(n))
        }
        PyObject::Bool(b) => Ok(py_int(if *b { 1 } else { 0 })),
        _ => Err(PyError::type_error(format!("int() argument must be a string or number, not '{}'", obj.type_name()))),
    }
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
            match &*obj_borrowed {
                PyObject::Instance { typ, .. } => {
                    let typ_ref = typ.borrow();
                    match &*typ_ref {
                        PyObject::Type { dict: type_dict, .. } => type_dict.get("__str__").cloned(),
                        _ => None,
                    }
                }
                _ => None,
            }
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
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__repr__").cloned(),
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
            _ => Err(PyError::type_error(format!("cannot convert '{}' to bytes", obj.type_name()))),
        }
    }
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
    let vm_ptr = VM_PTR.with(|p| *p.borrow());
    if let Some(ptr) = vm_ptr {
        let vm = unsafe { &*ptr };
        let frame = vm.frames.last().ok_or_else(|| PyError::runtime_error("no frame"))?;
        let globals = frame.globals.borrow();
        let mut d = crate::object::PyDict::new();
        for (k, v) in globals.iter() {
            d.set(py_str(k), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(d)))
    } else {
        Ok(py_dict())
    }
}

pub fn builtin_locals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let vm_ptr = VM_PTR.with(|p| *p.borrow());
    if let Some(ptr) = vm_ptr {
        let vm = unsafe { &*ptr };
        let frame = vm.frames.last().ok_or_else(|| PyError::runtime_error("no frame"))?;
        let mut d = crate::object::PyDict::new();
        for (k, v) in frame.locals.iter() {
            d.set(py_str(k), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(d)))
    } else {
        Ok(py_dict())
    }
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
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__delattr__").cloned(),
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
        PyObject::Function { code, globals: g, defaults, .. } => {
            let mut frame = super::vm::Frame::new(std::rc::Rc::new(code.clone()), g.clone(), std::rc::Rc::new(create_builtins()));
            let code = code.clone();
            let defaults = defaults.clone();
            // Set self at index 0
            if !code.varnames.is_empty() {
                frame.fast_locals[0] = Some(self_obj.clone());
                frame.locals.insert(code.varnames[0].clone(), self_obj);
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
                    frame.locals.insert(code.varnames[idx].clone(), args[i].clone());
                }
            }
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in (named_params.saturating_sub(1))..npos {
                    extra.push(args[i].clone());
                }
                frame.locals.insert(vararg_name.clone(), py_tuple(extra));
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
                            frame.locals.insert(code.varnames[idx].clone(), val);
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
    let mut index = start;
    let mut items = Vec::new();
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => {
                items.push(py_tuple(vec![py_int(index as i64), val]));
                index += 1;
            }
            Err(PyError::StopIteration) => return Ok(py_list(items)),
            Err(e) => return Err(e),
        }
    }
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
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__iter__").cloned(),
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
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__next__").cloned(),
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
                return f(&[args[0].clone()]);
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
                PyObject::Str(s) => s.clone(),
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
    Ok(PyObjectRef::new(PyObject::File { file: std::rc::Rc::new(std::cell::RefCell::new(file)) }))
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
            Ok(PyObjectRef::new(PyObject::List(rev)))
        }
        PyObject::Tuple(v) => {
            let mut rev = v.clone(); rev.reverse();
            Ok(PyObjectRef::imm(PyObject::Tuple(rev)))
        }
        PyObject::Str(s) => Ok(py_str(&s.chars().rev().collect::<String>())),
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
            Ok(PyObjectRef::new(PyObject::List(v)))
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
        println!("For information about a specific object, type help(object)");
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

pub fn builtin_eval(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("eval() requires at least 1 argument"));
    }
    let source = args[0].str();
    let mut parser = crate::parser::Parser::new(&source);
    let program = parser.parse_program().map_err(|e| PyError::type_error(format!("eval parse error: {}", e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler.compile(&program, "<eval>").map_err(|e| PyError::type_error(format!("eval compile error: {}", e)))?;
    let mut vm = crate::vm::VirtualMachine::new();
    vm.run(code).map_err(|e| PyError::type_error(format!("eval error: {}", e)))
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
    let mut vm = crate::vm::VirtualMachine::new();
    vm.run(*code).map_err(|e| PyError::type_error(format!("exec error: {}", e)))?;
    Ok(py_none())
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
                let mut frame = super::vm::Frame::new(std::rc::Rc::new(code.clone()), g.clone(), std::rc::Rc::new(create_builtins()));
                for i in 0..npos.min(named_params) {
                    if i < code.varnames.len() {
                        frame.fast_locals[i] = Some(a[i].clone());
                        frame.locals.insert(code.varnames[i].clone(), a[i].clone());
                    }
                }
                if let Some(vararg_name) = &code.vararg_name {
                    let mut extra = Vec::new();
                    for i in named_params..npos {
                        extra.push(a[i].clone());
                    }
                    frame.locals.insert(vararg_name.clone(), py_tuple(extra));
                }
                if npos < named_params {
                    let num_defaults = code.num_defaults;
                    for i in npos..named_params {
                        let default_idx = num_defaults.saturating_sub(named_params - i);
                        if default_idx < defaults.len() {
                            frame.locals.insert(code.varnames[i].clone(), defaults[default_idx].clone());
                        }
                    }
                }
                if let Some(kwarg_name) = &code.kwarg_name {
                    if !frame.locals.contains_key(kwarg_name) {
                        frame.locals.insert(kwarg_name.clone(), py_dict());
                    }
                }
                let mut vm = super::vm::VirtualMachine::new();
                vm.frames.push(frame);
                vm.execute()
            } else { unreachable!() }
        }
        3 => {
            let bf = {
                let obj = f.borrow();
                if let PyObject::BoundMethod { func: bf, .. } = &*obj {
                    bf.clone()
                } else { return Err(PyError::type_error("not a bound method")); }
            };
            let mut all_args = vec![];
            all_args.extend(a);
            builtin_call(&bf, &all_args)
        }
        4 => {
            if let PyObject::Type { dict: type_dict, .. } = &*f.borrow() {
                let instance = PyObjectRef::new(PyObject::Instance {
                    typ: f.clone(),
                    dict: std::collections::HashMap::new(),
                });
                if let Some(init) = type_dict.get("__init__").cloned() {
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

// ---- Attribute access ----

pub trait ObjectAccess {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef>;
    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()>;
    fn del_attribute(&mut self, name: &str) -> PyResult<()>;
}

impl ObjectAccess for PyObject {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef> {
        match self {
            PyObject::Module { dict, .. } => {
                if name == "__dict__" {
                    // Convert module's HashMap to a PyDict
                    use crate::object::ObjectAccess;
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        let _ = pd.set(py_str(k), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(pd)));
                }
                dict.get(name).cloned().ok_or_else(|| PyError::attribute_error(format!("module has no attribute '{}'", name)))
            }
            PyObject::Type { dict, mro, bases, name: type_name } => {
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
                dict.get(name).cloned().or_else(|| {
                    for base in mro.iter().skip(1) {
                        if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                            if let Some(val) = base_dict.get(name) {
                                return Some(val.clone());
                            }
                        }
                    }
                    None
                }).ok_or_else(|| PyError::attribute_error(format!("type has no attribute '{}'", name)))
            }
            PyObject::Instance { dict, typ } => {
                dict.get(name).cloned().or_else(|| {
                    let typ_ref = typ.borrow();
                    if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                        type_dict.get(name).cloned().or_else(|| {
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if let Some(val) = base_dict.get(name) {
                                        return Some(val.clone());
                                    }
                                }
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
                                let (k, v) = items.into_iter().next().unwrap();
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
                                let other = args[1].borrow();
                                if let PyObject::Set(other_set) = &*other {
                                    Ok(py_bool(set.to_vec().iter().all(|item| other_set.contains(item).unwrap_or(false))))
                                } else { Err(PyError::type_error("issubset() argument must be a set")) }
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
                                let other = args[1].borrow();
                                if let PyObject::Set(other_set) = &*other {
                                    Ok(py_bool(other_set.to_vec().iter().all(|item| set.contains(item).unwrap_or(false))))
                                } else { Err(PyError::type_error("issuperset() argument must be a set")) }
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
                    "__name__" | "__qualname__" => Ok(py_str(func_name)),
                    _ => dict.get(name).cloned().ok_or_else(|| PyError::attribute_error(format!(
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
                                    // For send(), push the sent value onto the stack
                                    if args.len() > 1 && args[1].borrow().type_name() != "function" {
                                        f.stack.push(args[1].clone());
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
                                                Err(crate::object::PyError::StopIteration)
                                            }
                                        }
                                        Err(e) => {
                                            *frame_opt = None;
                                            if matches!(&e, crate::object::PyError::StopIteration) {
                                                return Err(e);
                                            }
                                            Err(e)
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
                                                Err(crate::object::PyError::StopIteration)
                                            }
                                        }
                                        Err(e) => {
                                            *frame_opt = None;
                                            if matches!(&e, crate::object::PyError::StopIteration) {
                                                return Err(e);
                                            }
                                            Err(e)
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
            PyObject::File { file: _ } => {
                match name {
                    "read" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "read".to_string(),
                        func: |args| {
                            use std::io::Read;
                            if let PyObject::File { file } = &*args[0].borrow() {
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
                            if let PyObject::File { file } = &*args[0].borrow() {
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
                            if let PyObject::File { file } = &mut *args[0].borrow_mut() {
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
                            if let PyObject::File { file } = &*args[file_obj_idx].borrow() {
                                let _ = file.borrow().sync_all();
                            }
                            // Replace with /dev/null to close the actual file descriptor
                            if let PyObject::File { file } = &mut *args[file_obj_idx].borrow_mut() {
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
                                let result = locked.result.clone();
                                let call_result = crate::object::builtin_call(&target, &thread_args);
                                match call_result {
                                    Ok(val) => {
                                        *result.lock().unwrap() = Some(val);
                                    }
                                    Err(_) => {}
                                }
                                locked.handle = Some(std::thread::spawn(|| {}));
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
            PyObject::Exception { typ, args, .. } => {
                match name {
                    "__name__" => Ok(py_str(typ)),
                    "args" => Ok(py_tuple(args.clone())),
                    _ => Err(PyError::attribute_error(format!("'Exception' object has no attribute '{}'", name))),
                }
            }
            _ => Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", self.type_name(), name))),
        }
    }

    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, .. } => {
                dict.insert(name.to_string(), value);
                Ok(())
            }
            PyObject::Module { dict, .. } => {
                dict.insert(name.to_string(), value);
                Ok(())
            }
            PyObject::Type { dict, .. } => {
                dict.insert(name.to_string(), value);
                Ok(())
            }
            PyObject::Function { dict, .. } => {
                dict.insert(name.to_string(), value);
                Ok(())
            }
            _ => Err(PyError::attribute_error(format!("cannot set attribute '{}' on '{}'", name, self.type_name()))),
        }
    }

    fn del_attribute(&mut self, name: &str) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, .. } => {
                dict.remove(name).ok_or_else(|| PyError::attribute_error(format!("'{}' object has no attribute '{}'", self.type_name(), name)))?;
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
            _ => Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", self.type_name(), name))),
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
                        PyObject::Type { dict: type_dict, .. } => type_dict.get("__index__").cloned(),
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
    // Check for __getitem__ on custom classes
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__getitem__").cloned(),
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
                _ => Err(PyError::type_error("tuple indices must be integers or slices")),
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
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__setitem__").cloned(),
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
make_exception_func!(builtin_make_exception_connectionerror, "ConnectionError");
make_exception_func!(builtin_make_exception_brokenpipeerror, "BrokenPipeError");
make_exception_func!(builtin_make_exception_connectionrefusederror, "ConnectionRefusedError");
make_exception_func!(builtin_make_exception_blockingioerror, "BlockingIOError");
make_exception_func!(builtin_make_exception_childprocesserror, "ChildProcessError");
make_exception_func!(builtin_make_exception_interruptederror, "InterruptedError");
make_exception_func!(builtin_make_exception_timeouterror, "TimeoutError");
make_exception_func!(builtin_make_exception_unicodedecodeerror, "UnicodeDecodeError");
make_exception_func!(builtin_make_exception_unicodeencodeerror, "UnicodeEncodeError");

// ---- Module creation ----

pub fn create_builtins() -> HashMap<String, PyObjectRef> {
    let mut builtins = HashMap::new();
    builtins.insert("None".to_string(), py_none());
    builtins.insert("True".to_string(), py_bool(true));
    builtins.insert("False".to_string(), py_bool(false));
    builtins.insert("Ellipsis".to_string(), PyObjectRef::imm(PyObject::Str("...".to_string())));

    macro_rules! add_func {
        ($name:expr, $func:expr) => {
            builtins.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
        };
    }

    add_func!("print", builtin_print);
    add_func!("len", builtin_len);
    add_func!("range", builtin_range);
    add_func!("type", builtin_type_of);
    add_func!("int", builtin_int);
    add_func!("float", builtin_float);
    add_func!("str", builtin_str);
    add_func!("bool", builtin_bool);
    add_func!("list", builtin_list);
    add_func!("tuple", builtin_tuple);
    add_func!("dict", builtin_dict);
    add_func!("set", builtin_set);
    add_func!("abs", builtin_abs);
    add_func!("hasattr", builtin_hasattr);
    add_func!("getattr", builtin_getattr);
    add_func!("setattr", builtin_setattr);
    add_func!("delattr", builtin_delattr);
    add_func!("ord", builtin_ord);
    add_func!("chr", builtin_chr);
    add_func!("hex", builtin_hex);
    add_func!("oct", builtin_oct);
    add_func!("bin", builtin_bin);
    add_func!("ascii", builtin_ascii);
    add_func!("memoryview", builtin_memoryview);
    add_func!("input", builtin_input);
    add_func!("exit", builtin_exit);
    add_func!("repr", builtin_repr);
    add_func!("sorted", builtin_sorted);
    add_func!("enumerate", builtin_enumerate);
    add_func!("iter", builtin_iter);
    add_func!("next", builtin_next);
    add_func!("sum", builtin_sum);
    add_func!("max", builtin_max);
    add_func!("min", builtin_min);
    add_func!("id", builtin_id);
    add_func!("vars", builtin_vars);
    add_func!("isinstance", builtin_isinstance);
    add_func!("open", builtin_open);
    add_func!("any", builtin_any);
    add_func!("all", builtin_all);
    add_func!("callable", builtin_callable);
    add_func!("breakpoint", builtin_breakpoint);
    add_func!("pow", builtin_pow);
    add_func!("reversed", builtin_reversed);
    add_func!("issubclass", builtin_issubclass);
    add_func!("help", builtin_help);
    add_func!("eval", builtin_eval);
    add_func!("exec", builtin_exec);
    add_func!("compile", builtin_compile);
    add_func!("super", builtin_super);
    add_func!("map", builtin_map);
    add_func!("filter", builtin_filter);
    add_func!("zip", builtin_zip);
    add_func!("property", builtin_property);
    add_func!("staticmethod", builtin_staticmethod);
    add_func!("classmethod", builtin_classmethod);
    add_func!("bytes", builtin_bytes);
    add_func!("bytearray", builtin_bytearray);
    add_func!("frozenset", builtin_frozenset);
    add_func!("format", builtin_format);
    add_func!("object", builtin_object);
    add_func!("hash", builtin_hash);
    add_func!("slice", builtin_slice);
    add_func!("divmod", builtin_divmod);
    add_func!("round", builtin_round);
    add_func!("dir", builtin_dir);
    add_func!("globals", builtin_globals);
    add_func!("locals", builtin_locals);

    macro_rules! add_exc_type {
        ($name:expr, $func:expr) => {
            builtins.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
        };
    }

    add_exc_type!("BaseException", builtin_make_exception_baseexception);
    add_exc_type!("Exception", builtin_make_exception_exception);
    add_exc_type!("TypeError", builtin_make_exception_typeerror);
    add_exc_type!("ValueError", builtin_make_exception_valueerror);
    add_exc_type!("ZeroDivisionError", builtin_make_exception_zerodivisionerror);
    add_exc_type!("NameError", builtin_make_exception_nameerror);
    add_exc_type!("AttributeError", builtin_make_exception_attributeerror);
    add_exc_type!("IndexError", builtin_make_exception_indexerror);
    add_exc_type!("KeyError", builtin_make_exception_keyerror);
    add_exc_type!("RuntimeError", builtin_make_exception_runtimeerror);
    add_exc_type!("StopIteration", builtin_make_exception_stopiteration);
    add_exc_type!("AssertionError", builtin_make_exception_assertionerror);
    add_exc_type!("OSError", builtin_make_exception_oserror);
    add_exc_type!("ImportError", builtin_make_exception_importerror);
    add_exc_type!("LookupError", builtin_make_exception_lookuperror);
    add_exc_type!("ArithmeticError", builtin_make_exception_arithmeticerror);
    add_exc_type!("FloatingPointError", builtin_make_exception_floatingpointerror);
    add_exc_type!("OverflowError", builtin_make_exception_overflowerror);
    add_exc_type!("EnvironmentError", builtin_make_exception_environmenterror);
    add_exc_type!("IOError", builtin_make_exception_ioerror);
    add_exc_type!("FileNotFoundError", builtin_make_exception_filenotfounderror);
    add_exc_type!("PermissionError", builtin_make_exception_permissionerror);
    add_exc_type!("NotImplementedError", builtin_make_exception_notimplementederror);
    add_exc_type!("RecursionError", builtin_make_exception_recursionerror);
    add_exc_type!("KeyboardInterrupt", builtin_make_exception_keyboardinterrupt);
    add_exc_type!("GeneratorExit", builtin_make_exception_generatorexit);
    add_exc_type!("SystemExit", builtin_make_exception_systemexit);
    add_exc_type!("ModuleNotFoundError", builtin_make_exception_modulenotfounderror);
    add_exc_type!("StopAsyncIteration", builtin_make_exception_stopasynciteration);
    add_exc_type!("EOFError", builtin_make_exception_eoferror);
    add_exc_type!("ConnectionError", builtin_make_exception_connectionerror);
    add_exc_type!("BrokenPipeError", builtin_make_exception_brokenpipeerror);
    add_exc_type!("ConnectionRefusedError", builtin_make_exception_connectionrefusederror);
    add_exc_type!("BlockingIOError", builtin_make_exception_blockingioerror);
    add_exc_type!("ChildProcessError", builtin_make_exception_childprocesserror);
    add_exc_type!("InterruptedError", builtin_make_exception_interruptederror);
    add_exc_type!("TimeoutError", builtin_make_exception_timeouterror);
    add_exc_type!("UnicodeDecodeError", builtin_make_exception_unicodedecodeerror);
    add_exc_type!("UnicodeEncodeError", builtin_make_exception_unicodeencodeerror);

    let math_module = PyObjectRef::new(PyObject::Module {
        name: "math".to_string(),
        dict: create_math_dict(),
    });
    builtins.insert("math".to_string(), math_module.clone());

    builtins
}

pub fn create_math_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! math_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    math_func!("sqrt", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sqrt() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sqrt())), PyObject::Float(f) => Ok(py_float(f.sqrt())), _ => Err(PyError::type_error("sqrt() argument must be a number")) }
    });
    math_func!("sin", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sin() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sin())), PyObject::Float(f) => Ok(py_float(f.sin())), _ => Err(PyError::type_error("sin() argument must be a number")) }
    });
    math_func!("cos", |args| {
        if args.len() != 1 { return Err(PyError::type_error("cos() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).cos())), PyObject::Float(f) => Ok(py_float(f.cos())), _ => Err(PyError::type_error("cos() argument must be a number")) }
    });
    math_func!("tan", |args| {
        if args.len() != 1 { return Err(PyError::type_error("tan() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).tan())), PyObject::Float(f) => Ok(py_float(f.tan())), _ => Err(PyError::type_error("tan() argument must be a number")) }
    });
    math_func!("floor", |args| {
        if args.len() != 1 { return Err(PyError::type_error("floor() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_int(i.clone())), PyObject::Float(f) => Ok(py_int(f.floor() as i64)), _ => Err(PyError::type_error("floor() argument must be a number")) }
    });
    math_func!("ceil", |args| {
        if args.len() != 1 { return Err(PyError::type_error("ceil() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_int(i.clone())), PyObject::Float(f) => Ok(py_int(f.ceil() as i64)), _ => Err(PyError::type_error("ceil() argument must be a number")) }
    });
    math_func!("exp", |args| {
        if args.len() != 1 { return Err(PyError::type_error("exp() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).exp())), PyObject::Float(f) => Ok(py_float(f.exp())), _ => Err(PyError::type_error("exp() argument must be a number")) }
    });
    math_func!("pow", |args| {
        if args.len() != 2 { return Err(PyError::type_error("pow() takes exactly two arguments")); }
        let a = args[0].borrow();
        let b = args[1].borrow();
        match (&*a, &*b) {
            (PyObject::Int(i), PyObject::Int(j)) => Ok(py_float(i.to_f64().unwrap_or(0.0).powf(j.to_f64().unwrap_or(0.0)))),
            (PyObject::Int(i), PyObject::Float(f)) => Ok(py_float(i.to_f64().unwrap_or(0.0).powf(*f))),
            (PyObject::Float(f), PyObject::Int(i)) => Ok(py_float(f.powf(i.to_f64().unwrap_or(0.0)))),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a.powf(*b))),
            _ => Err(PyError::type_error("pow() argument must be a number")),
        }
    });
    math_func!("log", |args| {
        if args.len() < 1 || args.len() > 2 { return Err(PyError::type_error("log() takes one or two arguments")); }
        let v = args[0].borrow();
        let x = match &*v { PyObject::Int(i) => i.to_f64().unwrap_or(0.0), PyObject::Float(f) => *f, _ => return Err(PyError::type_error("log() argument must be a number")) };
        let base = if args.len() == 2 {
            let b = args[1].borrow();
            match &*b { PyObject::Int(i) => i.to_f64().unwrap_or(0.0), PyObject::Float(f) => *f, _ => return Err(PyError::type_error("log() base must be a number")) }
        } else {
            std::f64::consts::E
        };
        Ok(py_float(x.log(base)))
    });
    math_func!("abs", |args| {
        if args.len() != 1 { return Err(PyError::type_error("abs() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).abs())), PyObject::Float(f) => Ok(py_float(f.abs())), _ => Err(PyError::type_error("abs() argument must be a number")) }
    });
    d.insert("pi".to_string(), py_float(std::f64::consts::PI));
    d.insert("e".to_string(), py_float(std::f64::consts::E));
    d
}

pub fn create_sys_dict(argv: Vec<String>) -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sys_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    sys_func!("exit", |args| {
        let code = if args.len() > 0 {
            match &*args[0].borrow() {
                PyObject::Int(i) => i.to_i64().unwrap_or(0) as i32,
                _ => 1,
            }
        } else { 0 };
        Err(PyError::SystemExit(code))
    });
    d.insert("argv".to_string(), py_list(argv.into_iter().map(|s| py_str(&s)).collect()));
    d.insert("path".to_string(), py_list(vec![]));
    d.insert("modules".to_string(), py_dict());
    d.insert("version".to_string(), py_str("3.12.0 (RustPython 0.1.0)"));
    d.insert("stdin".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::open("/dev/stdin").unwrap_or_else(|_| {
            // Fallback: create a temporary file
            std::fs::File::create("/dev/null").unwrap()
        }))),
    }));
    d.insert("stdout".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::create("/dev/stdout").unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
    }));
    d.insert("stderr".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::create("/dev/stderr").unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
    }));
    d.insert("platform".to_string(), py_str(std::env::consts::OS));
    d.insert("implementation".to_string(), py_str("RustPython"));
    d.insert("byteorder".to_string(), py_str(if cfg!(target_endian = "little") { "little" } else { "big" }));
    d.insert("maxsize".to_string(), py_int(i64::MAX));
    d.insert("maxunicode".to_string(), py_int(1114111));
    d.insert("api_version".to_string(), py_int(1013));
    d.insert("executable".to_string(), py_str(&std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
    d.insert("prefix".to_string(), py_str("/usr"));
    d.insert("exec_prefix".to_string(), py_str("/usr"));
    d.insert("winver".to_string(), py_str("3.12"));
    d
}

pub fn create_os_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! os_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    os_func!("listdir", |args| {
        let path = if args.len() > 0 { args[0].str() } else { ".".to_string() };
        match std::fs::read_dir(&path) {
            Ok(entries) => {
                let names: Vec<PyObjectRef> = entries.filter_map(|e| e.ok()).map(|e| py_str(&e.file_name().to_string_lossy())).collect();
                Ok(py_list(names))
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("mkdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("mkdir() takes at least 1 argument")); }
        match std::fs::create_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("remove", |args| {
        if args.is_empty() { return Err(PyError::type_error("remove() takes at least 1 argument")); }
        match std::fs::remove_file(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("rename", |args| {
        if args.len() < 2 { return Err(PyError::type_error("rename() takes 2 arguments")); }
        match std::fs::rename(&args[0].str(), &args[1].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("system", |args| {
        if args.is_empty() { return Err(PyError::type_error("system() takes at least 1 argument")); }
        let cmd = args[0].str();
        match std::process::Command::new("sh").arg("-c").arg(&cmd).status() {
            Ok(status) => Ok(py_int(status.code().unwrap_or(0) as i64)),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("chdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("chdir() takes at least 1 argument")); }
        match std::env::set_current_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("getcwd", |_| {
        match std::env::current_dir() {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("getenv", |args| {
        if args.is_empty() { return Ok(py_none()); }
        let key = args[0].str();
        match std::env::var(&key) {
            Ok(val) => Ok(py_str(&val)),
            Err(_) => {
                if args.len() > 1 { Ok(args[1].clone()) }
                else { Ok(py_none()) }
            }
        }
    });

    os_func!("putenv", |args| {
        if args.len() < 2 { return Err(PyError::type_error("putenv() takes exactly 2 arguments")); }
        std::env::set_var(args[0].str(), args[1].str());
        Ok(py_none())
    });

    os_func!("unsetenv", |args| {
        if args.is_empty() { return Err(PyError::type_error("unsetenv() takes at least 1 argument")); }
        std::env::remove_var(args[0].str());
        Ok(py_none())
    });

    // File descriptor operations
    os_func!("open", |args| {
        if args.len() < 2 { return Err(PyError::type_error("open() requires at least 2 arguments")); }
        let path = args[0].str();
        let flags = args[1].as_i64().unwrap_or(0) as i32;
        let mut opts = std::fs::OpenOptions::new();
        // O_RDONLY=0, O_WRONLY=1, O_RDWR=2 — check access mode
        let access_mode = flags & 3;
        if access_mode == 0 { opts.read(true); }     // O_RDONLY
        if access_mode == 1 { opts.write(true); }    // O_WRONLY
        if access_mode == 2 { opts.write(true); opts.read(true); } // O_RDWR
        if flags & 64 != 0 { opts.create(true); }       // O_CREAT = 64
        if flags & 512 != 0 { opts.truncate(true); }    // O_TRUNC = 512
        if flags & 1024 != 0 { opts.append(true); }     // O_APPEND = 1024
        match opts.open(&path) {
            Ok(file) => {
                use std::os::unix::io::IntoRawFd;
                Ok(py_int(file.into_raw_fd() as i64))
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("read", |args| {
        if args.len() < 2 { return Err(PyError::type_error("read() requires at least 2 arguments")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let n = args[1].as_i64().unwrap_or(0) as usize;
        use std::os::unix::io::FromRawFd;
        let mut buf = vec![0u8; n];
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        use std::io::Read;
        match file.read(&mut buf) {
            Ok(count) => {
                buf.truncate(count);
                std::mem::forget(file); // Don't close the fd
                Ok(PyObjectRef::new(PyObject::Bytes(buf)))
            }
            Err(e) => {
                std::mem::forget(file);
                Err(PyError::OsError(format!("{}", e)))
            }
        }
    });
    os_func!("write", |args| {
        if args.len() < 2 { return Err(PyError::type_error("write() requires at least 2 arguments")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let data = match &*args[1].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("write() argument 2 must be bytes or str")),
        };
        use std::os::unix::io::FromRawFd;
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        use std::io::Write;
        match file.write(&data) {
            Ok(count) => {
                std::mem::forget(file);
                Ok(py_int(count as i64))
            }
            Err(e) => {
                std::mem::forget(file);
                Err(PyError::OsError(format!("{}", e)))
            }
        }
    });
    os_func!("close", |args| {
        if args.is_empty() { return Err(PyError::type_error("close() requires at least 1 argument")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        use std::os::unix::io::FromRawFd;
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        drop(file); // Closes the fd
        Ok(py_none())
    });

    // OS flags for open()
    d.insert("O_RDONLY".to_string(), py_int(0));
    d.insert("O_WRONLY".to_string(), py_int(1));
    d.insert("O_RDWR".to_string(), py_int(2));
    d.insert("O_CREAT".to_string(), py_int(64));
    d.insert("O_TRUNC".to_string(), py_int(512));
    d.insert("O_APPEND".to_string(), py_int(1024));

    // environ dict
    let mut environ_dict = HashMap::new();
    for (key, val) in std::env::vars() {
        environ_dict.insert(key, py_str(&val));
    }
    d.insert("environ".to_string(), create_module("environ", environ_dict));

    // os.path sub-module
    let mut path_dict = HashMap::new();
    macro_rules! path_func {
        ($name:expr, $func:expr) => {
            path_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    path_func!("join", |args| {
        let parts: Vec<String> = args.iter().map(|a| a.str()).collect();
        if parts.is_empty() { return Ok(py_str("")); }
        let result = parts.join("/");
        Ok(py_str(&result))
    });
    path_func!("dirname", |args| {
        if args.is_empty() { return Err(PyError::type_error("dirname() takes at least 1 argument")); }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => if i == 0 { "/".to_string() } else { path[..i].to_string() },
            None => ".".to_string(),
        };
        Ok(py_str(&result))
    });
    path_func!("basename", |args| {
        if args.is_empty() { return Err(PyError::type_error("basename() takes at least 1 argument")); }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => path[i+1..].to_string(),
            None => path,
        };
        Ok(py_str(&result))
    });
    path_func!("split", |args| {
        if args.is_empty() { return Err(PyError::type_error("split() takes at least 1 argument")); }
        let path = args[0].str();
        let (head, tail) = match path.rfind('/') {
            Some(i) => {
                let h = if i == 0 { "/".to_string() } else { path[..i].to_string() };
                let t = path[i+1..].to_string();
                (h, t)
            }
            None => (".".to_string(), path.clone()),
        };
        Ok(py_list(vec![py_str(&head), py_str(&tail)]))
    });
    path_func!("splitext", |args| {
        if args.is_empty() { return Err(PyError::type_error("splitext() takes at least 1 argument")); }
        let path = args[0].str();
        let dot = path.rfind('.');
        let (root, ext) = match dot {
            Some(i) if i > path.rfind('/').map_or(0, |j| j + 1) => {
                (path[..i].to_string(), path[i..].to_string())
            }
            _ => (path.clone(), "".to_string()),
        };
        Ok(py_list(vec![py_str(&root), py_str(&ext)]))
    });
    path_func!("exists", |args| {
        if args.is_empty() { return Err(PyError::type_error("exists() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).exists()))
    });
    path_func!("isfile", |args| {
        if args.is_empty() { return Err(PyError::type_error("isfile() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).is_file()))
    });
    path_func!("isdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("isdir() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).is_dir()))
    });
    d.insert("path".to_string(), create_module("path", path_dict));
    d
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

fn json_decode(s: &str) -> PyResult<PyObjectRef> {
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

fn json_encode_full(val: &PyObjectRef, indent: Option<usize>, sort_keys: bool, level: usize) -> PyResult<PyObjectRef> {
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

pub fn create_json_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! json_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    json_func!("dumps", |args| {
        if args.is_empty() { return Err(PyError::type_error("dumps() missing required argument")); }
        let indent = if args.len() > 1 {
            let v = args[1].as_i64().unwrap_or(-1);
            if v >= 0 { Some(v as usize) } else { None }
        } else { None };
        let sort_keys = if args.len() > 2 { args[2].truthy() } else { false };
        json_encode_full(&args[0], indent, sort_keys, 0)
    });

    json_func!("loads", |args| {
        if args.is_empty() { return Err(PyError::type_error("loads() missing required argument")); }
        let s = args[0].str();
        json_decode(&s)
    });

    d
}

pub fn create_collections_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! coll_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // deque: double-ended queue
    coll_func!("deque", |args| {
        let iterable = if args.len() > 0 { Some(args[0].clone()) } else { None };
        let mut deque = std::collections::VecDeque::new();
        if let Some(iter) = iterable {
            // Iterate over the iterable and add items
            if let Ok(it) = crate::object::builtin_iter(&[iter]) {
                loop {
                    match crate::object::builtin_next(&[it.clone()]) {
                        Ok(v) => deque.push_back(v),
                        Err(crate::object::PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(PyObjectRef::new(PyObject::List(deque.into_iter().collect())))
    });

    // Counter: count hashable objects
    coll_func!("Counter", |args| {
        if args.is_empty() {
            return Ok(crate::object::py_dict());
        }
        let iterable = &args[0];
        let mut counts = std::collections::HashMap::<usize, (PyObjectRef, i64)>::new();
        let mut order = Vec::new();
        if let Ok(it) = crate::object::builtin_iter(&[iterable.clone()]) {
            loop {
                match crate::object::builtin_next(&[it.clone()]) {
                    Ok(item) => {
                        let hash = item.hash()?;
                        let entry = counts.entry(hash).or_insert_with(|| {
                            order.push(hash);
                            (item.clone(), 0)
                        });
                        entry.1 += 1;
                    }
                    Err(crate::object::PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let dict = crate::object::py_dict();
        for hash in &order {
            if let Some((item, count)) = counts.get(hash) {
                let count_val = crate::object::py_int(*count);
                if let crate::object::PyObject::Dict(d) = &mut *dict.borrow_mut() {
                    d.set(item.clone(), count_val)?;
                }
            }
        }
        Ok(dict)
    });

    d
}

pub fn call_function(func: &PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
    let f = func.borrow();
    match &*f {
        PyObject::BuiltinFunction { func: bf, .. } => {
            return bf(&args);
        }
        _ => {}
    }
    drop(f);
    Err(PyError::type_error(format!("'{}' object is not callable", func.borrow().type_name())))
}

pub fn create_functools_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ft_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    ft_func!("reduce", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("reduce() takes at least 2 arguments"));
        }
        let func = args[0].clone();
        let iterable = &args[1];
        let it = builtin_iter(&[iterable.clone()])?;
        let mut acc = match builtin_next(&[it.clone()]) {
            Ok(v) => v,
            Err(PyError::StopIteration) => {
                if args.len() >= 3 { return Ok(args[2].clone()); }
                return Err(PyError::type_error("reduce() of empty sequence with no initial value"));
            }
            Err(e) => return Err(e),
        };
        loop {
            match builtin_next(&[it.clone()]) {
                Ok(v) => {
                    let result = builtin_call(&func, &[acc, v])?;
                    acc = result;
                }
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(acc)
    });

    ft_func!("partial", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("partial() takes at least 1 argument"));
        }
        let func = args[0].clone();
        let partial_args: Vec<PyObjectRef> = args[1..].to_vec();
        Ok(PyObjectRef::new(PyObject::Partial { func, args: partial_args }))
    });

    d
}

pub fn create_itertools_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! it_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    it_func!("chain", |args| {
        let mut items = Vec::new();
        for arg in args {
            if let Ok(it) = builtin_iter(&[arg.clone()]) {
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(v) => items.push(v),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(py_list(items))
    });

    it_func!("count", |args| {
        let start = if args.len() > 0 {
            if let Some(n) = args[0].as_i64() { n } else { 0i64 }
        } else { 0i64 };
        let step = if args.len() > 1 {
            if let Some(n) = args[1].as_i64() { n } else { 1i64 }
        } else { 1i64 };
        let mut current = start;
        let mut items = Vec::new();
        for _ in 0..10000 {
            items.push(py_int(current));
            current += step;
        }
        Ok(py_list(items))
    });

    it_func!("product", |args| {
        if args.is_empty() {
            return Ok(py_list(vec![py_tuple(vec![])]));
        }
        let mut pools: Vec<Vec<PyObjectRef>> = Vec::new();
        for arg in args {
            let mut pool = Vec::new();
            if let Ok(it) = builtin_iter(&[arg.clone()]) {
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(v) => pool.push(v),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            pools.push(pool);
        }
        let mut result = vec![vec![]];
        for pool in &pools {
            let mut new_result = Vec::new();
            for prefix in &result {
                for item in pool {
                    let mut new_prefix = prefix.clone();
                    new_prefix.push(item.clone());
                    new_result.push(new_prefix);
                }
            }
            result = new_result;
        }
        Ok(py_list(result.into_iter().map(|v| py_tuple(v)).collect()))
    });

    d
}

static RNG_STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn fast_random_u64() -> u64 {
    RNG_STATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

fn fast_random_f64() -> f64 {
    (fast_random_u64() >> 11) as f64 * (1.0 / 9007199254740992.0)
}

pub fn create_random_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! rnd_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    rnd_func!("random", |args| {
        Ok(py_float(fast_random_f64()))
    });

    rnd_func!("randint", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("randint() takes at least 2 arguments"));
        }
        let a = args[0].as_i64().ok_or_else(|| PyError::type_error("randint() argument must be int"))?;
        let b = args[1].as_i64().ok_or_else(|| PyError::type_error("randint() argument must be int"))?;
        if a > b {
            return Err(PyError::ValueError("randint() empty range".to_string()));
        }
        let range = (b - a + 1) as u64;
        let n = fast_random_u64() % range;
        Ok(py_int(a + n as i64))
    });

    rnd_func!("choice", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("choice() takes at least 1 argument"));
        }
        let seq = &args[0];
        let seq_borrowed = seq.borrow();
        let len = match &*seq_borrowed {
            PyObject::List(v) => v.len(),
            PyObject::Tuple(v) => v.len(),
            PyObject::Str(s) => s.len(),
            _ => return Err(PyError::type_error("choice() argument must be a sequence")),
        };
        if len == 0 {
            return Err(PyError::IndexError("cannot choose from an empty sequence".to_string()));
        }
        let idx = (fast_random_u64() % len as u64) as usize;
        let val = match &*seq_borrowed {
            PyObject::List(v) => v[idx].clone(),
            PyObject::Tuple(v) => v[idx].clone(),
            PyObject::Str(s) => py_str(&s[idx..=idx]),
            _ => unreachable!(),
        };
        Ok(val)
    });

    rnd_func!("uniform", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("uniform() takes at least 2 arguments"));
        }
        let a = args[0].as_i64().unwrap_or(0) as f64;
        let b = args[1].as_i64().unwrap_or(1) as f64;
        Ok(py_float(a + (b - a) * fast_random_f64()))
    });

    rnd_func!("shuffle", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("shuffle() takes at least 1 argument"));
        }
        let seq = &args[0];
        let seq_borrowed = seq.borrow();
        if let PyObject::List(items) = &*seq_borrowed {
            let mut items = items.clone();
            drop(seq_borrowed);
            let len = items.len();
            for i in (1..len).rev() {
                let j = (fast_random_u64() % (i + 1) as u64) as usize;
                items.swap(i, j);
            }
            *seq.borrow_mut() = PyObject::List(items);
            Ok(py_none())
        } else {
            Err(PyError::type_error("shuffle() argument must be a list"))
        }
    });

    d
}

pub fn create_datetime_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dt_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    dt_func!("datetime", |args| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs() as i64;
        let nanos = now.subsec_nanos();
        // Format as ISO string
        let seconds = secs % 60;
        let minutes = (secs / 60) % 60;
        let hours = (secs / 3600) % 24;
        let days = secs / 86400;
        // Approximate year/month/day from days since epoch
        let mut y = 1970i64;
        let mut remaining = days;
        loop {
            let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
            if remaining < year_days { break; }
            remaining -= year_days;
            y += 1;
        }
        let is_leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let month_days = [31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut m = 1usize;
        for days_in_month in &month_days {
            if remaining < *days_in_month { break; }
            remaining -= days_in_month;
            m += 1;
        }
        let d = remaining + 1;
        let date_str = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, hours, minutes, seconds);
        Ok(py_str(&date_str))
    });

    dt_func!("date", |args| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs() as i64;
        let days = secs / 86400;
        let mut y = 1970i64;
        let mut remaining = days;
        loop {
            let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
            if remaining < year_days { break; }
            remaining -= year_days;
            y += 1;
        }
        let is_leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let month_days = [31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut m = 1usize;
        for days_in_month in &month_days {
            if remaining < *days_in_month { break; }
            remaining -= days_in_month;
            m += 1;
        }
        let d = remaining + 1;
        Ok(py_str(&format!("{:04}-{:02}-{:02}", y, m, d)))
    });

    dt_func!("now", |args| {
        let s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(py_float(s.as_secs_f64()))
    });

    d
}

fn socket_addr_to_string(addr: &PyObjectRef) -> PyResult<String> {
    let borrowed = addr.borrow();
    match &*borrowed {
        PyObject::Tuple(items) if items.len() == 2 => {
            let host = items[0].str();
            let port = items[1].as_i64().ok_or_else(|| PyError::type_error("port must be int"))?;
            Ok(format!("{}:{}", host, port))
        }
        PyObject::Str(s) => Ok(s.clone()),
        _ => {
            // Fallback: use str representation
            Ok(addr.str())
        }
    }
}

pub fn create_select_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sel_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    sel_func!("select", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("select() takes at least 3 arguments"));
        }
        let rlist = &args[0];
        let _wlist = &args[1];
        let _xlist = &args[2];
        let mut readable = Vec::new();
        let rlist_b = rlist.borrow();
        if let PyObject::List(items) = &*rlist_b {
            for item in items {
                readable.push(item.clone());
            }
        }
        Ok(py_tuple(vec![py_list(readable), py_list(vec![]), py_list(vec![])]))
    });

    d
}

pub struct ThreadInner {
    pub handle: Option<std::thread::JoinHandle<()>>,
    pub result: std::sync::Arc<std::sync::Mutex<Option<PyObjectRef>>>,
    pub target: PyObjectRef,
    pub args: Vec<PyObjectRef>,
}

pub struct LockInner {
    pub lock: std::sync::atomic::AtomicBool,
}

pub fn create_socket_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sock_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    sock_func!("socket", |args| {
        let family = if args.len() > 0 { args[0].as_i64().unwrap_or(2) } else { 2 };
        let _sock_type = if args.len() > 1 { args[1].as_i64().unwrap_or(1) } else { 1 };
        let _proto = if args.len() > 2 { args[2].as_i64().unwrap_or(0) } else { 0 };
        if family != 2 {
            return Err(PyError::runtime_error("Only AF_INET sockets are supported"));
        }
        Ok(PyObjectRef::new(PyObject::Socket {
            inner: std::rc::Rc::new(std::cell::RefCell::new(SocketInner::Uninitialized)),
        }))
    });

    d.insert("AF_INET".to_string(), py_int(2));
    d.insert("SOCK_STREAM".to_string(), py_int(1));
    d.insert("SOCK_DGRAM".to_string(), py_int(2));
    d.insert("SOL_SOCKET".to_string(), py_int(1));
    d.insert("SO_REUSEADDR".to_string(), py_int(2));

    d
}

pub fn create_re_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! re_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    re_func!("search", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("search() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                match re.find(&string) {
                    Some(m) => {
                        let start = m.start();
                        let end = m.end();
                        let text = m.as_str().to_string();
                        Ok(py_tuple(vec![py_int(start as i64), py_int(end as i64), py_str(&text)]))
                    }
                    None => Ok(py_none()),
                }
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("match", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("match() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                match re.find_at(&string, 0) {
                    Some(m) if m.start() == 0 => {
                        let end = m.end();
                        let text = m.as_str().to_string();
                        Ok(py_tuple(vec![py_int(0), py_int(end as i64), py_str(&text)]))
                    }
                    _ => Ok(py_none()),
                }
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                let results: Vec<PyObjectRef> = re.find_iter(&string)
                    .map(|m| py_str(m.as_str()))
                    .collect();
                Ok(py_list(results))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("sub", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("sub() takes at least 3 arguments"));
        }
        let pattern = args[0].str();
        let repl = args[1].str();
        let string = args[2].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                let result = re.replace_all(&string, repl.as_str());
                Ok(py_str(&result))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("split", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("split() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        let limit = if args.len() > 2 { args[2].as_i64().unwrap_or(0) as usize } else { 0 };
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                let parts: Vec<PyObjectRef> = if limit > 0 {
                    re.splitn(&string, limit).map(|s| py_str(s)).collect()
                } else {
                    re.split(&string).map(|s| py_str(s)).collect()
                };
                Ok(py_list(parts))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    d
}

pub fn create_subprocess_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sub_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    sub_func!("run", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("run() missing required argument"));
        }
        let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
            items.iter().map(|a| a.str()).collect()
        } else {
            vec![args[0].str()]
        };
        if cmd_args.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let output = std::process::Command::new(&cmd_args[0])
            .args(&cmd_args[1..])
            .output()
            .map_err(|e| PyError::OsError(format!("{}", e)))?;
        let returncode = output.status.code().unwrap_or(-1) as i64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok(py_dict())
    });

    sub_func!("check_output", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("check_output() missing required argument"));
        }
        let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
            items.iter().map(|a| a.str()).collect()
        } else {
            vec![args[0].str()]
        };
        if cmd_args.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let output = std::process::Command::new(&cmd_args[0])
            .args(&cmd_args[1..])
            .output()
            .map_err(|e| PyError::OsError(format!("{}", e)))?;
        if !output.status.success() {
            return Err(PyError::runtime_error(format!("Command returned non-zero exit status")));
        }
        Ok(py_str(&String::from_utf8_lossy(&output.stdout)))
    });

    // Constants
    d.insert("PIPE".to_string(), py_int(-1));

    d
}

pub fn create_threading_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! thr_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    thr_func!("Thread", |args| {
        let target = if args.len() > 0 { args[0].clone() } else { py_none() };
        let thread_args = if args.len() > 1 {
            if let PyObject::Tuple(items) = &*args[1].borrow() {
                items.clone()
            } else { vec![] }
        } else { vec![] };
        let inner = std::sync::Arc::new(std::sync::Mutex::new(ThreadInner {
            handle: None,
            result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            target,
            args: thread_args,
        }));
        Ok(PyObjectRef::new(PyObject::Thread(inner)))
    });

    thr_func!("Lock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(LockInner {
            lock: std::sync::atomic::AtomicBool::new(false),
        }));
        Ok(PyObjectRef::new(PyObject::Lock(inner)))
    });

    thr_func!("current_thread", |_| {
        Ok(py_str("MainThread"))
    });

    thr_func!("active_count", |_| {
        Ok(py_int(1))
    });

    d
}

/// Creates a callable that returns a dict-like object for WeakValueDictionary
pub fn create_weakref_weak_val_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakValueDictionary".to_string(),
        func: |args| {
            if args.len() > 0 {
                // Copy items from the argument
                if let PyObject::Dict(items) = &*args[0].borrow() {
                    let mut new_dict = PyDict::new();
                    for (k, v) in items.items() {
                        let _ = new_dict.set(k, v);
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(new_dict)));
                }
            }
            Ok(py_dict())
        },
    })
}

/// Creates a callable that returns a dict-like object for WeakKeyDictionary
pub fn create_weakref_weak_key_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakKeyDictionary".to_string(),
        func: |args| {
            if args.len() > 0 {
                if let PyObject::Dict(items) = &*args[0].borrow() {
                    let mut new_dict = PyDict::new();
                    for (k, v) in items.items() {
                        let _ = new_dict.set(k, v);
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(new_dict)));
                }
            }
            Ok(py_dict())
        },
    })
}

/// Creates a callable that returns a set-like object for WeakSet
pub fn create_weakref_weak_set() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakSet".to_string(),
        func: |args| {
            if args.len() > 0 {
                if let PyObject::Set(s) = &*args[0].borrow() {
                    return Ok(args[0].clone());
                }
                if let PyObject::List(items) = &*args[0].borrow() {
                    let mut s = PySet::new();
                    for item in items {
                        let _ = s.add(item.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Set(s)));
                }
            }
            Ok(PyObjectRef::new(PyObject::Set(PySet::new())))
        },
    })
}

pub fn create_module(name: &str, dict: HashMap<String, PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Module {
        name: name.to_string(),
        dict,
    })
}

/// Creates a minimal native copy module — shallow and deep copy for basic types.
pub fn create_copy_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! copy_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    copy_func!("copy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("copy() missing required argument"));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::None => Ok(py_none()),
            PyObject::Bool(b) => Ok(py_bool(*b)),
            PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) | PyObject::Bytes(_) => Ok(obj.clone()),
            PyObject::Tuple(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for item in items {
                    new_items.push(item.clone());
                }
                Ok(PyObjectRef::imm(PyObject::Tuple(new_items)))
            }
            PyObject::List(items) => {
                let new_items: Vec<PyObjectRef> = items.iter().map(|i| {
                    // Shallow copy: clone references
                    let b = i.borrow();
                    match &*b {
                        PyObject::None => py_none(),
                        PyObject::Bool(b) => py_bool(*b),
                        PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) => i.clone(),
                        _ => i.clone(),
                    }
                }).collect();
                Ok(py_list(new_items))
            }
            PyObject::Dict(dict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.items() {
                    let _ = new_dict.set(k, v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
            }
            PyObject::Set(s) => {
                let mut new_set = PySet::new();
                for item in s.to_vec() {
                    let _ = new_set.add(item);
                }
                Ok(PyObjectRef::new(PyObject::Set(new_set)))
            }
            _ => {
                // For instances and custom types, try __copy__
                if let Ok(copy_method) = borrowed.get_attribute("__copy__") {
                    drop(borrowed);
                    return crate::object::call_function(&copy_method, vec![obj.clone()]);
                }
                Ok(obj.clone())
            }
        }
    });

    copy_func!("deepcopy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("deepcopy() missing required argument"));
        }
        let obj = &args[0];
        let memo = if args.len() > 1 { args[1].clone() } else { py_dict() };
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::None => Ok(py_none()),
            PyObject::Bool(b) => Ok(py_bool(*b)),
            PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) | PyObject::Bytes(_) => Ok(obj.clone()),
            PyObject::Tuple(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for item in items {
                    new_items.push(deepcopy_one(item, &memo)?);
                }
                Ok(PyObjectRef::imm(PyObject::Tuple(new_items)))
            }
            PyObject::List(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for item in items {
                    new_items.push(deepcopy_one(item, &memo)?);
                }
                Ok(py_list(new_items))
            }
            PyObject::Dict(dict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.items() {
                    let new_k = deepcopy_one(&k, &memo)?;
                    let new_v = deepcopy_one(&v, &memo)?;
                    let _ = new_dict.set(new_k, new_v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
            }
            _ => {
                // For instances, try __deepcopy__ first
                if let Ok(dc_method) = borrowed.get_attribute("__deepcopy__") {
                    drop(borrowed);
                    return crate::object::call_function(&dc_method, vec![obj.clone(), memo]);
                }
                Ok(obj.clone())
            }
        }
    });

    // Error class
    d.insert("Error".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "Error".to_string(),
        func: |args| {
            let msg = if !args.is_empty() { args[0].str() } else { "copy error".to_string() };
            Err(PyError::Exception(msg, py_none()))
        },
    }));

    d
}

/// Helper: deep-copy a single object with memo support
fn deepcopy_one(obj: &PyObjectRef, memo: &PyObjectRef) -> Result<PyObjectRef, PyError> {
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

/// Creates the _weakref module — native replacement for the C extension.
/// Provides enough for CPython's weakref.py to load.
pub fn create_weakref_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! wr_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // ref(obj) returns a weak reference object (callable)
    // If the object is still alive, calling it returns the object
    // Since we don't have full GC, we use a simple Rc-based weak reference
    wr_func!("ref", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("ref() requires at least 1 argument"));
        }
        let obj = args[0].clone();
        // Return a BuiltinMethod that when called returns the original object
        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "weakref".to_string(),
            func: |args| Ok(args[0].clone()),
            self_obj: obj,
        }))
    });

    wr_func!("proxy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("proxy() requires at least 1 argument"));
        }
        Ok(args[0].clone())
    });

    wr_func!("getweakrefcount", |_| Ok(py_int(0)));
    wr_func!("getweakrefs", |_| Ok(py_list(vec![])));

    // Type constants
    d.insert("ReferenceType".to_string(), py_str("weakref"));
    d.insert("ProxyType".to_string(), py_str("weakproxy"));
    d.insert("CallableProxyType".to_string(), py_str("weakcallableproxy"));

    // Internal function used by weakrefset
    wr_func!("_remove_dead_weakref", |_| Ok(py_none()));

    d
}

/// Creates the _collections_abc module — native replacement for the C extension.
pub fn create_collections_abc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! abc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    abc_func!("__import__", |_| Ok(py_bool(true)));

    // Abstract base classes as simple markers
    let abc_meta = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ABCMeta".to_string(),
        func: |args| {
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_dict(), // simplified type
                dict: HashMap::new(),
            }))
        },
    });

    d.insert("ABCMeta".to_string(), abc_meta);
    d.insert("Hashable".to_string(), py_str("Hashable"));
    d.insert("Iterable".to_string(), py_str("Iterable"));
    d.insert("Iterator".to_string(), py_str("Iterator"));
    d.insert("Sized".to_string(), py_str("Sized"));
    d.insert("Callable".to_string(), py_str("Callable"));
    d.insert("Sequence".to_string(), py_str("Sequence"));
    d.insert("MutableSequence".to_string(), py_str("MutableSequence"));
    d.insert("Set".to_string(), py_str("Set"));
    d.insert("MutableSet".to_string(), py_str("MutableSet"));
    d.insert("Mapping".to_string(), py_str("Mapping"));
    d.insert("MutableMapping".to_string(), py_str("MutableMapping"));
    d.insert("MappingView".to_string(), py_str("MappingView"));
    d.insert("ItemsView".to_string(), py_str("ItemsView"));
    d.insert("KeysView".to_string(), py_str("KeysView"));
    d.insert("ValuesView".to_string(), py_str("ValuesView"));
    d.insert("Container".to_string(), py_str("Container"));

    d
}

/// Creates a minimal `types` module with commonly used types.
pub fn create_types_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! t_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    t_func!("FunctionType", |args| {
        if args.is_empty() { return Err(PyError::type_error("FunctionType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("LambdaType", |args| {
        if args.is_empty() { return Err(PyError::type_error("LambdaType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("MethodType", |args| {
        if args.is_empty() { return Err(PyError::type_error("MethodType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("BuiltinFunctionType", |args| {
        if args.is_empty() { return Err(PyError::type_error("BuiltinFunctionType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("ModuleType", |args| {
        if args.is_empty() { return Err(PyError::type_error("ModuleType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("NoneType", |_| Ok(py_none()));
    t_func!("GeneratorType", |args| {
        if args.is_empty() { return Err(PyError::type_error("GeneratorType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("CoroutineType", |args| {
        if args.is_empty() { return Err(PyError::type_error("CoroutineType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("SimpleNamespace", |args| {
        let d = py_dict();
        if args.len() > 0 {
            if let PyObject::Dict(items) = &*args[0].borrow() {
                let mut new_dict = PyDict::new();
                for (k, v) in items.items() {
                    let _ = new_dict.set(k, v);
                }
                return Ok(PyObjectRef::new(PyObject::Dict(new_dict)));
            }
        }
        Ok(d)
    });
    d.insert("CodeType".to_string(), py_str("code"));
    d.insert("MappingProxyType".to_string(), py_str("mappingproxy"));

    d
}

/// Creates a minimal `struct` module for binary packing/unpacking.
pub fn create_struct_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! s_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn fmt_size(fmt: &str) -> usize {
        let mut size = 0usize;
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            let count = match c {
                ' ' | '!' | '@' | '<' | '>' | '=' => continue,
                '0'..='9' => {
                    let mut n = String::from(c);
                    while let Some(d @ '0'..='9') = chars.peek() {
                        n.push(*d);
                        chars.next();
                    }
                    n.parse::<usize>().unwrap_or(1)
                }
                _ => 1,
            };
            size += match c {
                'c' | 'b' | 'B' | '?' | 'x' => 1 * count,
                'h' | 'H' => 2 * count,
                'i' | 'I' | 'l' | 'L' | 'f' => 4 * count,
                'q' | 'Q' | 'd' => 8 * count,
                's' | 'p' => count,
                _ => 1 * count,
            };
        }
        size
    }

    s_func!("calcsize", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("calcsize() missing required argument"));
        }
        let fmt = args[0].str();
        Ok(py_int(fmt_size(&fmt) as i64))
    });

    s_func!("pack", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("pack() requires format string and values"));
        }
        let fmt = args[0].str();
        let values: Vec<u8> = args[1..].iter().map(|arg| {
            if let Some(n) = arg.as_i64() { n as u8 } else { 0u8 }
        }).collect();
        let size = fmt_size(&fmt);
        let mut data = Vec::with_capacity(size);
        for v in &values { data.push(*v); }
        while data.len() < size { data.push(0); }
        Ok(PyObjectRef::imm(PyObject::Bytes(data)))
    });

    s_func!("unpack", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("unpack() requires format string and buffer"));
        }
        let fmt = args[0].str();
        let buf = {
            let b = args[1].borrow();
            match &*b {
                PyObject::Bytes(data) => data.clone(),
                _ => return Err(PyError::type_error("unpack() arg 2 must be bytes")),
            }
        };
        let size = fmt_size(&fmt);
        if buf.len() < size {
            return Err(PyError::type_error(format!("unpack() requires a buffer of {} bytes", size)));
        }
        let values: Vec<PyObjectRef> = buf[..size].iter().map(|&b| py_int(b as i64)).collect();
        Ok(PyObjectRef::imm(PyObject::Tuple(values)))
    });

    d.insert("error".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "error".to_string(),
        func: |args| {
            let msg = if !args.is_empty() { args[0].str() } else { "struct error".to_string() };
            Err(PyError::Exception(msg, py_none()))
        },
    }));

    d
}

/// Creates a native bisect module for binary search and insertion on lists.
pub fn create_bisect_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! bisect_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    bisect_func!("bisect_left", |args| {
        if args.len() < 2 { return Err(PyError::type_error("bisect_left() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("bisect_left() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if items[mid].borrow().lt(x)? { lo = mid + 1; } else { hi = mid; }
        }
        Ok(py_int(lo as i64))
    });

    bisect_func!("bisect_right", |args| {
        if args.len() < 2 { return Err(PyError::type_error("bisect_right() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("bisect_right() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if x.borrow().lt(&items[mid])? { hi = mid; } else { lo = mid + 1; }
        }
        Ok(py_int(lo as i64))
    });

    bisect_func!("insort_left", |args| {
        if args.len() < 2 { return Err(PyError::type_error("insort_left() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("insort_left() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if items[mid].borrow().lt(x)? { lo = mid + 1; } else { hi = mid; }
        }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.insert(lo, x.clone()); Ok(py_none()) }
        else { Err(PyError::type_error("insort_left() argument must be a list")) }
    });

    bisect_func!("insort_right", |args| {
        if args.len() < 2 { return Err(PyError::type_error("insort_right() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("insort_right() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if x.borrow().lt(&items[mid])? { hi = mid; } else { lo = mid + 1; }
        }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.insert(lo, x.clone()); Ok(py_none()) }
        else { Err(PyError::type_error("insort_right() argument must be a list")) }
    });

    d
}

/// Creates a native heapq module for heap queue operations on lists.
pub fn create_heapq_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! heap_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Internal: sift-down (for heappop, heapreplace, heapify)
    fn _siftdown(heap: &mut Vec<PyObjectRef>, start: usize, pos: usize) {
        let mut pos = pos;
        while pos > start {
            let parent = (pos - 1) / 2;
            if heap[pos].borrow().lt(&heap[parent]).unwrap_or(false) {
                heap.swap(pos, parent);
                pos = parent;
            } else {
                break;
            }
        }
    }

    // Internal: sift-up (for heapify)
    fn _siftup(heap: &mut Vec<PyObjectRef>, pos: usize) {
        let end = heap.len();
        let mut pos = pos;
        let start = pos;
        while pos < end {
            let left = 2 * pos + 1;
            let right = 2 * pos + 2;
            let mut smallest = pos;
            if left < end && heap[left].borrow().lt(&heap[smallest]).unwrap_or(false) {
                smallest = left;
            }
            if right < end && heap[right].borrow().lt(&heap[smallest]).unwrap_or(false) {
                smallest = right;
            }
            if smallest == pos { break; }
            heap.swap(pos, smallest);
            pos = smallest;
        }
        // Bubble back up if needed (after moving nodes)
        _siftdown(heap, start, pos);
    }

    heap_func!("heapify", |args| {
        if args.is_empty() { return Err(PyError::type_error("heapify() missing required argument")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            let n = list.len();
            if n > 1 {
                for i in (0..n / 2).rev() {
                    _siftup(list, i);
                }
            }
            Ok(py_none())
        } else {
            Err(PyError::type_error("heapify() argument must be a list"))
        }
    });

    heap_func!("heappush", |args| {
        if args.len() < 2 { return Err(PyError::type_error("heappush() requires 2 arguments (heap, item)")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            list.push(args[1].clone());
            _siftdown(list, 0, list.len() - 1);
            Ok(py_none())
        } else {
            Err(PyError::type_error("heappush() argument must be a list"))
        }
    });

    heap_func!("heappop", |args| {
        if args.is_empty() { return Err(PyError::type_error("heappop() missing required argument")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            if list.is_empty() { return Err(PyError::index_error("pop from an empty heap")); }
            let last = list.len() - 1;
            list.swap(0, last);
            let result = list.pop().unwrap();
            if !list.is_empty() { _siftup(list, 0); }
            Ok(result)
        } else {
            Err(PyError::type_error("heappop() argument must be a list"))
        }
    });

    heap_func!("heapreplace", |args| {
        if args.len() < 2 { return Err(PyError::type_error("heapreplace() requires 2 arguments (heap, item)")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            if list.is_empty() { return Err(PyError::index_error("heapreplace() on empty heap")); }
            let result = list[0].clone();
            list[0] = args[1].clone();
            _siftup(list, 0);
            Ok(result)
        } else {
            Err(PyError::type_error("heapreplace() argument must be a list"))
        }
    });

    // Helper: extract comparable values for nlargest/nsmallest
    fn _extract_items(args: &[PyObjectRef]) -> PyResult<(usize, Vec<PyObjectRef>)> {
        if args.len() < 2 { return Err(PyError::type_error("requires at least 2 arguments (n, iterable)")); }
        let n = args[0].as_i64().ok_or_else(|| PyError::type_error("n must be an integer"))?;
        if n < 0 { return Err(PyError::value_error("n must be non-negative")); }
        let n = n as usize;
        // Extract items from iterable
        let iterable = crate::object::builtin_iter(&[args[1].clone()])?;
        let mut items = Vec::new();
        loop {
            match crate::object::builtin_next(&[iterable.clone()]) {
                Ok(val) => items.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok((n, items))
    }

    heap_func!("nlargest", |args| {
        let (n, mut items) = _extract_items(args)?;
        if n == 0 { return Ok(py_list(Vec::new())); }
        // Use a min-heap of size n to track largest n elements
        if items.len() <= n {
            // Sort descending
            items.sort_by(|a, b| b.borrow().lt(a).unwrap_or(false).cmp(&true).reverse());
            return Ok(py_list(items));
        }
        // Build a min-heap of the first n elements
        let mut heap: Vec<PyObjectRef> = items.drain(..n).collect();
        if heap.len() > 1 {
            for i in (0..heap.len() / 2).rev() {
                _siftup(&mut heap, i);
            }
        }
        for item in items {
            if item.borrow().lt(&heap[0]).unwrap_or(false) {
                // item < smallest in heap, skip
            } else {
                heap[0] = item;
                _siftup(&mut heap, 0);
            }
        }
        // Sort descending
        heap.sort_by(|a, b| b.borrow().lt(a).unwrap_or(false).cmp(&true).reverse());
        Ok(py_list(heap))
    });

    heap_func!("nsmallest", |args| {
        let (n, mut items) = _extract_items(args)?;
        if n == 0 { return Ok(py_list(Vec::new())); }
        if items.len() <= n {
            items.sort_by(|a, b| a.borrow().lt(b).unwrap_or(false).cmp(&true));
            return Ok(py_list(items));
        }
        // Use a max-heap (negation) of size n to track smallest n elements
        // Actually, we can use a max-heap: track largest in the small set
        // For max-heap we invert comparison
        let mut heap: Vec<PyObjectRef> = items.drain(..n).collect();
        if heap.len() > 1 {
            for i in (0..heap.len() / 2).rev() {
                _siftup_max(&mut heap, i);
            }
        }
        for item in items {
            if heap[0].borrow().lt(&item).unwrap_or(false) {
                // item < heap[0], skip
            } else {
                heap[0] = item;
                _siftup_max(&mut heap, 0);
            }
        }
        heap.sort_by(|a, b| a.borrow().lt(b).unwrap_or(false).cmp(&true));
        Ok(py_list(heap))
    });

    fn _siftup_max(heap: &mut Vec<PyObjectRef>, pos: usize) {
        let end = heap.len();
        let mut pos = pos;
        while pos < end {
            let left = 2 * pos + 1;
            let right = 2 * pos + 2;
            let mut largest = pos;
            if left < end && heap[largest].borrow().lt(&heap[left]).unwrap_or(false) {
                largest = left;
            }
            if right < end && heap[largest].borrow().lt(&heap[right]).unwrap_or(false) {
                largest = right;
            }
            if largest == pos { break; }
            heap.swap(pos, largest);
            pos = largest;
        }
    }

    d
}

use std::sync::atomic::{AtomicI64, Ordering};
static ENUM_AUTO_COUNTER: AtomicI64 = AtomicI64::new(1);

/// Creates a native enum module with Enum, IntEnum, and auto().
pub fn create_enum_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! enum_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    enum_func!("auto", |args| {
        let _ = args;
        let val = ENUM_AUTO_COUNTER.fetch_add(1, Ordering::SeqCst);
        Ok(py_int(val))
    });

    enum_func!("Enum", |args| {
        if args.is_empty() { return Err(PyError::type_error("Enum() requires at least 1 argument")); }
        // Enum is a callable that returns an int.
        // If called with (name, value), return value; if called with just value, return it.
        if args.len() >= 2 {
            Ok(args[1].clone())
        } else {
            Ok(args[0].clone())
        }
    });

    enum_func!("IntEnum", |args| {
        if args.is_empty() { return Err(PyError::type_error("IntEnum() requires at least 1 argument")); }
        if args.len() >= 2 {
            Ok(args[1].clone())
        } else {
            Ok(args[0].clone())
        }
    });

    d
}

// === GLOB MODULE ===
pub fn create_glob_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! glob_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn glob_match(name: &str, pattern: &str) -> bool {
        let re_str = format!("^{}$", pattern.replace(".", "\\.").replace("?", ".").replace("*", ".*"));
        regex::Regex::new(&re_str).map(|re| re.is_match(name)).unwrap_or(false)
    }

    fn walk_glob(base: &std::path::Path, parts: &[&str], prefix: &str, results: &mut Vec<String>) {
        if parts.is_empty() {
            return;
        }
        let part = parts[0];
        let rest = &parts[1..];

        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !glob_match(&name, part) {
                    continue;
                }
                let full = if prefix.is_empty() { name.clone() } else { format!("{}/{}", prefix, name) };
                if rest.is_empty() {
                    results.push(full);
                } else {
                    let path = entry.path();
                    if path.is_dir() {
                        walk_glob(&path, rest, &full, results);
                    }
                }
            }
        }
    }

    glob_func!("glob", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("glob() takes exactly 1 argument"));
        }
        let pattern = args[0].str();
        let pattern = pattern.trim().to_string();
        if pattern.is_empty() {
            return Ok(py_list(vec![]));
        }

        let is_absolute = pattern.starts_with('/');
        let parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Ok(py_list(vec![]));
        }

        let start = if is_absolute { std::path::Path::new("/") } else { std::path::Path::new(".") };

        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(start) {
            let first = parts[0];
            let rest = &parts[1..];
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !glob_match(&name, first) {
                    continue;
                }
                if rest.is_empty() {
                    results.push(name);
                } else {
                    let path = entry.path();
                    if path.is_dir() {
                        walk_glob(&path, rest, &name, &mut results);
                    }
                }
            }
        }

        results.sort();
        let py_results: Vec<PyObjectRef> = results.into_iter().map(|s| py_str(&s)).collect();
        Ok(py_list(py_results))
    });
    d
}

// === FNMATCH MODULE ===
pub fn create_fnmatch_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! fnmatch_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn fnmatch_match(name: &str, pattern: &str) -> bool {
        let mut re_str = String::from("^");
        for ch in pattern.chars() {
            match ch {
                '.' => re_str.push_str("\\."),
                '*' => re_str.push_str(".*"),
                '?' => re_str.push('.'),
                other => re_str.push(other),
            }
        }
        re_str.push('$');
        regex::Regex::new(&re_str).map(|re| re.is_match(name)).unwrap_or(false)
    }

    fnmatch_func!("fnmatch", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("fnmatch() takes exactly 2 arguments"));
        }
        let name = args[0].str();
        let pattern = args[1].str();
        Ok(py_bool(fnmatch_match(&name, &pattern)))
    });
    d
}

// === TEXTWRAP MODULE ===
pub fn create_textwrap_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tw_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    tw_func!("dedent", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("dedent() takes exactly 1 argument"));
        }
        let text = args[0].str();
        let indent = text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);
        let result: String = text.lines()
            .map(|l| {
                if l.len() >= indent && l.chars().take(indent).all(|c| c.is_whitespace()) {
                    &l[indent..]
                } else {
                    l
                }
            })
            .collect::<Vec<&str>>()
            .join("\n");
        Ok(py_str(&result))
    });

    tw_func!("indent", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("indent() takes at least 2 arguments"));
        }
        let text = args[0].str();
        let prefix = args[1].str();
        let result: String = text.lines()
            .map(|l| format!("{}{}", prefix, l))
            .collect::<Vec<String>>()
            .join("\n");
        Ok(py_str(&result))
    });

    tw_func!("fill", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("fill() takes at least 1 argument"));
        }
        let text = args[0].str();
        let width = if args.len() > 2 {
            args[2].as_i64().unwrap_or(70) as usize
        } else {
            70
        };
        if width == 0 || width >= text.len() {
            return Ok(py_str(&text));
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        for word in words {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        Ok(py_str(&lines.join("\n")))
    });

    tw_func!("shorten", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("shorten() takes at least 1 argument"));
        }
        let text = args[0].str();
        let width = if args.len() > 1 {
            args[1].as_i64().unwrap_or(70) as usize
        } else {
            70
        };
        if text.len() <= width {
            return Ok(py_str(&text));
        }
        let truncated: String = text.chars().take(width).collect();
        if let Some(last_space) = truncated.rfind(' ') {
            let result: String = truncated[..last_space].to_string() + " ...";
            Ok(py_str(&result))
        } else {
            Ok(py_str(&(truncated + " ...")))
        }
    });

    d
}

// === PPRINT MODULE ===
pub fn create_pprint_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pp_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn pprint_recurse(obj: &PyObjectRef, indent: usize, out: &mut String) {
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::List(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(item, indent + 4, out);
                    if i < items.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push(']');
            }
            PyObject::Tuple(items) => {
                if items.is_empty() {
                    out.push_str("()");
                    return;
                }
                if items.len() == 1 {
                    out.push_str("(\n");
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(&items[0], indent + 4, out);
                    out.push_str(",\n");
                    out.push_str(&" ".repeat(indent));
                    out.push(')');
                    return;
                }
                out.push_str("(\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(item, indent + 4, out);
                    if i < items.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push(')');
            }
            PyObject::Dict(dict) => {
                if dict.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                let pairs = dict.items();
                for (i, (k, v)) in pairs.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(k, indent + 4, out);
                    out.push_str(": ");
                    pprint_recurse(v, indent + 4, out);
                    if i < pairs.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push('}');
            }
            PyObject::Set(items) => {
                let vec = items.to_vec();
                if vec.is_empty() {
                    out.push_str("set()");
                    return;
                }
                out.push_str("{\n");
                for (i, item) in vec.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(item, indent + 4, out);
                    if i < vec.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push('}');
            }
            PyObject::Str(s) => {
                out.push('\'');
                out.push_str(s);
                out.push('\'');
            }
            _ => {
                out.push_str(&borrowed.repr());
            }
        }
    }

    pp_func!("pprint", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("pprint() takes at least 1 argument"));
        }
        let mut out = String::new();
        pprint_recurse(&args[0], 0, &mut out);
        print!("{}", out);
        Ok(py_none())
    });

    d
}

// === HASHLIB MODULE ===
pub fn create_hashlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! hl_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    hl_func!("sha256", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha256() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha256() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"sha256");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    hl_func!("md5", |args| {
        if args.len() != 1 { return Err(PyError::type_error("md5() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("md5() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"md5");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    d
}

// === BASE64 MODULE ===
pub fn create_base64_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! b64_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn b64_encode(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let len = chunk.len();
            let b0 = chunk[0];
            let b1 = if len > 1 { chunk[1] } else { 0 };
            let b2 = if len > 2 { chunk[2] } else { 0 };
            out.push(CHARS[(b0 >> 2) as usize] as char);
            out.push(CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if len > 1 {
                out.push(CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                out.push('=');
            }
            if len > 2 {
                out.push(CHARS[(b2 & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
        let mut rev = [255u8; 256];
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for (i, &c) in alphabet.iter().enumerate() {
            rev[c as usize] = i as u8;
        }
        let bytes = s.as_bytes();
        if bytes.len() % 4 != 0 {
            return Err("Invalid base64 input length".to_string());
        }
        let mut out = Vec::new();
        for chunk in bytes.chunks(4) {
            let mut vals = [0u8; 4];
            for i in 0..4 {
                if chunk[i] == b'=' {
                    vals[i] = 0;
                } else {
                    let v = rev[chunk[i] as usize];
                    if v == 255 {
                        return Err("Invalid base64 character".to_string());
                    }
                    vals[i] = v;
                }
            }
            out.push((vals[0] << 2) | (vals[1] >> 4));
            if chunk[2] != b'=' {
                out.push((vals[1] << 4) | (vals[2] >> 2));
            }
            if chunk[3] != b'=' {
                out.push((vals[2] << 6) | vals[3]);
            }
        }
        Ok(out)
    }

    b64_func!("b64encode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b64encode() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("b64encode() argument must be bytes")),
        };
        Ok(py_str(&b64_encode(&bytes)))
    });

    b64_func!("b64decode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b64decode() takes exactly one argument")); }
        let data = args[0].borrow();
        let s = match &*data {
            PyObject::Str(s) => s.clone(),
            _ => return Err(PyError::type_error("b64decode() argument must be a string")),
        };
        match b64_decode(&s) {
            Ok(bytes) => Ok(PyObjectRef::imm(PyObject::Bytes(bytes))),
            Err(e) => Err(PyError::value_error(e)),
        }
    });

    d
}

// === UUID MODULE ===
pub fn create_uuid_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! uuid_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    uuid_func!("uuid4", |args| {
        if !args.is_empty() { return Err(PyError::type_error("uuid4() takes no arguments")); }
        let r1 = fast_random_u64();
        let r2 = fast_random_u64();
        let time_low = r1 as u32;
        let time_mid = (r1 >> 32) as u16;
        let time_hi_and_version = ((r1 >> 48) as u16 & 0x0FFF) | 0x4000;
        let clock_seq = (r2 as u16 & 0x3FFF) | 0x8000;
        let node = (r2 >> 16) as u64;
        Ok(py_str(&format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            time_low, time_mid, time_hi_and_version, clock_seq, node)))
    });

    d
}

// === STRING MODULE ===
pub fn create_string_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let ascii_lowercase = "abcdefghijklmnopqrstuvwxyz";
    let ascii_uppercase = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let ascii_letters = &format!("{}{}", ascii_lowercase, ascii_uppercase);
    let digits = "0123456789";
    let hexdigits = "0123456789abcdefABCDEF";
    let octdigits = "01234567";
    let punctuation = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
    let whitespace = " \t\n\r\u{0b}\u{0c}";
    let printable = &format!("{}{}{}{}", digits, ascii_letters, punctuation, whitespace);

    d.insert("ascii_letters".to_string(), py_str(ascii_letters));
    d.insert("ascii_lowercase".to_string(), py_str(ascii_lowercase));
    d.insert("ascii_uppercase".to_string(), py_str(ascii_uppercase));
    d.insert("digits".to_string(), py_str(digits));
    d.insert("hexdigits".to_string(), py_str(hexdigits));
    d.insert("octdigits".to_string(), py_str(octdigits));
    d.insert("punctuation".to_string(), py_str(punctuation));
    d.insert("printable".to_string(), py_str(printable));
    d.insert("whitespace".to_string(), py_str(whitespace));

    d
}

// === CSV MODULE ===
pub fn create_csv_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! csv_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    csv_func!("reader", |args| {
        if args.is_empty() { return Err(PyError::type_error("reader() missing required argument")); }
        let s = args[0].str();
        let mut result = Vec::new();
        for line in s.lines() {
            if line.trim().is_empty() { continue; }
            let fields: Vec<PyObjectRef> = line.split(',').map(|f| py_str(f.trim())).collect();
            result.push(py_list(fields));
        }
        Ok(py_list(result))
    });

    csv_func!("writer", |args| {
        if args.is_empty() { return Err(PyError::type_error("writer() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(rows) = &*borrowed {
            let mut lines = Vec::new();
            for row in rows {
                let row_b = row.borrow();
                if let PyObject::List(fields) = &*row_b {
                    let line: Vec<String> = fields.iter().map(|f| f.str()).collect();
                    lines.push(line.join(","));
                } else {
                    return Err(PyError::type_error("writer() argument must be a list of lists"));
                }
            }
            Ok(py_str(&lines.join("\n")))
        } else {
            Err(PyError::type_error("writer() argument must be a list of lists"))
        }
    });

    d
}

// === IO MODULE ===
fn io_stringio_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
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

fn io_stringio_readline(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
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

fn io_stringio_write(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
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

fn io_stringio_seek(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Err(PyError::type_error("seek() missing required argument")); }
    let self_obj = &args[0];
    let pos = args[1].as_i64().unwrap_or(0);
    if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
        dict.insert("_pos".to_string(), py_int(pos));
    }
    Ok(py_int(pos))
}

fn io_stringio_tell(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("tell() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        Ok(dict.get("_pos").cloned().unwrap_or(py_int(0)))
    } else {
        Err(PyError::type_error("StringIO instance required"))
    }
}

fn io_stringio_getvalue(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("getvalue() missing required 'self' argument")); }
    let self_obj = &args[0];
    let inst = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        Ok(dict.get("_buffer").cloned().unwrap_or(py_str("")))
    } else {
        Err(PyError::type_error("StringIO instance required"))
    }
}

pub fn create_io_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! io_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    io_func!("StringIO", |args| {
        // Create the type on each call (no caching needed for native modules)
        let mut type_dict = HashMap::new();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: io_stringio_read }));
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "readline".to_string(), func: io_stringio_readline }));
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: io_stringio_write }));
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "seek".to_string(), func: io_stringio_seek }));
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "tell".to_string(), func: io_stringio_tell }));
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "getvalue".to_string(), func: io_stringio_getvalue }));
        let typ = PyObjectRef::new(PyObject::Type {
            name: "StringIO".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });
        let mut instance_dict = HashMap::new();
        let initial = if !args.is_empty() { args[0].str() } else { String::new() };
        instance_dict.insert("_buffer".to_string(), py_str(&initial));
        instance_dict.insert("_pos".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ,
            dict: instance_dict,
        }))
    });

    d
}

// === STATISTICS MODULE ===
pub fn create_statistics_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! stat_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    stat_func!("mean", |args| {
        if args.is_empty() { return Err(PyError::type_error("mean() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.is_empty() {
                return Err(PyError::ValueError("mean() argument is empty".to_string()));
            }
            let mut sum = 0.0f64;
            let mut count = 0usize;
            for item in items {
                let v = item.borrow();
                match &*v {
                    PyObject::Int(i) => { sum += i.to_f64().unwrap_or(0.0); count += 1; }
                    PyObject::Float(f) => { sum += f; count += 1; }
                    _ => return Err(PyError::type_error("mean() argument must contain numbers")),
                }
            }
            Ok(py_float(sum / count as f64))
        } else {
            Err(PyError::type_error("mean() argument must be a list"))
        }
    });

    stat_func!("median", |args| {
        if args.is_empty() { return Err(PyError::type_error("median() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.is_empty() {
                return Err(PyError::ValueError("median() argument is empty".to_string()));
            }
            let mut nums: Vec<f64> = Vec::with_capacity(items.len());
            for item in items {
                let v = item.borrow();
                match &*v {
                    PyObject::Int(i) => nums.push(i.to_f64().unwrap_or(0.0)),
                    PyObject::Float(f) => nums.push(*f),
                    _ => return Err(PyError::type_error("median() argument must contain numbers")),
                }
            }
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = nums.len();
            if n % 2 == 0 {
                Ok(py_float((nums[n/2 - 1] + nums[n/2]) / 2.0))
            } else {
                Ok(py_float(nums[n/2]))
            }
        } else {
            Err(PyError::type_error("median() argument must be a list"))
        }
    });

    stat_func!("stdev", |args| {
        if args.is_empty() { return Err(PyError::type_error("stdev() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.len() < 2 {
                return Err(PyError::ValueError("stdev() requires at least 2 data points".to_string()));
            }
            let mut nums: Vec<f64> = Vec::with_capacity(items.len());
            for item in items {
                let v = item.borrow();
                match &*v {
                    PyObject::Int(i) => nums.push(i.to_f64().unwrap_or(0.0)),
                    PyObject::Float(f) => nums.push(*f),
                    _ => return Err(PyError::type_error("stdev() argument must contain numbers")),
                }
            }
            let n = nums.len() as f64;
            let sum: f64 = nums.iter().sum();
            let mean = sum / n;
            let variance: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
            Ok(py_float(variance.sqrt()))
        } else {
            Err(PyError::type_error("stdev() argument must be a list"))
        }
    });

    stat_func!("mode", |args| {
        if args.is_empty() { return Err(PyError::type_error("mode() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.is_empty() {
                return Err(PyError::ValueError("mode() argument is empty".to_string()));
            }
            let mut counts = std::collections::HashMap::new();
            let mut max_count = 0i64;
            let mut modes: Vec<PyObjectRef> = Vec::new();
            for item in items {
                let hash = item.hash()?;
                let entry = counts.entry(hash).or_insert((0i64, item.clone()));
                entry.0 += 1;
            }
            // Find the max count
            for (_, (count, ref item)) in &counts {
                if *count > max_count {
                    max_count = *count;
                    modes.clear();
                    modes.push(item.clone());
                } else if *count == max_count {
                    modes.push(item.clone());
                }
            }
            if modes.len() == 1 {
                Ok(modes[0].clone())
            } else {
                Ok(py_list(modes))
            }
        } else {
            Err(PyError::type_error("mode() argument must be a list"))
        }
    });

    d
}


pub fn create_contextlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ctx_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    ctx_func!("contextmanager", |args| {
        if args.is_empty() { return Err(PyError::type_error("contextmanager() missing argument")); }
        Ok(args[0].clone())
    });
    ctx_func!("nullcontext", |args| {
        if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) }
    });
    ctx_func!("suppress", |_| Ok(py_none()));
    d
}

pub fn create_decimal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dec_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    dec_func!("Decimal", |args| {
        if args.is_empty() { return Err(PyError::type_error("Decimal() missing argument")); }
        let val = args[0].str();
        Ok(py_str(&format!("Decimal('{}')", val)))
    });
    d
}

pub fn create_fractions_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! frac_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    frac_func!("Fraction", |args| {
        if args.len() < 2 { return Err(PyError::type_error("Fraction() requires 2 arguments")); }
        let n = args[0].as_i64().unwrap_or(0);
        let mut den = args[1].as_i64().unwrap_or(1);
        if den == 0 { return Err(PyError::ValueError("Fraction denominator cannot be zero".to_string())); }
        let mut num = n;
        if den < 0 { num = -num; den = -den; }
        let g = {
            let mut a = num.abs();
            let mut b = den;
            while b != 0 { let t = b; b = a % b; a = t; }
            a
        };
        if g > 1 { num /= g; den /= g; }
        Ok(py_str(&format!("{}/{}", num, den)))
    });
    d
}

// ---- platform module ----
pub fn create_platform_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! plat_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    plat_func!("platform", |_| {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        Ok(py_str(&format!("{}-{}", os, arch)))
    });
    plat_func!("machine", |_| {
        Ok(py_str(std::env::consts::ARCH))
    });
    plat_func!("processor", |_| {
        // Fall back to architecture string if no more specific info
        Ok(py_str(std::env::consts::ARCH))
    });
    plat_func!("python_implementation", |_| {
        Ok(py_str("RustPython"))
    });
    plat_func!("python_version", |_| {
        Ok(py_str("3.12.0"))
    });
    plat_func!("system", |_| {
        Ok(py_str(std::env::consts::OS))
    });
    d
}

// ---- getopt module ----
pub fn create_getopt_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! getopt_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Helper: check if a short option expects an argument (followed by ':' in shortopts)
    fn short_has_arg(c: char, shortopts: &str) -> bool {
        if let Some(pos) = shortopts.find(c) {
            shortopts.as_bytes().get(pos + 1) == Some(&b':')
        } else {
            false
        }
    }

    getopt_func!("getopt", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("getopt() requires at least 2 arguments (args, shortopts)"));
        }
        let shortopts = args[1].str();
        // Parse longopts if provided (third argument is a list of long option names)
        let longopts: Vec<String> = if args.len() > 2 {
            if let PyObject::List(list) = &*args[2].borrow() {
                list.iter().map(|s| s.str()).collect()
            } else {
                return Err(PyError::type_error("longopts must be a list"));
            }
        } else {
            Vec::new()
        };

        // Extract the argument list from the first argument (should be a list of strings)
        let arg_list: Vec<String> = if let PyObject::List(list) = &*args[0].borrow() {
            list.iter().map(|s| s.str()).collect()
        } else {
            return Err(PyError::type_error("args must be a list"));
        };

        let mut opts: Vec<PyObjectRef> = Vec::new();
        let mut positional: Vec<PyObjectRef> = Vec::new();
        let mut i: usize = 1; // Skip the program name (args[0])
        let mut options_done = false;

        while i < arg_list.len() {
            let arg = &arg_list[i];
            if options_done || !arg.starts_with('-') {
                positional.push(py_str(arg));
                i += 1;
                if arg.starts_with('-') { options_done = true; }
                continue;
            }
            if arg == "--" {
                options_done = true;
                i += 1;
                continue;
            }
            if arg.starts_with("--") {
                // Long option
                let opt_name = &arg[2..];
                let (name, val) = if let Some(eq_pos) = opt_name.find('=') {
                    (&opt_name[..eq_pos], Some(&opt_name[eq_pos + 1..]))
                } else {
                    (opt_name, None)
                };
                // Check if this long option expects an argument
                let needs_val = longopts.iter().any(|lo| {
                    let base = if lo.ends_with('=') { &lo[..lo.len()-1] } else { lo.as_str() };
                    base == name && lo.ends_with('=')
                });
                match val {
                    Some(v) => opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str(v)])),
                    None => {
                        if needs_val {
                            i += 1;
                            if i < arg_list.len() {
                                opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str(&arg_list[i])]));
                            } else {
                                return Err(PyError::type_error(&format!("option --{} requires a value", name)));
                            }
                        } else {
                            opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str("")]));
                        }
                    }
                }
                i += 1;
            } else {
                // Short option(s)
                let chars: Vec<char> = arg[1..].chars().collect();
                for (j, c) in chars.iter().enumerate() {
                    if !shortopts.contains(*c) {
                        return Err(PyError::type_error(&format!("option -{} not recognized", c)));
                    }
                    if short_has_arg(*c, &shortopts) {
                        if j + 1 < chars.len() {
                            // Value attached: -xvalue
                            let val: String = chars[j + 1..].iter().collect();
                            opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str(&val)]));
                            break;
                        } else {
                            i += 1;
                            if i < arg_list.len() {
                                opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str(&arg_list[i])]));
                            } else {
                                return Err(PyError::type_error(&format!("option -{} requires an argument", c)));
                            }
                        }
                    } else {
                        opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str("")]));
                    }
                }
                i += 1;
            }
        }

        Ok(py_tuple(vec![py_list(opts), py_list(positional)]))
    });
    d
}

// ---- getpass module ----
pub fn create_getpass_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! getpass_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    getpass_func!("getuser", |_| {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        Ok(py_str(&user))
    });
    getpass_func!("getpass", |args| {
        let prompt = if args.is_empty() { "Password: ".to_string() } else { args[0].str() };
        // In this minimal native implementation, we echo the prompt and read a line from stdin.
        // This is simplified — a real getpass would disable terminal echo.
        print!("{}", prompt);
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut password = String::new();
        match std::io::stdin().read_line(&mut password) {
            Ok(_) => Ok(py_str(password.trim_end())),
            Err(_) => Err(PyError::runtime_error("failed to read password")),
        }
    });
    d
}

// ---- tempfile module ----
pub fn create_tempfile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! temp_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Simple random suffix generator using /dev/urandom or fallback
    fn random_suffix(len: usize) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        // Mix time-based and random characters
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let mut result = String::with_capacity(len);
        let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut seed = ts;
        for _ in 0..len {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let idx = (seed as usize) % chars.len();
            result.push(chars[idx] as char);
        }
        result
    }

    temp_func!("mkstemp", |_| {
        // Try up to 10 times to create a unique temp file
        for _ in 0..10 {
            let suffix = random_suffix(8);
            let name = format!("/tmp/tmp{}", suffix);
            match std::fs::File::create_new(&name) {
                Ok(file) => {
                    use std::os::fd::AsRawFd;
                    let fd = file.as_raw_fd();
                    // Return (fd, name) as a tuple
                    return Ok(py_tuple(vec![py_int(fd as i64), py_str(&name)]));
                }
                Err(_) => continue,
            }
        }
        Err(PyError::runtime_error("could not create temporary file"))
    });

    temp_func!("mkdtemp", |_| {
        for _ in 0..10 {
            let suffix = random_suffix(8);
            let name = format!("/tmp/tmp{}", suffix);
            match std::fs::create_dir(&name) {
                Ok(()) => return Ok(py_str(&name)),
                Err(_) => continue,
            }
        }
        Err(PyError::runtime_error("could not create temporary directory"))
    });

    // Add temporary directory path
    d.insert("tempdir".to_string(), py_str("/tmp"));
    d
}

// ---- shutil module ----
pub fn create_shutil_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! shutil_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    shutil_func!("copy", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("copy() requires 2 arguments (src, dst)"));
        }
        let src = args[0].str();
        let dst = args[1].str();
        match std::fs::copy(&src, &dst) {
            Ok(_) => Ok(py_str(&dst)),
            Err(e) => Err(PyError::OsError(format!("copy error: {}", e))),
        }
    });

    shutil_func!("rmtree", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("rmtree() requires 1 argument (path)"));
        }
        let path = args[0].str();
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("rmtree error: {}", e))),
        }
    });

    shutil_func!("move", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("move() requires 2 arguments (src, dst)"));
        }
        let src = args[0].str();
        let dst = args[1].str();
        match std::fs::rename(&src, &dst) {
            Ok(()) => Ok(py_str(&dst)),
            Err(e) => Err(PyError::OsError(format!("move error: {}", e))),
        }
    });
    d
}

// ---- graphlib module ----
pub fn create_graphlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! gl_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // TopologicalSorter — returns a sorted list from the given graph
    gl_func!("TopologicalSorter", |args| {
        // Accept a dict or list of pairs as graph
        let mut edges: Vec<(String, String)> = Vec::new();
        if args.len() >= 1 {
            let graph = &args[0];
            let borrowed = graph.borrow();
            match &*borrowed {
                PyObject::Dict(dict) => {
                    for (key, val) in dict.items() {
                        let k = key.str();
                        let v = val.str();
                        edges.push((k, v));
                    }
                }
                PyObject::List(items) => {
                    for item in items {
                        if let PyObject::Tuple(pair) = &*item.borrow() {
                            if pair.len() >= 2 {
                                edges.push((pair[0].str(), pair[1].str()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Simple topological sort: Kahn's algorithm
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        let mut nodes: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (from, to) in &edges {
            adj.entry(from.clone()).or_default().push(to.clone());
            in_degree.entry(to.clone()).or_insert(0);
            in_degree.entry(from.clone()).or_insert(0);
            nodes.insert(from.clone());
            nodes.insert(to.clone());
        }

        // Also handle nodes referenced only as keys/values in pairs
        for n in &edges {
            in_degree.entry(n.0.clone()).or_insert(0);
            in_degree.entry(n.1.clone()).or_insert(0);
            nodes.insert(n.0.clone());
            nodes.insert(n.1.clone());
        }

        for (_, neighbors) in &adj {
            for n in neighbors {
                *in_degree.entry(n.clone()).or_insert(0) += 1;
            }
        }

        let mut queue: Vec<String> = Vec::new();
        for (node, deg) in &in_degree {
            if *deg == 0 {
                queue.push(node.clone());
            }
        }

        let mut sorted: Vec<PyObjectRef> = Vec::new();
        while !queue.is_empty() {
            queue.sort_by(|a, b| b.cmp(a)); // use as stack
            let node = queue.pop().unwrap();
            sorted.push(py_str(&node));
            if let Some(neighbors) = adj.get(&node) {
                for n in neighbors {
                    if let Some(deg) = in_degree.get_mut(n) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(n.clone());
                        }
                    }
                }
            }
        }

        // If any nodes remain unprocessed, there's a cycle — just return empty
        // (stub behavior)
        if sorted.len() != nodes.len() {
            sorted.clear();
        }

        Ok(py_list(sorted))
    });

    d
}

// ---- pdb module ----
pub fn create_pdb_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pdb_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    pdb_func!("set_trace", |_| {
        println!("Debugger not available");
        Ok(py_none())
    });

    d
}

// ---- traceback module ----
pub fn create_traceback_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tb_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    tb_func!("format_exc", |_| {
        Ok(py_str(""))
    });

    tb_func!("print_exc", |_| {
        println!("No traceback");
        Ok(py_none())
    });

    d
}

// ---- warnings module ----
pub fn create_warnings_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! warn_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Store simplefilter state in a thread-local
    thread_local! {
        static WARN_FILTER: std::cell::RefCell<String> = std::cell::RefCell::new("default".to_string());
    }

    warn_func!("warn", |args| {
        let msg = if !args.is_empty() { args[0].str() } else { String::new() };
        println!("Warning: {}", msg);
        Ok(py_none())
    });

    warn_func!("simplefilter", |args| {
        if !args.is_empty() {
            let action = args[0].str();
            WARN_FILTER.with(|f| *f.borrow_mut() = action);
        }
        Ok(py_none())
    });

    // Insert the current filter state as a readable attribute
    d.insert("filters".to_string(), py_list(vec![]));

    d
}

// ---- abc module ----
pub fn create_abc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! abc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // ABC class — returns a simple Instance with a type marker
    abc_func!("ABC", |args| {
        let _ = args;
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "abc".to_string(),
                dict: HashMap::new(),
            }),
            dict: HashMap::new(),
        }))
    });

    // abstractmethod — returns the function unchanged
    abc_func!("abstractmethod", |args| {
        if !args.is_empty() {
            Ok(args[0].clone())
        } else {
            Ok(py_none())
        }
    });

    d
}
