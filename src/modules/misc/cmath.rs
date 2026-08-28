use crate::object::*;
use std::collections::HashMap;
use num_traits::ToPrimitive;

pub fn create_cmath_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cm_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    cm_func!("sqrt", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("sqrt() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sqrt())),
            PyObject::Float(f) => Ok(py_float(f.sqrt())),
            _ => Err(PyError::type_error("sqrt() argument must be a number")),
        }
    });
    cm_func!("sin", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("sin() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sin())),
            PyObject::Float(f) => Ok(py_float(f.sin())),
            _ => Err(PyError::type_error("sin() argument must be a number")),
        }
    });
    cm_func!("cos", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("cos() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).cos())),
            PyObject::Float(f) => Ok(py_float(f.cos())),
            _ => Err(PyError::type_error("cos() argument must be a number")),
        }
    });
    d
}
