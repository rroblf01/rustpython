// Auto-extracted from src/object/attrs/mod.rs lines 5304-5495
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Array(arr) => {
                let typecode = arr.typecode;
                let is_float = array_typecode_is_float(typecode);
                match name {
                    "itemsize" => Ok(py_int(mv_itemsize(&typecode.to_string()) as i64)),
                    "typecode" => Ok(py_str(&typecode.to_string())),
                    "__len__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__len__".to_string(),
                        func: |args| {
                            if let PyObject::Array(arr) = &*args[0].borrow() {
                                Ok(py_int(arr.data.len() as i64))
                            } else {
                                Err(PyError::runtime_error("__len__ on non-array"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: |args| {
                            if let PyObject::Array(arr) = &*args[0].borrow() {
                                let items: Vec<PyObjectRef> = arr
                                    .data
                                    .iter()
                                    .map(|v| {
                                        if array_typecode_is_float(arr.typecode) {
                                            py_float(*v)
                                        } else {
                                            py_int(*v as i64)
                                        }
                                    })
                                    .collect();
                                Ok(PyObjectRef::new(PyObject::ListIter {
                                    list: items,
                                    index: 0,
                                }))
                            } else {
                                Err(PyError::runtime_error("__iter__ on non-array"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if let PyObject::Array(arr) = &*args[0].borrow() {
                                let idx =
                                    args.get(1).and_then(|a| a.as_i64()).ok_or_else(|| {
                                        PyError::type_error("array indices must be integers")
                                    })?;
                                let len = arr.data.len() as i64;
                                let i = if idx < 0 { len + idx } else { idx };
                                if i < 0 || i >= len {
                                    return Err(PyError::index_error("array index out of range"));
                                }
                                let v = arr.data[i as usize];
                                Ok(if array_typecode_is_float(arr.typecode) {
                                    py_float(v)
                                } else {
                                    py_int(v as i64)
                                })
                            } else {
                                Err(PyError::runtime_error("__getitem__ on non-array"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "tobytes" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "tobytes".to_string(),
                        func: |args| {
                            if let PyObject::Array(arr) = &*args[0].borrow() {
                                let is_float = array_typecode_is_float(arr.typecode);
                                let isz = mv_itemsize(&arr.typecode.to_string());
                                let mut out = Vec::with_capacity(arr.data.len() * isz);
                                for &v in &arr.data {
                                    if is_float {
                                        if isz == 4 {
                                            out.extend_from_slice(&(v as f32).to_ne_bytes());
                                        } else {
                                            out.extend_from_slice(&v.to_ne_bytes());
                                        }
                                    } else {
                                        let n = v as i64;
                                        match isz {
                                            1 => out.push(n as u8),
                                            2 => out.extend_from_slice(&(n as i16).to_ne_bytes()),
                                            4 => out.extend_from_slice(&(n as i32).to_ne_bytes()),
                                            _ => out.extend_from_slice(&n.to_ne_bytes()),
                                        }
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::Bytes(out)))
                            } else {
                                Err(PyError::runtime_error("tobytes on non-array"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "tolist" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "tolist".to_string(),
                        func: |args| {
                            if let PyObject::Array(arr) = &*args[0].borrow() {
                                let is_float = array_typecode_is_float(arr.typecode);
                                let items: Vec<PyObjectRef> = arr
                                    .data
                                    .iter()
                                    .map(|&v| {
                                        if is_float {
                                            py_float(v)
                                        } else {
                                            py_int(v as i64)
                                        }
                                    })
                                    .collect();
                                Ok(py_list(items))
                            } else {
                                Err(PyError::runtime_error("tolist on non-array"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "append" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "append".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "append() takes exactly one argument",
                                ));
                            }
                            let v = if array_typecode_is_float(match &*args[0].borrow() {
                                PyObject::Array(a) => a.typecode,
                                _ => 'B',
                            }) {
                                args[1].as_f64().unwrap_or(0.0)
                            } else {
                                args[1].as_i64().unwrap_or(0) as f64
                            };
                            if let PyObject::Array(arr) = &mut *args[0].borrow_mut() {
                                arr.data.push(v);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "extend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "extend".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "extend() takes exactly one argument",
                                ));
                            }
                            let is_float = match &*args[0].borrow() {
                                PyObject::Array(a) => array_typecode_is_float(a.typecode),
                                _ => false,
                            };
                            let items = collect_iterable(&args[1])?;
                            let mut vals = Vec::with_capacity(items.len());
                            for it in &items {
                                vals.push(if is_float {
                                    it.as_f64().unwrap_or(0.0)
                                } else {
                                    it.as_i64().unwrap_or(0) as f64
                                });
                            }
                            if let PyObject::Array(arr) = &mut *args[0].borrow_mut() {
                                arr.data.extend(vals);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "buffer_info" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "buffer_info".to_string(),
                        func: |args| {
                            if let PyObject::Array(arr) = &*args[0].borrow() {
                                Ok(py_tuple(vec![py_int(0), py_int(arr.data.len() as i64)]))
                            } else {
                                Err(PyError::runtime_error("buffer_info on non-array"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => {
                        let _ = is_float;
                        Err(PyError::attribute_error(format!(
                            "'array.array' object has no attribute '{}'",
                            name
                        )))
                    }
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
