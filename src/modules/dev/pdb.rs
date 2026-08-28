use crate::object::*;
use std::collections::HashMap;

pub fn create_pdb_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pdb_func {
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

    pdb_func!("set_trace", |_| {
        println!("Debugger not available");
        Ok(py_none())
    });

    d
}
