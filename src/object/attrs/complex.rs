// Auto-extracted from src/object/attrs/mod.rs lines 750-785
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Complex(re, im) => match name {
                "real" => Ok(py_float(*re)),
                "imag" => Ok(py_float(*im)),
                "conjugate" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "conjugate".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        match &*obj {
                            PyObject::Complex(re, im) => {
                                Ok(PyObjectRef::imm(PyObject::Complex(*re, -im)))
                            }
                            _ => Err(PyError::type_error("conjugate() requires a complex self")),
                        }
                    },
                    self_obj: PyObjectRef::imm(PyObject::Complex(*re, *im)),
                })),
                "__complex__" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "__complex__".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        match &*obj {
                            PyObject::Complex(re, im) => {
                                Ok(PyObjectRef::imm(PyObject::Complex(*re, *im)))
                            }
                            _ => Err(PyError::type_error("__complex__() requires a complex self")),
                        }
                    },
                    self_obj: PyObjectRef::imm(PyObject::Complex(*re, *im)),
                })),
                "__float__" => Err(PyError::type_error("can't convert complex to float")),
                "__int__" => Err(PyError::type_error("can't convert complex to int")),
                _ => Err(PyError::attribute_error(format!(
                    "'complex' object has no attribute '{}'",
                    name
                ))),
            },
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
