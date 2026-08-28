// Auto-extracted from src/object/attrs/mod.rs lines 7490-7580
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Slice { start, stop, step } => {
                match name {
                    // A real `slice`'s `.start`/`.stop`/`.step` return
                    // WHATEVER object was actually passed to the `slice()`
                    // constructor, unchanged (real Python slices can hold
                    // arbitrary objects, not just ints — a documented,
                    // if less common, pattern; e.g. custom `__index__`
                    // objects or, as `test_slice.py::test_members` checks
                    // directly, a totally arbitrary object with no numeric
                    // meaning at all: `slice(obj).stop is obj`). This used
                    // to force EVERY non-`None` value through
                    // `.as_i64().unwrap_or(0)` — silently replacing any
                    // non-integer stored value with `0` (or `1` for
                    // `step`) instead of returning it, breaking both
                    // `test_members`'s arbitrary-object case and
                    // `test_deepcopy`'s mutable-index case (`slice([1,2],
                    // [3,4], [5,6])` — reading `.start` back never
                    // returned the actual list at all).
                    "start" => Ok(start.clone()),
                    "stop" => Ok(stop.clone()),
                    "step" => Ok(step.clone()),
                    "indices" => {
                        let start_ref = start.clone();
                        let stop_ref = stop.clone();
                        let step_ref = step.clone();
                        Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                if args.is_empty() {
                                    return Err(PyError::type_error(
                                        "indices() takes exactly 1 argument",
                                    ));
                                }
                                // Components can be ANY int (huge ones beyond
                                // i64 — real test_slice.py::test_indices sweeps
                                // values up to 2**100) or an `__index__` object;
                                // a float / arbitrary object must raise
                                // TypeError. Parsed at CALL time (accessing
                                // `.indices` must never validate the components).
                                let comp = |v: &PyObjectRef| -> PyResult<num_bigint::BigInt> {
                                    crate::object::to_index(v).map_err(|_| PyError::type_error(
                                    "slice indices must be integers or None or have an __index__ method"
                                ))
                                };
                                let length = comp(&args[0])?;
                                if length.sign() == num_bigint::Sign::Minus {
                                    return Err(PyError::value_error(
                                        "length should not be negative",
                                    ));
                                }
                                let (rs, re, st) = crate::object::subscript::slice_indices_values(
                                    &start_ref, &stop_ref, &step_ref, &length,
                                )?;
                                return Ok(py_tuple(vec![py_int(rs), py_int(re), py_int(st)]));
                            },
                        ))))
                    }
                    "__hash__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__hash__".to_string(),
                        func: |args| {
                            if let PyObject::Slice { start, stop, step } = &*args[0].borrow() {
                                let h = args[0].hash()?;
                                Ok(py_int(h as i64))
                            } else {
                                Err(PyError::runtime_error("__hash__ on non-slice"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__reduce__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__reduce__".to_string(),
                        func: |args| {
                            if let PyObject::Slice { start, stop, step } = &*args[0].borrow() {
                                Ok(py_tuple(vec![
                                    PyObjectRef::imm(PyObject::BuiltinFunction {
                                        name: "slice".to_string(),
                                        func: builtin_slice,
                                    }),
                                    py_tuple(vec![start.clone(), stop.clone(), step.clone()]),
                                ]))
                            } else {
                                Err(PyError::runtime_error("__reduce__ on non-slice"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'slice' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
