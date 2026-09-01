use crate::object::*;
use std::collections::HashMap;

pub fn create_linecache_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! lc_func {
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

    lc_func!("getline", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "getline() requires at least 2 arguments (filename, lineno)",
            ));
        }
        Ok(py_str(""))
    });

    lc_func!("clearcache", |_| { Ok(py_none()) });

    lc_func!("checkcache", |_| { Ok(py_none()) });

    d
}
