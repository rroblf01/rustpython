use crate::object::*;
use std::collections::HashMap;

pub fn create_json_tool_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! jt_func {
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

    jt_func!("main", |_args| {
        // Read all of stdin
        let mut input = String::new();
        use std::io::Read;
        match std::io::stdin().read_to_string(&mut input) {
            Ok(_) => {
                // Parse JSON
                let parsed = json_decode(&input)?;
                // Pretty-print with indent=2
                let formatted = json_encode_full(&parsed, Some(2), true, 0)?;
                // Print to stdout
                println!("{}", formatted.str());
                Ok(py_none())
            }
            Err(e) => Err(PyError::runtime_error(format!(
                "json.tool error reading stdin: {}",
                e
            ))),
        }
    });

    d
}
