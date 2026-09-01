use crate::object::*;
use std::collections::HashMap;

pub fn create_logging_config_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! log_cfg_func {
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
    log_cfg_func!("dictConfig", |_args| {
        // Simplified stub: accepts a dict but does nothing
        // A full implementation would configure loggers, handlers, formatters from the dict
        Ok(py_none())
    });
    d
}
