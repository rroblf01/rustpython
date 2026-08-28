use crate::object::*;
use std::collections::HashMap;

pub fn create_opcode_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("ENABLE_SPECIALIZATION", py_bool(false));
    d.insert_str("ENABLE_SPECIALIZATION_FT", py_bool(false));
    // stack_effect(opcode, oparg) -> int: return the stack effect of an opcode
    d.insert_str(
        "stack_effect",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "stack_effect".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "stack_effect() missing required argument",
                    ));
                }
                // Return a conservative estimate (2 for most ops, 0 for simple)
                let opcode_str = args[0].str();
                match opcode_str.as_str() {
                    "RETURN_VALUE" | "POP_TOP" => Ok(py_int(-1)),
                    "LOAD_CONST" | "LOAD_FAST" | "LOAD_NAME" | "LOAD_GLOBAL" | "LOAD_DEREF" => {
                        Ok(py_int(1))
                    }
                    "BUILD_LIST" | "BUILD_TUPLE" | "BUILD_SET" | "BUILD_MAP" | "BUILD_STRING" => {
                        Ok(py_int(
                            1 - args.get(1).and_then(|a| a.as_i64()).unwrap_or(1) as i64,
                        ))
                    }
                    "CALL" | "CALL_FUNCTION_EX" | "CALL_KW" => Ok(py_int(-1)),
                    _ => Ok(py_int(0)),
                }
            },
        }),
    );
    d
}
