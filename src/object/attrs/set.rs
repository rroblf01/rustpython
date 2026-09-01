// Auto-extracted from src/object/attrs/mod.rs lines 3395-3843
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Set(_s) => {
                match name {
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
                    "add" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "add".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "add() takes exactly one argument",
                                ));
                            }
                            pyset_safe_add(&args[0], args[1].clone())?;
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "remove" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "remove".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "remove() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                set.remove(&args[1])
                            } else {
                                Err(PyError::runtime_error("remove on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "discard" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "discard".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "discard() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                let _ = set.remove(&args[1]);
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("discard on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                set.pop()
                                    .ok_or_else(|| PyError::key_error("pop from an empty set"))
                            } else {
                                Err(PyError::runtime_error("pop on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                set.clear();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("clear on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                Ok(PyObjectRef::new(PyObject::Set(set.clone())))
                            } else {
                                Err(PyError::runtime_error("copy on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Set(set) = &*args[0].borrow() {
                                Ok(py_int(72 + (set.len() as i64) * 8))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "union" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "union".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "union() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let mut result = set.clone();
                                // Real `set.union(*others)` accepts ANY
                                // iterable per argument, not just another
                                // set — `convert_to_set` matches
                                // `issubset`/`issuperset`'s already-correct
                                // handling just below. Real trigger:
                                // CPython's own `test_compare.py`, which
                                // calls these against frozensets/lists.
                                for other_arg in &args[1..] {
                                    let other_set = convert_to_set(other_arg)?;
                                    for item in other_set.to_vec() {
                                        result.add(item)?;
                                    }
                                }
                                Ok(PyObjectRef::new(PyObject::Set(result)))
                            } else {
                                Err(PyError::runtime_error("union on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "intersection" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "intersection".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "intersection() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let others: Vec<PySet> = args[1..]
                                    .iter()
                                    .map(convert_to_set)
                                    .collect::<PyResult<_>>()?;
                                let mut result = PySet::new();
                                for item in set.to_vec() {
                                    let in_all = others.iter().all(|other_set| {
                                        other_set.contains(&item).unwrap_or(false)
                                    });
                                    if in_all {
                                        result.add(item)?;
                                    }
                                }
                                Ok(PyObjectRef::new(PyObject::Set(result)))
                            } else {
                                Err(PyError::runtime_error("intersection on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "difference" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "difference".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "difference() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let others: Vec<PySet> = args[1..]
                                    .iter()
                                    .map(convert_to_set)
                                    .collect::<PyResult<_>>()?;
                                let mut result = PySet::new();
                                for item in set.to_vec() {
                                    let in_any = others.iter().any(|other_set| {
                                        other_set.contains(&item).unwrap_or(false)
                                    });
                                    if !in_any {
                                        result.add(item)?;
                                    }
                                }
                                Ok(PyObjectRef::new(PyObject::Set(result)))
                            } else {
                                Err(PyError::runtime_error("difference on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "symmetric_difference" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "symmetric_difference".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "symmetric_difference() takes exactly one argument",
                                ));
                            }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
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
                                Ok(PyObjectRef::new(PyObject::Set(result)))
                            } else {
                                Err(PyError::runtime_error("symmetric_difference on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "issubset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issubset".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "issubset() takes exactly one argument",
                                ));
                            }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let other_set = convert_to_set(&args[1])?;
                                Ok(py_bool(
                                    set.to_vec()
                                        .iter()
                                        .all(|item| other_set.contains(item).unwrap_or(false)),
                                ))
                            } else {
                                Err(PyError::runtime_error("issubset on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "issuperset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "issuperset".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "issuperset() takes exactly one argument",
                                ));
                            }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                let other_set = convert_to_set(&args[1])?;
                                Ok(py_bool(
                                    other_set
                                        .to_vec()
                                        .iter()
                                        .all(|item| set.contains(item).unwrap_or(false)),
                                ))
                            } else {
                                Err(PyError::runtime_error("issuperset on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdisjoint" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdisjoint".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "isdisjoint() takes exactly one argument",
                                ));
                            }
                            let s = args[0].borrow();
                            if let PyObject::Set(set) = &*s {
                                // Real `set.isdisjoint(other)` accepts ANY
                                // iterable, not just another set — matches
                                // `issuperset`/`issubset` just above, which
                                // already correctly use `convert_to_set`
                                // instead of a narrow `PyObject::Set`-only
                                // match. Real trigger: CPython's own
                                // `test_compare.py`, which calls
                                // `isdisjoint()` against frozensets/lists.
                                let other_set = convert_to_set(&args[1])?;
                                Ok(py_bool(
                                    !set.to_vec()
                                        .iter()
                                        .any(|item| other_set.contains(item).unwrap_or(false)),
                                ))
                            } else {
                                Err(PyError::runtime_error("isdisjoint on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "update".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "update() takes at least 1 argument",
                                ));
                            }
                            if !matches!(&*args[0].borrow(), PyObject::Set(_)) {
                                return Err(PyError::runtime_error("update on non-set"));
                            }
                            // Each item is added via `pyset_safe_add`, which never
                            // holds `args[0]`'s own borrow across an `.equals()`
                            // call (unlike the old `args[0].borrow_mut()`-for-the-
                            // whole-loop version) — see its doc comment for why.
                            // Real `set.update(*others)` accepts ANY iterable per
                            // argument (frozenset, list, tuple, ...), not just
                            // another set — matches `issubset`/`issuperset`'s
                            // already-correct `convert_to_set` handling.
                            for other_arg in &args[1..] {
                                let items = convert_to_set(other_arg)?.to_vec();
                                for item in items {
                                    pyset_safe_add(&args[0], item)?;
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "intersection_update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "intersection_update".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "intersection_update() takes at least 1 argument",
                                ));
                            }
                            let others: Vec<PySet> = args[1..]
                                .iter()
                                .map(convert_to_set)
                                .collect::<PyResult<_>>()?;
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                let items: Vec<PyObjectRef> = set
                                    .to_vec()
                                    .iter()
                                    .filter(|item| {
                                        others.iter().all(|other_set| {
                                            other_set.contains(item).unwrap_or(false)
                                        })
                                    })
                                    .cloned()
                                    .collect();
                                set.clear();
                                for item in items {
                                    set.add(item)?;
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("intersection_update on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "difference_update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "difference_update".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "difference_update() takes at least 1 argument",
                                ));
                            }
                            let others: Vec<PySet> = args[1..]
                                .iter()
                                .map(convert_to_set)
                                .collect::<PyResult<_>>()?;
                            if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                let items: Vec<PyObjectRef> = set
                                    .to_vec()
                                    .iter()
                                    .filter(|item| {
                                        !others.iter().any(|other_set| {
                                            other_set.contains(item).unwrap_or(false)
                                        })
                                    })
                                    .cloned()
                                    .collect();
                                set.clear();
                                for item in items {
                                    set.add(item)?;
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("difference_update on non-set"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "symmetric_difference_update" => {
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "symmetric_difference_update".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error(
                                        "symmetric_difference_update() takes exactly one argument",
                                    ));
                                }
                                let other_set = convert_to_set(&args[1])?;
                                if let PyObject::Set(set) = &mut *args[0].borrow_mut() {
                                    for item in other_set.to_vec() {
                                        if set.contains(&item).unwrap_or(false) {
                                            set.remove(&item)?;
                                        } else {
                                            set.add(item)?;
                                        }
                                    }
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error(
                                        "symmetric_difference_update on non-set",
                                    ))
                                }
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    "__len__" => {
                        let len = _s.len() as i64;
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_int(len))
                            },
                        ))))
                    }
                    "__iter__" => {
                        let set_clone = _s.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                crate::object::builtin_iter(&[PyObjectRef::new(PyObject::Set(set_clone.clone()))])
                            },
                        ))))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'set' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
