// Auto-extracted from src/object/attrs/mod.rs lines 750-785
use super::*;
use crate::interner;
use crate::object::*;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
        PyObject::Complex(re, im) => match name {
            "real" => Ok(py_float(*re)),
            "imag" => Ok(py_float(*im)),
            "conjugate" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                name: "conjugate".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    match &*obj {
                        PyObject::Complex(re, im) => {
                            Ok(PyObjectRef::imm(PyObject::Complex(*re, -im)))
                        }
                        _ => Err(PyError::type_error("conjugate() requires a complex self")),
                    }
                },
                self_obj: PyObjectRef::imm(PyObject::Complex(*re, *im)),
            })),
            "__complex__" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                name: "__complex__".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    match &*obj {
                        PyObject::Complex(re, im) => {
                            Ok(PyObjectRef::imm(PyObject::Complex(*re, *im)))
                        }
                        _ => Err(PyError::type_error("__complex__() requires a complex self")),
                    }
                },
                self_obj: PyObjectRef::imm(PyObject::Complex(*re, *im)),
            })),
            "__float__" => Err(PyError::type_error("can't convert complex to float")),
            "__int__" => Err(PyError::type_error("can't convert complex to int")),
"__getnewargs__" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                name: "__getnewargs__".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    match &*obj {
                        PyObject::Complex(re, im) => {
                            Ok(py_tuple(vec![py_float(*re), py_float(*im)]))
                        }
                        _ => Err(PyError::type_error("__getnewargs__() requires a complex self")),
                    }
                },
                self_obj: PyObjectRef::imm(PyObject::Complex(*re, *im)),
            })),
"__format__" => {
                // format(complex, spec) / complex.__format__(spec) — native
                // complex instance lookup doesn't fall back to the type dict
                // (test_format: "'complex' object has no attribute
                // '__format__'"). Also validates specs invalid for complex
                // (zero-pad, '=' align, integer types → ValueError, test_format).
                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__format__".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "__format__ requires a format-spec argument",
                            ));
                        }
                        let spec = args[1].str();
                        // Invalid for complex: '=' align, integer types
                        // (b c d o x X), zero-padding ('0' as the width).
                        if spec.contains('=') {
                            return Err(PyError::value_error(
                                "Invalid format specifier '='",
                            ));
                        }
                        if let Some(t) = spec.chars().last() {
                            if matches!(t, 'b' | 'c' | 'd' | 'o' | 'x' | 'X') {
                                return Err(PyError::value_error(format!(
                                    "Invalid format specifier '{}'",
                                    t
                                )));
                            }
                        }
                        // Zero-padding: '0' as the first width digit (after
                        // optional fill/align/sign) — e.g. '010f'.
                        {
                            let mut idx = 0;
                            let chars: Vec<char> = spec.chars().collect();
                            // skip fill+align (up to2 chars if2nd is align)
                            if idx < chars.len() && chars[idx] == '=' {
                                // already handled above
                            }
                            if idx + 1 < chars.len()
                                && matches!(chars[idx + 1], '<' | '^' | '>')
                            {
                                idx += 2; // fill + align
                            } else if idx < chars.len()
                                && matches!(chars[idx], '<' | '^' | '>')
                            {
                                idx += 1; // align
                            }
                            if idx < chars.len()
                                && matches!(chars[idx], '+' | '-' | ' ')
                            {
                                idx += 1; // sign
                            }
                            if idx < chars.len()
                                && chars[idx] == '0'
                                && idx + 1 < chars.len()
                                && chars[idx + 1].is_ascii_digit()
                            {
                                return Err(PyError::value_error(
                                    "Zero padding is not allowed in complex format specifier",
                                ));
                            }
                        }
                        let result = crate::vm::format_with_spec(&args[0], &spec)?;
                        Ok(py_str(&result))
                    },
                    self_obj: PyObjectRef::imm(PyObject::Complex(*re, *im)),
                }))
            }
            _ => Err(PyError::attribute_error(format!(
                "'complex' object has no attribute '{}'",
                name
            ))),
        },
        _ => unreachable!(
            "{} called with non-{} object",
            stringify!(get),
            o.type_name()
        ),
    }
}
