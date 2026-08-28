// Auto-extracted from src/object/attrs/mod.rs lines 6427-6649
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Int(_) | PyObject::Bool(_) => {
                let int_value =
                    int_or_bool_value(&PyObjectRef::new(o.clone())).unwrap_or_default();
                match name {
                    "__bool__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__bool__".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                Ok(py_bool(!v.is_zero()))
                            } else {
                                Err(PyError::runtime_error("__bool__ on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__float__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__float__".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                Ok(py_float(v.to_f64().unwrap_or(0.0)))
                            } else {
                                Err(PyError::runtime_error("__float__ on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "bit_length" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "bit_length".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                Ok(py_int(v.bits() as i64))
                            } else {
                                Err(PyError::runtime_error("bit_length on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "bit_count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "bit_count".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                let count: u32 = if v < num_bigint::BigInt::from(0) {
                                    let neg = -(v + 1i32);
                                    neg.to_bytes_le().1.iter().map(|b| b.count_ones()).sum()
                                } else {
                                    v.to_bytes_le().1.iter().map(|b| b.count_ones()).sum()
                                };
                                Ok(py_int(count as i64))
                            } else {
                                Err(PyError::runtime_error("bit_count on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `int`'s share of the `numbers.Rational`/`Integral` ABC
                    // protocol (`as_integer_ratio`/`numerator`/`denominator`
                    // /`real`/`imag`) — an int IS its own numerator with
                    // denominator 1, and its own real part with a zero
                    // imaginary part, matching real CPython exactly. Needed
                    // by any code walking the numeric tower generically
                    // (real trigger: CPython's own `Lib/statistics.py`'s
                    // `_exact_ratio`, which tries `x.as_integer_ratio()`
                    // then falls back to `(x.numerator, x.denominator)` —
                    // both raised `AttributeError` before this, since only
                    // `float`/`Fraction` had `as_integer_ratio` and nothing
                    // implemented the ABC-style numerator/denominator pair).
                    "as_integer_ratio" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "as_integer_ratio".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                Ok(py_tuple(vec![py_int(v.clone()), py_int(1)]))
                            } else {
                                Err(PyError::runtime_error("as_integer_ratio on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "numerator" | "real" => Ok(py_int(int_value.clone())),
                    "denominator" => Ok(py_int(1)),
                    "imag" => Ok(py_int(0)),
                    // `int.conjugate()` — part of the same `numbers.Complex`
                    // protocol as `float`'s arm just above; a plain int is
                    // trivially its own conjugate. Missing before, raising
                    // `AttributeError` (real trigger: CPython's own
                    // `test_abstract_numbers.py`).
                    "conjugate" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "conjugate".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                Ok(py_int(v.clone()))
                            } else {
                                Err(PyError::runtime_error("conjugate on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::imm(PyObject::Int(int_value.clone())),
                    })),
                    // `int.__round__()`/`float.__round__()` — `round()` the
                    // builtin already works, but wasn't accessible as a
                    // named dunder (real trigger: CPython's own
                    // `test_int.py`/`test_float.py`, both directly calling
                    // `x.__round__(...)`).
                    "__round__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__round__".to_string(),
                        func: |args| builtin_round(args),
                        self_obj: PyObjectRef::imm(PyObject::Int(int_value.clone())),
                    })),
                    "to_bytes" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "to_bytes".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "to_bytes() takes at least 2 arguments (1 given)",
                                ));
                            }
                            if let PyObject::Int(val) = &*args[0].borrow() {
                                let length = args[1]
                                    .as_i64()
                                    .ok_or_else(|| PyError::type_error("length must be int"))?;
                                let byteorder = args[2].str();
                                let signed = if args.len() > 3 {
                                    args[3].truthy()
                                } else {
                                    false
                                };
                                if length <= 0 {
                                    return Err(PyError::type_error("length must be positive"));
                                }
                                let len = length as usize;
                                let (_, bytes) = if byteorder == "little" {
                                    val.to_bytes_le()
                                } else {
                                    val.to_bytes_be()
                                };
                                // Handle negative numbers for signed=True
                                if signed && val.sign() == Sign::Minus {
                                    // For signed negative, compute two's complement
                                    let abs_val = -val.clone();
                                    let (_, abs_bytes) = if byteorder == "little" {
                                        abs_val.to_bytes_le()
                                    } else {
                                        abs_val.to_bytes_be()
                                    };
                                    // Create two's complement
                                    let mut result = vec![0u8; len];
                                    for i in 0..abs_bytes.len().min(len) {
                                        result[if byteorder == "little" {
                                            i
                                        } else {
                                            len - 1 - i
                                        }] = abs_bytes[i];
                                    }
                                    // Two's complement: invert bits and add 1
                                    for b in result.iter_mut() {
                                        *b = !*b;
                                    }
                                    // Add 1
                                    let mut carry = 1u16;
                                    if byteorder == "little" {
                                        for b in result.iter_mut() {
                                            let v = *b as u16 + carry;
                                            *b = v as u8;
                                            carry = v >> 8;
                                        }
                                    } else {
                                        for b in result.iter_mut().rev() {
                                            let v = *b as u16 + carry;
                                            *b = v as u8;
                                            carry = v >> 8;
                                        }
                                    }
                                    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                                } else {
                                    // Pad or truncate to fit length
                                    if bytes.len() > len {
                                        return Err(PyError::type_error("int too big to convert"));
                                    }
                                    let mut result = vec![0u8; len];
                                    if byteorder == "little" {
                                        for i in 0..bytes.len() {
                                            result[i] = bytes[i];
                                        }
                                    } else {
                                        let offset = len - bytes.len();
                                        for i in 0..bytes.len() {
                                            result[offset + i] = bytes[i];
                                        }
                                    }
                                    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                                }
                            } else {
                                Err(PyError::runtime_error("to_bytes on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__index__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__index__".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                Ok(py_int(v.clone()))
                            } else {
                                Err(PyError::runtime_error("__index__ on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__int__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__int__".to_string(),
                        func: |args| {
                            if let Some(v) = int_or_bool_value(&args[0]) {
                                Ok(py_int(v.clone()))
                            } else {
                                Err(PyError::runtime_error("__int__ on non-int"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'int' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
