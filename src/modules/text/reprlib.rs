use crate::object::*;
use std::collections::HashMap;

pub fn create_reprlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "repr",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "repr".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("repr() missing required argument"));
                }
                let s = if args.len() > 1 {
                    let limit = args[1].as_i64().unwrap_or(80) as usize;
                    let obj_repr = args[0].repr();
                    if obj_repr.len() > limit {
                        if limit > 3 {
                            format!("{}...", &obj_repr[..limit - 3])
                        } else {
                            obj_repr[..limit].to_string()
                        }
                    } else {
                        obj_repr
                    }
                } else {
                    let obj_repr = args[0].repr();
                    if obj_repr.len() > 80 {
                        format!("{}...", &obj_repr[..77])
                    } else {
                        obj_repr
                    }
                };
                Ok(py_str(&s))
            },
        }),
    );
    d
}
