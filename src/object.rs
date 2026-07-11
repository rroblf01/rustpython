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

#[derive(Clone)]
pub enum PyObjectRef {
    SmallInt(i64),
    SmallBool(bool),
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
            PyObjectRef::None => Ok(0),
            PyObjectRef::Mut(rc) => rc.borrow().hash(),
            PyObjectRef::Imm(rc) => (**rc).hash(),
        }
    }
    pub fn equals(&self, other: &PyObjectRef) -> PyResult<bool> {
        if let (Some(ai), Some(bi)) = (self.as_i64(), other.as_i64()) {
            return Ok(ai == bi);
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
        cause: Option<Box<PyError>>,
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
            PyObject::Range { .. } => "range",
            PyObject::RangeIter { .. } => "range_iterator",
            PyObject::ListIter { .. } => "list_iterator",
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
            PyObject::Super { .. } => "super",
            PyObject::Property { .. } => "property",
            PyObject::StaticMethod { .. } => "staticmethod",
            PyObject::ClassMethod { .. } => "classmethod",
            PyObject::Generator { .. } => "generator",
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
            PyObject::Bytes(b) => format!("b'{}'", String::from_utf8_lossy(b)),
            PyObject::ByteArray(b) => format!("bytearray(b'{}')", String::from_utf8_lossy(b)),
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
            PyObject::Range { start, stop, step } => {
                if *step == 1 { format!("range({}, {})", start, stop) }
                else { format!("range({}, {}, {})", start, stop, step) }
            }
            PyObject::RangeIter { .. } => "<range_iterator object>".to_string(),
            PyObject::ListIter { .. } => "<list_iterator object>".to_string(),
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
            PyObject::Super { .. } => format!("<super object>"),
            PyObject::Property { .. } => format!("<property object>"),
            PyObject::StaticMethod { .. } => format!("<staticmethod object>"),
            PyObject::ClassMethod { .. } => format!("<classmethod object>"),
            PyObject::Generator { .. } => format!("<generator object>"),
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
            PyObject::Range { start, stop, step } => *step > 0 && *start < *stop || *step < 0 && *start > *stop,
            PyObject::RangeIter { current, stop, step } => *step > 0 && *current < *stop || *step < 0 && *current > *stop,
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
            c if c.is_ascii_control() => {
                out.push_str(&format!("\\x{:02x}", c as u8));
            }
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
    PyObjectRef::imm(PyObject::Float(f))
}

pub fn py_str(s: &str) -> PyObjectRef {
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
            _ => None,
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
            _ => Err(PyError::type_error(format!("cannot convert '{}' to list", obj.type_name()))),
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

fn call_bound_method(func: PyObjectRef, self_obj: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
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
        // Simple insertion sort for small, merge sort for large
        // Actually, let's use Rust's sort_by with our comparison
        v.sort_by(|a, b| {
            match a.borrow().gt(b) {
                Ok(true) => std::cmp::Ordering::Greater,
                Ok(false) => match a.borrow().lt(b) {
                    Ok(true) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                },
                Err(_) => std::cmp::Ordering::Equal,
            }
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
        return call_bound_method(f, args[0].clone(), vec![]);
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

pub fn builtin_isinstance(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("isinstance() takes exactly 2 arguments"));
    }
    let obj = args[0].borrow();
    let class = args[1].borrow();
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
        PyObject::BoundMethod { .. } | PyObject::Partial { .. }
    );
    Ok(py_bool(is_callable))
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
    let cls = args[0].borrow();
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
    let source = args[0].str();
    let mut parser = crate::parser::Parser::new(&source);
    let program = parser.parse_program().map_err(|e| PyError::type_error(format!("exec parse error: {}", e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler.compile(&program, "<exec>").map_err(|e| PyError::type_error(format!("exec compile error: {}", e)))?;
    let mut vm = crate::vm::VirtualMachine::new();
    vm.run(code).map_err(|e| PyError::type_error(format!("exec error: {}", e)))?;
    Ok(py_none())
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
    Ok(PyObjectRef::new(PyObject::Property {
        getter,
        setter,
        deleter: None,
        doc: None,
    }))
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
                dict.get(name).cloned().ok_or_else(|| PyError::attribute_error(format!("module has no attribute '{}'", name)))
            }
            PyObject::Type { dict, mro, .. } => {
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
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.pop().ok_or_else(|| PyError::runtime_error("pop from empty list")) }
                            else { Err(PyError::runtime_error("pop on non-list")) }
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
                    _ => Err(PyError::attribute_error(format!("'list' object has no attribute '{}'", name))),
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
                    "startswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "startswith".to_string(),
                        func: |args| {
                            if args.len() < 2 { return Err(PyError::type_error("startswith() takes exactly one argument")); }
                            Ok(py_bool(args[0].str().starts_with(&args[1].str())))
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
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() { d.remove(&args[1]) }
                            else { Err(PyError::runtime_error("pop on non-dict")) }
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
            PyObject::Function { dict, .. } => {
                dict.get(name).cloned().ok_or_else(|| PyError::attribute_error(format!("'function' object has no attribute '{}'", name)))
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
                    _ => Err(PyError::attribute_error(format!("'generator' object has no attribute '{}'", name))),
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
                        func: |_| Ok(py_none()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'file' object has no attribute '{}'", name))),
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
    add_func!("isinstance", builtin_isinstance);
    add_func!("open", builtin_open);
    add_func!("any", builtin_any);
    add_func!("all", builtin_all);
    add_func!("callable", builtin_callable);
    add_func!("pow", builtin_pow);
    add_func!("reversed", builtin_reversed);
    add_func!("issubclass", builtin_issubclass);
    add_func!("help", builtin_help);
    add_func!("eval", builtin_eval);
    add_func!("exec", builtin_exec);
    add_func!("super", builtin_super);
    add_func!("map", builtin_map);
    add_func!("filter", builtin_filter);
    add_func!("zip", builtin_zip);
    add_func!("property", builtin_property);
    add_func!("staticmethod", builtin_staticmethod);
    add_func!("classmethod", builtin_classmethod);
    add_func!("bytes", builtin_bytes);
    add_func!("bytearray", builtin_bytearray);
    add_func!("format", builtin_format);
    add_func!("object", builtin_object);
    add_func!("hash", builtin_hash);
    add_func!("slice", builtin_slice);
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
    d
}

pub fn create_os_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! os_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    os_func!("getcwd", |_| {
        match std::env::current_dir() {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
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

pub fn create_json_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! json_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    json_func!("dumps", |args| {
        if args.is_empty() { return Err(PyError::type_error("dumps() missing required argument")); }
        json_encode(&args[0])
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

fn call_function(func: &PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
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

pub fn create_module(name: &str, dict: HashMap<String, PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Module {
        name: name.to_string(),
        dict,
    })
}
