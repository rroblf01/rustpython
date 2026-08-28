use crate::object::*;
use std::collections::HashMap;

pub fn create_array_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Array type as a factory function
    d.insert_str(
        "array",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "array".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "array() requires at least 1 argument (typecode)",
                    ));
                }
                let typecode_str = args[0].str();
                if typecode_str.is_empty() {
                    return Err(PyError::value_error("empty typecode".to_string()));
                }
                let typecode = typecode_str.chars().next().unwrap();
                // Real Python's `array` module accepts all of `bBuhHiIlLqQfd` —
                // this previously only recognized 'i'/'f'/'d', raising
                // `ValueError: bad typecode` for anything else (e.g. `array
                // .array('B', ...)`, an extremely common "typed byte buffer"
                // idiom used throughout CPython's own test suite as setup/helper
                // code, not something specific to `array` itself). `PyArray`
                // stores every element as `f64` regardless of typecode (a
                // simplification — real per-typecode overflow/wraparound
                // semantics and `itemsize` aren't modeled), but that's already
                // true for the 'i' case this accepted before; broadening which
                // typecodes are ACCEPTED (and read back as `int` vs `float` per
                // `array_typecode_is_float` below) fixes the far more common
                // "construction rejected outright" failure mode.
                if !"bBuhHiIlLqQfdwu".contains(typecode) {
                    return Err(PyError::value_error(format!("bad typecode '{}'", typecode)));
                }
                let is_float = array_typecode_is_float(typecode);
                let is_unicode = typecode == 'w' || typecode == 'u';
                let mut data: Vec<f64> = Vec::new();
                if args.len() > 1 {
                    let init = &args[1];
                    let init_borrowed = init.borrow();
                    match &*init_borrowed {
                        PyObject::List(items) => {
                            for item in items {
                                if is_float {
                                    data.push(item.as_f64().unwrap_or(0.0));
                                } else if is_unicode {
                                    let s = item.str();
                                    let ch = s.chars().next().unwrap_or('\0') as u32 as f64;
                                    data.push(ch);
                                } else {
                                    data.push(item.as_i64().unwrap_or(0) as f64);
                                }
                            }
                        }
                        PyObject::Tuple(items) => {
                            for item in items {
                                if is_float {
                                    data.push(item.as_f64().unwrap_or(0.0));
                                } else if is_unicode {
                                    let s = item.str();
                                    let ch = s.chars().next().unwrap_or('\0') as u32 as f64;
                                    data.push(ch);
                                } else {
                                    data.push(item.as_i64().unwrap_or(0) as f64);
                                }
                            }
                        }
                        PyObject::Str(s) if is_unicode => {
                            for ch in s.chars() {
                                data.push(ch as u32 as f64);
                            }
                        }
                        _ => {
                            // Try iterating
                            let iter_obj = builtin_iter(&[init.clone()])?;
                            loop {
                                match builtin_next(&[iter_obj.clone()]) {
                                    Ok(item) => {
                                        if is_float {
                                            data.push(item.as_f64().unwrap_or(0.0));
                                        } else if is_unicode {
                                            let s = item.str();
                                            let ch = s.chars().next().unwrap_or('\0') as u32 as f64;
                                            data.push(ch);
                                        } else {
                                            data.push(item.as_i64().unwrap_or(0) as f64);
                                        }
                                    }
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                }
                Ok(PyObjectRef::new(PyObject::Array(PyArray {
                    typecode,
                    data,
                })))
            },
        }),
    );

    d
}
