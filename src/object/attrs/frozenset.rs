// Auto-extracted from src/object/attrs/mod.rs lines 7280-7489
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::FrozenSet(_items) => {
                match name {
                    "issuperset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issuperset".to_string(),
                        func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            let self_set = if let PyObject::FrozenSet(s) = &*args[0].borrow() {
                                s.clone()
                            } else if let PyObject::Set(s) = &*args[0].borrow() {
                                s.clone()
                            } else {
                                return Err(PyError::type_error(
                                    "issuperset requires a set/frozenset",
                                ));
                            };
                            let other = if args.len() < 2 {
                                return Err(PyError::type_error("issuperset requires 1 argument"));
                            } else {
                                &args[1]
                            };
                            let other_set = convert_to_set(other)?;
                            Ok(py_bool(self_set.is_superset(&other_set)))
                        },
                        self_obj: py_none(),
                    })),
                    "issubset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issubset".to_string(),
                        func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            let self_set = if let PyObject::FrozenSet(s) = &*args[0].borrow() {
                                s.clone()
                            } else if let PyObject::Set(s) = &*args[0].borrow() {
                                s.clone()
                            } else {
                                return Err(PyError::type_error(
                                    "issubset requires a set/frozenset",
                                ));
                            };
                            let other = if args.len() < 2 {
                                return Err(PyError::type_error("issubset requires 1 argument"));
                            } else {
                                &args[1]
                            };
                            let other_set = convert_to_set(other)?;
                            Ok(py_bool(self_set.is_subset(&other_set)))
                        },
                        self_obj: py_none(),
                    })),
                    // Needed for the extremely common `frozenset(x).__contains__`
                    // idiom (a bound method used as a first-class predicate
                    // value, not called directly) — real CPython's own
                    // `Lib/keyword.py` does exactly this for `iskeyword`.
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__contains__() takes exactly one argument",
                                ));
                            }
                            if let PyObject::FrozenSet(set) = &*args[0].borrow() {
                                Ok(py_bool(set.contains(&args[1])?))
                            } else {
                                Err(PyError::runtime_error("__contains__ on non-frozenset"))
                            }
                        },
                        self_obj: py_none(),
                    })),
                    // `frozenset` was missing its own `union`/`intersection`/
                    // `difference`/`symmetric_difference`/`isdisjoint`/`copy`
                    // entirely (only `issuperset`/`issubset`/`__contains__`
                    // existed above) — real trigger: CPython's own
                    // `test_compare.py`, which exercises these against
                    // frozensets directly. No `*_update` variants: frozenset
                    // is immutable, those don't apply. Each mirrors `set`'s
                    // own implementation (just above, `PyObject::Set`'s
                    // match arm) but always produces a `FrozenSet` result.
                    "union" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "union".to_string(),
                        func: |args| {
                            if let PyObject::FrozenSet(set) = &*args[0].borrow() {
                                let mut result = set.clone();
                                for other_arg in &args[1..] {
                                    let other_set = convert_to_set(other_arg)?;
                                    for item in other_set.to_vec() {
                                        result.add(item)?;
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::FrozenSet(result)))
                            } else {
                                Err(PyError::runtime_error("union on non-frozenset"))
                            }
                        },
                        self_obj: py_none(),
                    })),
                    "intersection" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "intersection".to_string(),
                        func: |args| {
                            if let PyObject::FrozenSet(set) = &*args[0].borrow() {
                                let others: Vec<PySet> = args[1..]
                                    .iter()
                                    .map(convert_to_set)
                                    .collect::<PyResult<_>>()?;
                                let mut result = PySet::new();
                                for item in set.to_vec() {
                                    if others
                                        .iter()
                                        .all(|other_set| other_set.contains(&item).unwrap_or(false))
                                    {
                                        result.add(item)?;
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::FrozenSet(result)))
                            } else {
                                Err(PyError::runtime_error("intersection on non-frozenset"))
                            }
                        },
                        self_obj: py_none(),
                    })),
                    "difference" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "difference".to_string(),
                        func: |args| {
                            if let PyObject::FrozenSet(set) = &*args[0].borrow() {
                                let others: Vec<PySet> = args[1..]
                                    .iter()
                                    .map(convert_to_set)
                                    .collect::<PyResult<_>>()?;
                                let mut result = PySet::new();
                                for item in set.to_vec() {
                                    if !others
                                        .iter()
                                        .any(|other_set| other_set.contains(&item).unwrap_or(false))
                                    {
                                        result.add(item)?;
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::FrozenSet(result)))
                            } else {
                                Err(PyError::runtime_error("difference on non-frozenset"))
                            }
                        },
                        self_obj: py_none(),
                    })),
                    "symmetric_difference" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "symmetric_difference".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "symmetric_difference() takes exactly one argument",
                                ));
                            }
                            if let PyObject::FrozenSet(set) = &*args[0].borrow() {
                                let other_set = convert_to_set(&args[1])?;
                                let mut result = PySet::new();
                                for item in set.to_vec() {
                                    if !other_set.contains(&item).unwrap_or(false) {
                                        result.add(item)?;
                                    }
                                }
                                for item in other_set.to_vec() {
                                    if !set.contains(&item).unwrap_or(false) {
                                        result.add(item)?;
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::FrozenSet(result)))
                            } else {
                                Err(PyError::runtime_error(
                                    "symmetric_difference on non-frozenset",
                                ))
                            }
                        },
                        self_obj: py_none(),
                    })),
                    "isdisjoint" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdisjoint".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "isdisjoint() takes exactly one argument",
                                ));
                            }
                            if let PyObject::FrozenSet(set) = &*args[0].borrow() {
                                let other_set = convert_to_set(&args[1])?;
                                Ok(py_bool(
                                    !set.to_vec()
                                        .iter()
                                        .any(|item| other_set.contains(item).unwrap_or(false)),
                                ))
                            } else {
                                Err(PyError::runtime_error("isdisjoint on non-frozenset"))
                            }
                        },
                        self_obj: py_none(),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            if let PyObject::FrozenSet(set) = &*args[0].borrow() {
                                Ok(PyObjectRef::imm(PyObject::FrozenSet(set.clone())))
                            } else {
                                Err(PyError::runtime_error("copy on non-frozenset"))
                            }
                        },
                        self_obj: py_none(),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'{}' object has no attribute '{}'",
                        o.type_name(),
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
