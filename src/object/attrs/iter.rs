// Extracted from src/object/attrs/mod.rs — iterator attribute dispatch
// Covers `list_iterator` and the generic `__next__`/`__iter__` backed iterators
// (`map`/`filter`/`zip`/`cycle`/`groupby`/`enumerate`/`GetItemIter`/`CallSentinelIter`).
// Keeps mod.rs under 1k lines; this is exactly ~70 lines extracted (1069-70=999).
use super::*;
use crate::object::*;

fn dict_version_changed(dict: &PyObjectRef, expected_version: u64) -> bool {
    match &*dict.borrow() {
        PyObject::Dict(d) => d.version() != expected_version,
        _ => false,
    }
}

fn deque_len(deque: &PyObjectRef) -> Option<usize> {
    let native = crate::object::native_backing_of(deque).or_else(|| {
        if matches!(&*deque.borrow(), PyObject::Deque { .. }) {
            Some(deque.clone())
        } else {
            None
        }
    })?;
    let nb = native.borrow();
    if let PyObject::Deque { data, .. } = &*nb {
        Some(data.len())
    } else {
        None
    }
}

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
        PyObject::ListIter { list: _, index: _ } => match name {
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
            "__length_hint__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__length_hint__".to_string(),
                func: |args| {
                    if let PyObject::ListIter { list, index } = &*args[0].borrow() {
                        Ok(py_int(list.len().saturating_sub(*index) as i64))
                    } else {
                        Err(PyError::runtime_error(
                            "__length_hint__ on non-list_iterator",
                        ))
                    }
                },
                self_obj: PyObjectRef::new(o.clone()),
            })),
            // `os.scandir()` returns one of these, and every caller in the
            // stdlib writes `with os.scandir(path) as it:` — without these
            // three the whole block raised "object does not support the
            // context manager protocol".
            "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "close".to_string(),
                func: |_args| Ok(py_none()),
                self_obj: PyObjectRef::new(o.clone()),
            })),
            "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__enter__".to_string(),
                func: |args| Ok(args.first().cloned().unwrap_or_else(py_none)),
                self_obj: PyObjectRef::new(o.clone()),
            })),
            "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__exit__".to_string(),
                func: |_args| Ok(py_bool(false)),
                self_obj: PyObjectRef::new(o.clone()),
            })),
            "__setstate__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__setstate__".to_string(),
                func: crate::object::builtins::list_iter_setstate,
                self_obj: PyObjectRef::new(o.clone()),
            })),
            "__reduce__" | "__reduce_ex__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.to_string(),
                func: crate::object::builtins::list_iter_reduce,
                self_obj: PyObjectRef::new(o.clone()),
            })),
            _ => Err(PyError::attribute_error(format!(
                "'list_iterator' object has no attribute '{}'",
                name
            ))),
        },
        // `operator.length_hint()` on a `reversed(list)` / getitem
        // iterator: the number of items left, which for the REVERSE case
        // follows the walk index (so a later append is deliberately not
        // counted, matching CPython's list_reverseiterator).
        PyObject::GetItemIter { .. } if name == "__length_hint__" => {
            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__length_hint__".to_string(),
                func: |args| {
                    let obj = args.first().cloned().unwrap_or_else(py_none);
                    let borrowed = obj.borrow();
                    let (inner, index, reverse, exhausted) = match &*borrowed {
                        PyObject::GetItemIter {
                            obj,
                            index,
                            reverse,
                            exhausted,
                        } => (obj.clone(), *index, *reverse, *exhausted),
                        _ => return Ok(py_int(0)),
                    };
                    let total = crate::object::builtin_len(std::slice::from_ref(&inner))
                        .ok()
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if reverse {
                        // CPython's listreviter_length_hint also reports 0
                        // once the list has shrunk past the walk index
                        // (`d[1:] = []` mid-iteration), and stays 0 after
                        // that even if the list grows again.
                        if exhausted || index < 0 || index >= total {
                            return Ok(py_int(0));
                        }
                        return Ok(py_int(index + 1));
                    }
                    if exhausted {
                        return Ok(py_int(0));
                    }
                    Ok(py_int((total - index).max(0)))
                },
                self_obj: PyObjectRef::new(o.clone()),
            }))
        }
        PyObject::SetIter { .. } if name == "__next__" || name == "__iter__" => {
            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.to_string(),
                func: if name == "__next__" {
                    builtin_next
                } else {
                    builtin_iter
                },
                self_obj: PyObjectRef::new(o.clone()),
            }))
        }
        PyObject::SetIter { .. } if name == "__length_hint__" => {
            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__length_hint__".to_string(),
                func: |args| {
                    let obj = args.first().cloned().unwrap_or_else(py_none);
                    let borrowed = obj.borrow();
                    match &*borrowed {
                        PyObject::SetIter {
                            set,
                            index,
                            start_len,
                        } => {
                            let current = match &*set.borrow() {
                                PyObject::Set(s) => s.len(),
                                _ => 0,
                            };
                            if current != *start_len {
                                return Ok(py_int(0));
                            }
                            Ok(py_int((current - index.min(&current)).max(0)))
                        }
                        _ => Ok(py_int(0)),
                    }
                },
                self_obj: PyObjectRef::new(o.clone()),
            }))
        }
        PyObject::ListRefIter { .. } if name == "__next__" || name == "__iter__" => {
            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.to_string(),
                func: if name == "__next__" {
                    builtin_next
                } else {
                    builtin_iter
                },
                self_obj: PyObjectRef::new(o.clone()),
            }))
        }
        PyObject::ListRefIter { .. } if name == "__length_hint__" => {
            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__length_hint__".to_string(),
                func: |args| {
                    let obj = args.first().cloned().unwrap_or_else(py_none);
                    let borrowed = obj.borrow();
                    let (list, index, exhausted) = match &*borrowed {
                        PyObject::ListRefIter {
                            list,
                            index,
                            exhausted,
                        } => (list.clone(), *index, *exhausted),
                        _ => return Ok(py_int(0)),
                    };
                    if exhausted {
                        return Ok(py_int(0));
                    }
                    let total = crate::object::builtin_len(std::slice::from_ref(&list))
                        .ok()
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    Ok(py_int((total - index as i64).max(0)))
                },
                self_obj: PyObjectRef::new(o.clone()),
            }))
        }
        PyObject::MapIterator { .. }
        | PyObject::FilterIterator { .. }
        | PyObject::ZipIterator { .. }
        | PyObject::CycleIter { .. }
        | PyObject::GroupByIter { .. }
        | PyObject::EnumerateIter { .. }
        | PyObject::GetItemIter { .. }
        | PyObject::CallSentinelIter { .. }
        | PyObject::DictIter { .. }
        | PyObject::DictValuesIter { .. }
        | PyObject::DictItemsIter { .. }
        | PyObject::DictRevIter { .. }
        | PyObject::DequeIter { .. }
        | PyObject::DequeRevIter { .. }
            if name == "__next__" || name == "__iter__" =>
        {
            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.to_string(),
                func: if name == "__next__" {
                    builtin_next
                } else {
                    builtin_iter
                },
                self_obj: PyObjectRef::new(o.clone()),
            }))
        }
        // `__length_hint__` for the dict-view and deque iterators — real
        // CPython exposes this (used by `operator.length_hint`/`list()`
        // presizing) even though these iterators have no public `__len__`.
        // Found via test_iterlen.py: `length_hint()` on these raised
        // TypeError instead of returning the remaining-items count.
        PyObject::DictIter { .. }
        | PyObject::DictValuesIter { .. }
        | PyObject::DictItemsIter { .. }
        | PyObject::DictRevIter { .. }
        | PyObject::DequeIter { .. }
        | PyObject::DequeRevIter { .. }
            if name == "__length_hint__" =>
        {
            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__length_hint__".to_string(),
                func: |args| {
                    // Real CPython's dict/deque iterators report a
                    // `__length_hint__` of 0 once the underlying container
                    // has been mutated during iteration (`dictiter_len` in
                    // CPython's `dictobject.c` checks the live container's
                    // version/size against the value captured at iterator
                    // creation) — found via test_iterlen.py's
                    // `test_immutable_during_iteration`, which mutates then
                    // expects `length_hint()==0` even though the raw
                    // remaining-slots count would otherwise be nonzero.
                    let hint = match &*args[0].borrow() {
                        PyObject::DictIter {
                            dict,
                            keys,
                            index,
                            expected_version,
                        } => {
                            if dict_version_changed(dict, *expected_version) {
                                0
                            } else {
                                keys.len().saturating_sub(*index)
                            }
                        }
                        PyObject::DictValuesIter {
                            dict,
                            values,
                            index,
                            expected_version,
                        } => {
                            if dict_version_changed(dict, *expected_version) {
                                0
                            } else {
                                values.len().saturating_sub(*index)
                            }
                        }
                        PyObject::DictItemsIter {
                            dict,
                            items,
                            index,
                            expected_version,
                        } => {
                            if dict_version_changed(dict, *expected_version) {
                                0
                            } else {
                                items.len().saturating_sub(*index)
                            }
                        }
                        PyObject::DictRevIter {
                            dict,
                            index,
                            expected_version,
                            ..
                        } => {
                            if dict_version_changed(dict, *expected_version) {
                                0
                            } else {
                                (*index + 1).max(0) as usize
                            }
                        }
                        PyObject::DequeIter {
                            deque,
                            index,
                            start_len,
                        } => {
                            let cur_len = deque_len(deque).unwrap_or(*start_len);
                            if cur_len != *start_len {
                                0
                            } else {
                                start_len.saturating_sub(*index)
                            }
                        }
                        PyObject::DequeRevIter {
                            deque,
                            index,
                            start_len,
                        } => {
                            let cur_len = deque_len(deque).unwrap_or(*start_len);
                            if cur_len != *start_len {
                                0
                            } else {
                                (*index + 1).max(0) as usize
                            }
                        }
                        _ => 0,
                    };
                    Ok(py_int(hint as i64))
                },
                self_obj: PyObjectRef::new(o.clone()),
            }))
        }
        _ => Err(PyError::attribute_error(format!(
            "'{}' object has no attribute '{}'",
            o.type_name(),
            name
        ))),
    }
}
