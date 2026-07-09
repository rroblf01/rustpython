use std::rc::Rc;
use std::cell::RefCell;
use std::fmt;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign, ToBigInt};
use num_traits::{Zero, One, ToPrimitive, float::FloatCore, Signed};
use crate::bytecode::CodeObject;

pub type BuiltinFunc = fn(&[PyObjectRef]) -> PyResult<PyObjectRef>;

#[derive(Clone)]
pub struct PyObjectRef(pub Rc<RefCell<PyObject>>);

impl PyObjectRef {
    pub fn new(obj: PyObject) -> Self {
        PyObjectRef(Rc::new(RefCell::new(obj)))
    }

    pub fn borrow(&self) -> std::cell::Ref<PyObject> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<PyObject> {
        self.0.borrow_mut()
    }

    pub fn is(&self, other: &PyObjectRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn repr(&self) -> String {
        self.borrow().repr()
    }

    pub fn str(&self) -> String {
        self.borrow().str()
    }

    pub fn truthy(&self) -> bool {
        self.borrow().truthy()
    }

    pub fn hash(&self) -> PyResult<usize> {
        self.borrow().hash()
    }

    pub fn equals(&self, other: &PyObjectRef) -> PyResult<bool> {
        self.borrow().equals(other)
    }

    pub fn get_type_name(&self) -> String {
        self.borrow().type_name()
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

// ---- Core Object Enum ----

#[derive(Clone)]
pub enum PyObject {
    None,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<PyObjectRef>),
    Tuple(Vec<PyObjectRef>),
    Dict(HashMap<String, PyObjectRef>),
    Set(Vec<PyObjectRef>),
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
            PyObject::List(_) => "list",
            PyObject::Tuple(_) => "tuple",
            PyObject::Dict(_) => "dict",
            PyObject::Set(_) => "set",
            PyObject::Slice { .. } => "slice",
            PyObject::Code(_) => "code",
            PyObject::Function { .. } => "function",
            PyObject::BuiltinFunction { .. } => "builtin_function_or_method",
            PyObject::BuiltinMethod { .. } => "builtin_method",
            PyObject::Module { .. } => "module",
            PyObject::Type { .. } => "type",
            PyObject::Instance { .. } => "instance",
            PyObject::Cell { .. } => "cell",
            PyObject::Capsule { .. } => "capsule",
            PyObject::Exception { .. } => "Exception",
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
                let items: Vec<String> = d.iter()
                    .map(|(k, v)| format!("{}: {}", k, v.repr()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            PyObject::Set(items) => {
                let items: Vec<String> = items.iter().map(|x| x.repr()).collect();
                format!("{{{}}}", items.join(", "))
            }
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
            PyObject::Tuple(items) => {
                let mut h: usize = 0x345678;
                for item in items {
                    h = h.wrapping_mul(1000003).wrapping_add(item.hash()?);
                }
                Ok(h)
            }
            _ => Err(PyError::type_error(format!("unhashable type: '{}'", self.type_name()))),
        }
    }

    pub fn equals(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        if std::mem::discriminant(self) != std::mem::discriminant(&*other) {
            return Ok(false);
        }
        match (self, &*other) {
            (PyObject::None, PyObject::None) => Ok(true),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a == b),
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a == b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a == b),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a == b),
            (PyObject::Bytes(a), PyObject::Bytes(b)) => Ok(a == b),
            (PyObject::List(a), PyObject::List(b)) => {
                if a.len() != b.len() { return Ok(false); }
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return Ok(false); }
                }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                if a.len() != b.len() { return Ok(false); }
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return Ok(false); }
                }
                Ok(true)
            }
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a == b),
            _ => Ok(false),
        }
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

pub fn py_none() -> PyObjectRef {
    PyObjectRef::new(PyObject::None)
}

pub fn py_bool(b: bool) -> PyObjectRef {
    PyObjectRef::new(PyObject::Bool(b))
}

pub fn py_int(i: impl Into<BigInt>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Int(i.into()))
}

pub fn py_float(f: f64) -> PyObjectRef {
    PyObjectRef::new(PyObject::Float(f))
}

