// Extracted from src/object/attrs/mod.rs — iterator attribute dispatch
// Covers `list_iterator` and the generic `__next__`/`__iter__` backed iterators
// (`map`/`filter`/`zip`/`cycle`/`groupby`/`enumerate`/`GetItemIter`/`CallSentinelIter`).
// Keeps mod.rs under 1k lines; this is exactly ~70 lines extracted (1069-70=999).
use crate::object::*;
use super::*;

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
                        Err(PyError::runtime_error("__length_hint__ on non-list_iterator"))
                    }
                },
                self_obj: PyObjectRef::new(o.clone()),
            })),
            "__setstate__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__setstate__".to_string(),
                func: crate::object::builtins::list_iter_setstate,
                self_obj: PyObjectRef::new(o.clone()),
            })),
            "__reduce__" | "__reduce_ex__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.to_string(),
                func: |args| {
                    if let PyObject::ListIter { list, index } = &*args[0].borrow() {
                        let iter_obj = args[0].clone();
                        let state = py_int(*index as i64);
                        Ok(py_tuple(vec![py_str("list_iterator"), py_tuple(vec![iter_obj, state])]))
                    } else {
                        Err(PyError::runtime_error("__reduce__ on non-list_iterator"))
                    }
                },
                self_obj: PyObjectRef::new(o.clone()),
            })),
            _ => Err(PyError::attribute_error(format!(
                "'list_iterator' object has no attribute '{}'",
                name
            ))),
        },
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
        _ => Err(PyError::attribute_error(format!(
            "'{}' object has no attribute '{}'",
            o.type_name(),
            name
        ))),
    }
}
