use crate::object::*;
use std::collections::HashMap;

pub fn create_traceback_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tb_func {
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

    tb_func!("format_exc", |_| { Ok(py_str("")) });

    tb_func!("print_exc", |_| {
        println!("No traceback");
        Ok(py_none())
    });

    d
}
