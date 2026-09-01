// Auto-extracted from src/object/attrs/mod.rs lines 8359-8620
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Range { start, stop, step } => match name {
                "start" => Ok(py_int(start.clone())),
                "stop" => Ok(py_int(stop.clone())),
                "step" => Ok(py_int(step.clone())),
                "__reduce__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__reduce__".to_string(),
                    func: |args| {
                        let s: &PyObjectRef = &args[0];
                        if let PyObject::Range { start, stop, step } = &*s.borrow() {
                            Ok(py_tuple(vec![
                                PyObjectRef::imm(PyObject::BuiltinFunction {
                                    name: "range".to_string(),
                                    func: builtin_range,
                                }),
                                py_tuple(vec![
                                    py_int(start.clone()),
                                    py_int(stop.clone()),
                                    py_int(step.clone()),
                                ]),
                            ]))
                        } else {
                            Err(PyError::runtime_error("__reduce__ on non-range"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__iter__" => Ok(PyObjectRef::new(PyObject::RangeIter {
                    current: start.clone(),
                    stop: stop.clone(),
                    step: step.clone(),
                })),
                "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__contains__".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "__contains__() takes exactly one argument",
                            ));
                        }
                        let val = &args[1];
                        if let PyObject::Range { start, stop, step } = &*args[0].borrow() {
                            let mut current = start.clone();
                            while if step.sign() == num_bigint::Sign::Plus {
                                current < *stop
                            } else {
                                current > *stop
                            } {
                                let item = PyObjectRef::imm(PyObject::Int(current.clone()));
                                if py_compare(&item, val, 2).unwrap_or(py_bool(false)).truthy() {
                                    return Ok(py_bool(true));
                                }
                                current += step;
                            }
                        }
                        Ok(py_bool(false))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__len__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__len__".to_string(),
                    func: |args| {
                        if args.is_empty() {
                            return Err(PyError::type_error(
                                "__len__() takes exactly one argument",
                            ));
                        }
                        let obj = args[0].borrow();
                        if let PyObject::Range { start, stop, step } = &*obj {
                            let len =
                                crate::object::ops_contains::range_len_values(start, stop, step);
                            if len.to_i64().is_none() {
                                return Err(PyError::overflow_error(
                                    "Python int too large to convert to C ssize_t",
                                ));
                            }
                            Ok(py_int(len))
                        } else {
                            Err(PyError::runtime_error("__len__ on non-range"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "count".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("count() takes exactly 1 argument"));
                        }
                        let val = &args[1];
                        if let PyObject::Range { start, stop, step } = &*args[0].borrow() {
                            // O(1) for ints (CPython): 1 if the value is in
                            // the range, else 0 — never iterate a huge range.
                            if let Ok(n) = crate::object::to_index(val) {
                                let in_range = range_contains_bigint(start, stop, step, &n);
                                return Ok(py_int(if in_range { 1 } else { 0 }));
                            }
                            // Non-int: iterate with equality (matches CPython).
                            let mut count = 0i64;
                            let mut current = start.clone();
                            while if step.sign() == num_bigint::Sign::Plus {
                                current < *stop
                            } else {
                                current > *stop
                            } {
                                let item = PyObjectRef::imm(PyObject::Int(current.clone()));
                                if py_compare(&item, val, 2)?.truthy() {
                                    count += 1;
                                }
                                current += step;
                            }
                            return Ok(py_int(count));
                        }
                        Ok(py_int(0))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "index".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("index() takes at least 1 argument"));
                        }
                        let val = &args[1];
                        if let PyObject::Range { start, stop, step } = &*args[0].borrow() {
                            // O(1) for ints: position = (val - start) / step.
                            if let Ok(n) = crate::object::to_index(val) {
                                if range_contains_bigint(start, stop, step, &n) {
                                    let pos = (&n - start) / step;
                                    return Ok(py_int(pos.abs()));
                                }
                                return Err(PyError::value_error("value not in range"));
                            }
                            // Non-int: iterate with equality.
                            let mut current = start.clone();
                            let mut idx = 0i64;
                            while if step.sign() == num_bigint::Sign::Plus {
                                current < *stop
                            } else {
                                current > *stop
                            } {
                                let item = PyObjectRef::imm(PyObject::Int(current.clone()));
                                if py_compare(&item, val, 2)?.truthy() {
                                    return Ok(py_int(idx));
                                }
                                current += step;
                                idx += 1;
                            }
                            return Err(PyError::value_error("value not in range"));
                        }
                        Err(PyError::value_error("value not in range"))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__getitem__".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "__getitem__() takes exactly 1 argument",
                            ));
                        }
                        if let PyObject::Range { start, stop, step } = &*args[0].borrow() {
                            let idx = &args[1];
                            let length =
                                crate::object::ops_contains::range_len_values(start, stop, step);
                            if let PyObject::Slice {
                                start: s,
                                stop: e,
                                step: p,
                            } = &*idx.borrow()
                            {
                                let (norm_start, norm_stop, norm_step) =
                                    crate::object::subscript::slice_indices_values(
                                        s, e, p, &length,
                                    )?;
                                // Value-mapped sub-range: the sliced range's
                                // start/stop are the ORIGINAL values at the
                                // normalized positions, the step is the
                                // original step scaled by the slice's step.
                                let new_start = start + norm_start * step;
                                let new_step = step * norm_step;
                                let new_stop = start + norm_stop * step;
                                Ok(PyObjectRef::imm(PyObject::Range {
                                    start: new_start,
                                    stop: new_stop,
                                    step: new_step,
                                }))
                            } else {
                                let i = crate::object::to_index(&args[1]).map_err(|_| {
                                    PyError::type_error("range indices must be integers or slices")
                                })?;
                                let pos = if i.sign() == num_bigint::Sign::Minus {
                                    length.clone() + i
                                } else {
                                    i
                                };
                                let zero = num_bigint::BigInt::from(0);
                                if pos < zero || pos >= length {
                                    return Err(PyError::IndexError(
                                        "range object index out of range".to_string(),
                                    ));
                                }
                                Ok(py_int(start + step * pos))
                            }
                        } else {
                            Err(PyError::runtime_error("__getitem__ on non-range"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'range' object has no attribute '{}'",
                    name
                ))),
            },
            PyObject::RangeIter {
                current,
                stop,
                step,
            } => {
                match name {
                    "__length_hint__" => {
                        let zero = num_bigint::BigInt::from(0);
                        let remaining = if step.sign() == num_bigint::Sign::Plus {
                            (stop - current).max(zero)
                        } else {
                            (current - stop).max(zero)
                        };
                        Ok(py_int(remaining / step.abs()))
                    }
                    // Same `__next__`/`__iter__`-not-a-named-attribute gap
                    // as every other iterator shape (see the shared
                    // fallback arm below) — `RangeIter` needed its own case
                    // since it already has a dedicated match arm here (for
                    // `__length_hint__`) that would otherwise shadow the
                    // shared one.
                    "__next__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__next__".to_string(),
                        func: builtin_next,
                        self_obj: PyObjectRef::new(o.clone()),
                    })),
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: builtin_iter,
                        self_obj: PyObjectRef::new(o.clone()),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'range_iterator' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            // `it.__next__()`/`it.__iter__()` as NAMED attributes were
            // missing entirely for every one of this codebase's iterator
            // shapes (confirmed: `iter([1]).__next__()` raised
            // `AttributeError` despite `next(it)` — the builtin FUNCTION
            // form, which already correctly dispatches on each of these
            // same variants — working fine). Real trigger: CPython's own
            // `test_tokenize.py`, which calls `.__next__()` directly on a
            // `list_iterator`. Delegates to the already-correct
            // `builtin_next`/`builtin_iter` implementations rather than
            // duplicating their per-variant logic.
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
