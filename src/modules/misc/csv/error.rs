use crate::object::*;

pub fn dialect_error(msg: String) -> PyError {
    PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&msg)], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))
}
