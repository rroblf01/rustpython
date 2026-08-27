use crate::object::*;
use std::collections::HashMap;

pub fn create_json_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! json_func {
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
    json_func!("dumps", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("dumps() missing required argument"));
        }
        let indent = if args.len() > 1 {
            let v = args[1].as_i64().unwrap_or(-1);
            if v >= 0 { Some(v as usize) } else { None }
        } else { None };
        let sort_keys = if args.len() > 2 { args[2].truthy() } else { false };
        json_encode_full(&args[0], indent, sort_keys, 0)
    });
    json_func!("loads", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("loads() missing required argument"));
        }
        let s = args[0].str();
        json_decode(&s)
    });
    d
}
pub const JSON_EXTRA_SOURCE: &str = include_str!("../json_extra.py");