pub fn py_str(s: &str) -> PyObjectRef {
    PyObjectRef::new(PyObject::Str(s.to_string()))
}

pub fn py_list(items: Vec<PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::List(items))
}

pub fn py_tuple(items: Vec<PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Tuple(items))
}

pub fn py_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::Dict(HashMap::new()))
}

pub fn py_set() -> PyObjectRef {
    PyObjectRef::new(PyObject::Set(Vec::new()))
}

// ---- Binary Operations ----

pub fn py_add(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
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
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() - b)),
        (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a - b)),
        (PyObject::Int(a), PyObject::Float(b)) => Ok(py_float(a.to_f64().unwrap() - b)),
        (PyObject::Float(a), PyObject::Int(b)) => Ok(py_float(a - b.to_f64().unwrap())),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for -: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_mul(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
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
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if b.is_zero() { return Err(PyError::zero_division()); }
            if a.sign() == Sign::Minus && &(a % b) != &BigInt::zero() {
                // Python floor division rounds toward negative infinity
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

pub fn py_bit_or(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() | b)),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for |: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_bit_xor(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() ^ b)),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for ^: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

pub fn py_bit_and(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => Ok(py_int(a.clone() & b)),
        _ => Err(PyError::type_error(format!("unsupported operand type(s) for &: '{}' and '{}'",
            a_obj.type_name(), b_obj.type_name()))),
    }
}

// ---- Comparison Operations ----

pub fn py_compare(a: &PyObjectRef, b: &PyObjectRef, op: u32) -> PyResult<PyObjectRef> {
    let result = match op {
        0 => a.borrow().lt(b)?,  // <
        1 => a.borrow().le(b)?,  // <=
        2 => a.borrow().equals(b)?, // ==
        3 => a.borrow().ge(b)?,  // >=
        4 => a.borrow().gt(b)?,  // >
        5 => a.borrow().ne(b)?,  // !=
        6 => contains_op(b, a)?, // in
        7 => !contains_op(b, a)?, // not in
        8 => a.is(b),   // is
        9 => !a.is(b),  // is not
        _ => return Err(PyError::runtime_error("unknown comparison operator")),
    };
    Ok(py_bool(result))
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
            match &*b.borrow() {
                PyObject::Str(s) => Ok(d.contains_key(s)),
                _ => {
                    let key_str = b.str();
                    Ok(d.contains_key(&key_str))
                }
            }
        }
        PyObject::Set(items) => {
            for item in items {
                if item.equals(b)? { return Ok(true); }
            }
            Ok(false)
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
    let len = match &*obj {
        PyObject::Str(s) => s.chars().count(),
        PyObject::List(v) => v.len(),
        PyObject::Tuple(v) => v.len(),
        PyObject::Dict(d) => d.len(),
        PyObject::Set(s) => s.len(),
        PyObject::Bytes(b) => b.len(),
        _ => return Err(PyError::type_error(format!("object of type '{}' has no len()", obj.type_name()))),
    };
    Ok(py_int(len))
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
    Ok(PyObjectRef::new(PyObject::Str(name)))
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
    else { Ok(py_str(&args[0].str())) }
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
            PyObject::Set(s) => Ok(py_list(s.clone())),
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

pub fn builtin_dict(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(py_dict())
}

pub fn builtin_set(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_set()) }
    else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => Ok(PyObjectRef::new(PyObject::Set(v.clone()))),
            PyObject::Tuple(v) => Ok(PyObjectRef::new(PyObject::Set(v.clone()))),
            _ => Err(PyError::type_error(format!("cannot convert '{}' to set", obj.type_name()))),
        }
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
    let n = args[0].borrow();
    if let PyObject::Int(i) = &*n {
        let code = i.to_u32().ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
        let c = char::from_u32(code).ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
        Ok(py_str(&c.to_string()))
    } else {
        Err(PyError::type_error("chr() argument must be int"))
    }
}

pub fn builtin_hex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hex() takes exactly one argument"));
    }
    match &*args[0].borrow() {
        PyObject::Int(i) => Ok(py_str(&format!("0x{:x}", i))),
        _ => Err(PyError::type_error("hex() argument must be int")),
    }
}

