use crate::object::*;
use std::collections::HashMap;

pub fn create_contextlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ctx_func {
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
    ctx_func!("contextmanager", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("contextmanager() missing argument"));
        }
        Ok(args[0].clone())
    });
    ctx_func!("nullcontext", |args| {
        if args.is_empty() {
            Ok(py_none())
        } else {
            Ok(args[0].clone())
        }
    });
    ctx_func!("suppress", |_| Ok(py_none()));
    d
}

/// ContextDecorator source — see VirtualMachine::install_source_defined_stdlib.
pub const CONTEXTLIB_SOURCE: &str = include_str!("../contextlib_extra.py");
