// Auto-extracted from src/object/attrs/mod.rs lines 1897-2064
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Tuple(_v) => {
                match name {
                    "__mul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__mul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__rmul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__rmul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__rmul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__reversed__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__reversed__".to_string(),
                        func: |args| {
                            if let PyObject::Tuple(tuple) = &*args[0].borrow() {
                                let mut rev = tuple.clone();
                                rev.reverse();
                                Ok(PyObjectRef::imm(PyObject::Tuple(rev)))
                            } else {
                                Err(PyError::runtime_error("__reversed__ on non-tuple"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Tuple(tuple) = &*args[0].borrow() {
                                Ok(py_int(40 + (tuple.len() as i64) * 8))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-tuple"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Same gap, same fix, as `list`'s own `__getitem__` arm.
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__getitem__() takes exactly one argument",
                                ));
                            }
                            py_getitem(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__contains__() takes exactly one argument",
                                ));
                            }
                            py_contains(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "count() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Tuple(tuple) = &*args[0].borrow() {
                                let mut n = 0i64;
                                for item in tuple.iter() {
                                    if py_compare(item, &args[1], 2)?.truthy() {
                                        n += 1;
                                    }
                                }
                                Ok(py_int(n))
                            } else {
                                Err(PyError::runtime_error("count on non-tuple"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "index".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "index() takes at least 1 argument",
                                ));
                            }
                            if args.len() > 4 {
                                return Err(PyError::type_error(format!(
                                    "index() takes at most 3 arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::Tuple(tuple) = &*args[0].borrow() {
                                // Clamp start/end with arbitrary-precision
                                // ints (huge bounds like 4*sys.maxsize must
                                // clamp, not silently collapse via as_i64).
                                use num_traits::ToPrimitive;
                                let len = tuple.len() as i64;
                                let clamp = |v: Option<&PyObjectRef>, default: i64| -> i64 {
                                    let Some(v) = v else {
                                        return default;
                                    };
                                    let n = crate::object::to_index(v).unwrap_or_else(|_| 0.into());
                                    let len_big = num_bigint::BigInt::from(len);
                                    let c = if n.sign() == num_bigint::Sign::Minus {
                                        (len_big.clone() + &n).max(0.into())
                                    } else {
                                        n.min(len_big.clone())
                                    };
                                    c.to_i64().unwrap_or(0)
                                };
                                let start = clamp(args.get(2), 0);
                                let end = clamp(args.get(3), len);
                                for i in start..end {
                                    if py_compare(&tuple[i as usize], &args[1], 2)?.truthy() {
                                        return Ok(py_int(i));
                                    }
                                }
                                Err(PyError::value_error("tuple.index(x): x not in tuple"))
                            } else {
                                Err(PyError::runtime_error("index on non-tuple"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => {
                        let len = _v.len() as i64;
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_int(len))
                            },
                        ))))
                    }
                    "__iter__" => {
                        let tuple_clone = _v.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                crate::object::builtin_iter(&[PyObjectRef::new(PyObject::Tuple(tuple_clone.clone()))])
                            },
                        ))))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'tuple' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