pub fn builtin_oct(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("oct() takes exactly one argument"));
    }
    match &*args[0].borrow() {
        PyObject::Int(i) => Ok(py_str(&format!("0o{:o}", i))),
        _ => Err(PyError::type_error("oct() argument must be int")),
    }
}

pub fn builtin_bin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("bin() takes exactly one argument"));
    }
    match &*args[0].borrow() {
        PyObject::Int(i) => Ok(py_str(&format!("0b{:b}", i))),
        _ => Err(PyError::type_error("bin() argument must be int")),
    }
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

pub fn builtin_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("repr() takes exactly one argument"));
    }
    Ok(py_str(&args[0].repr()))
}

pub fn builtin_sorted(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("sorted() takes at least 1 argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::List(v) => {
            let mut v = v.clone();
            // Simple sort (bubble sort for now)
            let len = v.len();
            for i in 0..len {
                for j in 0..len - 1 - i {
                    if v[j].borrow().gt(&v[j + 1])? {
                        v.swap(j, j + 1);
                    }
                }
            }
            Ok(py_list(v))
        }
        _ => Err(PyError::type_error(format!("cannot sort '{}'", obj.type_name()))),
    }
}

pub fn builtin_enumerate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("enumerate() takes at least 1 argument"));
    }
    let obj = args[0].borrow();
    let start = if args.len() > 1 {
        if let PyObject::Int(i) = &*args[1].borrow() {
            i.to_usize().unwrap_or(0)
        } else { 0 }
    } else { 0 };
    match &*obj {
        PyObject::List(v) => {
            let items: Vec<PyObjectRef> = v.iter().enumerate()
                .map(|(i, item)| py_tuple(vec![py_int((start + i) as i64), item.clone()]))
                .collect();
            Ok(py_list(items))
        }
        _ => Err(PyError::type_error(format!("cannot enumerate '{}'", obj.type_name()))),
    }
}

pub fn builtin_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("iter() takes exactly one argument"));
    }
    // For now, return the list itself as iterator (simplified)
    Ok(args[0].clone())
}

pub fn builtin_next(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("next() takes at least 1 argument"));
    }
    // For now, only work with lists as iterators
    let mut obj = args[0].borrow_mut();
    match &mut *obj {
        PyObject::List(v) => {
            if v.is_empty() {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                Ok(v.remove(0))
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
    let obj = args[0].borrow();
    match &*obj {
        PyObject::List(v) => {
            let mut total = start;
            for item in v {
                total = py_add(&total, item)?;
            }
            Ok(total)
        }
        PyObject::Tuple(v) => {
            let mut total = start;
            for item in v {
                total = py_add(&total, item)?;
            }
            Ok(total)
        }
        _ => Err(PyError::type_error(format!("cannot sum '{}'", obj.type_name()))),
    }
}

fn compare_gt(a: &PyObjectRef, b: &PyObjectRef) -> std::cmp::Ordering {
    match a.borrow().gt(b) {
        Ok(true) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Less,
    }
}

pub fn builtin_max(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let items = if args.len() == 1 {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => v.clone(),
            _ => return Err(PyError::type_error("argument to max() must be iterable")),
        }
    } else {
        args.to_vec()
    };
    items.into_iter().max_by(compare_gt).ok_or_else(|| PyError::value_error("max() arg is an empty sequence"))
}

pub fn builtin_min(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let items = if args.len() == 1 {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => v.clone(),
            _ => return Err(PyError::type_error("argument to min() must be iterable")),
        }
    } else {
        args.to_vec()
    };
    items.into_iter().min_by(compare_gt).ok_or_else(|| PyError::value_error("min() arg is an empty sequence"))
}

pub fn builtin_id(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("id() takes exactly one argument"));
    }
    let ptr = &*args[0].0 as *const _ as usize;
    Ok(py_int(ptr as i64))
}

