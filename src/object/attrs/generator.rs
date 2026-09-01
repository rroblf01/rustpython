// Auto-extracted from src/object/attrs/mod.rs lines 4190-4228
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Generator { frame: _gen_frame } => match name {
                "__next__" | "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: name.to_string(),
                    func: generator_next_fallback,
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "throw" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "throw".to_string(),
                    func: generator_throw_fallback,
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "close".to_string(),
                    func: |args| {
                        let gen = args[0].borrow();
                        if let PyObject::Generator { frame } = &*gen {
                            if let Ok(mut frame_opt) = frame.try_borrow_mut() {
                                *frame_opt = None;
                            }
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
                _ => Err(PyError::attribute_error(format!(
                    "'generator' object has no attribute '{}'",
                    name
                ))),
            },
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