pub fn builtin_isinstance(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("isinstance() takes exactly 2 arguments"));
    }
    let obj_type = args[0].borrow().type_name();
    let class_name = args[1].str();
    Ok(py_bool(obj_type == class_name || class_name == "object"))
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
            PyObject::Type { dict, .. } => {
                dict.get(name).cloned().ok_or_else(|| PyError::attribute_error(format!("type has no attribute '{}'", name)))
            }
            PyObject::Instance { dict, .. } => {
                dict.get(name).cloned().ok_or_else(|| PyError::attribute_error(format!("instance has no attribute '{}'", name)))
            }
            PyObject::List(v) => {
                match name {
                    "append" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "append".to_string(),
                        func: |args| {
                            if args.len() != 1 {
                                return Err(PyError::type_error("append() takes exactly one argument"));
                            }
                            // This should operate on the list, but we need self reference
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'list' object has no attribute '{}'", name))),
                }
            }
            PyObject::Str(s) => {
                match name {
                    "split" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "split".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "upper" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "upper".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lower" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "lower".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "strip" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "strip".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "startswith" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "startswith".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'str' object has no attribute '{}'", name))),
                }
            }
            PyObject::Dict(d) => {
                match name {
                    "keys" => {
                        let keys: Vec<PyObjectRef> = d.keys().map(|k| py_str(k)).collect();
                        Ok(py_list(keys))
                    }
                    "values" => {
                        let vals: Vec<PyObjectRef> = d.values().cloned().collect();
                        Ok(py_list(vals))
                    }
                    "items" => {
                        let items: Vec<PyObjectRef> = d.iter()
                            .map(|(k, v)| py_tuple(vec![py_str(k), v.clone()]))
                            .collect();
                        Ok(py_list(items))
                    }
                    "get" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            Err(PyError::runtime_error("method call not fully implemented"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!("'dict' object has no attribute '{}'", name))),
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

pub fn py_getitem(obj: &PyObjectRef, index: &PyObjectRef) -> PyResult<PyObjectRef> {
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
                    let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or(0) } else { 0 };
                    let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(len as isize) } else { len as isize };
                    let step_val = if let PyObject::Int(i) = &*st { i.to_isize().unwrap_or(1) } else { 1 };
                    if step_val > 0 {
                        let mut i = start_val;
                        while i < stop_val && i < len as isize {
                            result.push(items[i as usize].clone());
                            i += step_val;
                        }
                    } else {
                        let mut i = if stop_val < 0 { stop_val } else { stop_val - 1 };
                        while i > start_val - 1 && i >= 0 {
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
                    let start_val = if let PyObject::Int(i) = &*s { i.to_isize().unwrap_or(0) } else { 0 };
                    let stop_val = if let PyObject::Int(i) = &*e { i.to_isize().unwrap_or(len as isize) } else { len as isize };
                    let step_val = if let PyObject::Int(i) = &*st { i.to_isize().unwrap_or(1) } else { 1 };
                    let mut result = String::new();
                    if step_val > 0 {
                        let mut i = start_val;
                        while i < stop_val && i < len as isize {
                            result.push(chars[i as usize]);
                            i += step_val;
                        }
                    } else {
                        let mut i = start_val.max(0);
                        while i >= stop_val.max(0) && i < len as isize {
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
            let key_str = index.str();
            d.get(&key_str).cloned().ok_or_else(|| PyError::key_error(key_str))
        }
        _ => Err(PyError::type_error(format!("'{}' object is not subscriptable", o.type_name()))),
    }
}

pub fn py_setitem(obj: &PyObjectRef, index: &PyObjectRef, value: PyObjectRef) -> PyResult<()> {
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
            let key_str = index.str();
            d.insert(key_str, value);
            Ok(())
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

// ---- Module creation ----

pub fn create_builtins() -> HashMap<String, PyObjectRef> {
    let mut builtins = HashMap::new();
    builtins.insert("None".to_string(), py_none());
    builtins.insert("True".to_string(), py_bool(true));
    builtins.insert("False".to_string(), py_bool(false));
    builtins.insert("Ellipsis".to_string(), PyObjectRef::new(PyObject::Str("...".to_string())));

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

    builtins
}

pub fn create_module(name: &str, dict: HashMap<String, PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Module {
        name: name.to_string(),
        dict,
    })
}
