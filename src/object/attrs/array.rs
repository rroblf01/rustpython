// Auto-extracted from src/object/attrs/mod.rs lines 5304-5495
use super::*;
use crate::interner;
use crate::object::*;

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
                            let idx = args.get(1).and_then(|a| a.as_i64()).ok_or_else(|| {
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
                        if args.len() != 1 {
                            return Err(PyError::type_error(format!(
                                "tolist() takes no arguments ({} given)",
                                args.len() - 1
                            )));
                        }
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
                            return Err(PyError::type_error("append() takes exactly one argument"));
                        }
                        let is_float = array_typecode_is_float(match &*args[0].borrow() {
                            PyObject::Array(a) => a.typecode,
                            _ => 'B',
                        });
                        let v = if is_float {
                            args[1].as_f64().ok_or_else(|| {
                                PyError::type_error("must be real number, not str")
                            })?
                        } else {
                            if args[1].as_i64().is_none() {
                                return Err(PyError::type_error(
                                    "integer argument expected, got float",
                                ));
                            }
                            args[1].as_i64().unwrap() as f64
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
                            return Err(PyError::type_error("extend() takes exactly one argument"));
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
                "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "count".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("count() takes exactly one argument"));
                        }
                        if let PyObject::Array(arr) = &*args[0].borrow() {
                            let is_float = array_typecode_is_float(arr.typecode);
                            let target = if is_float {
                                args[1].as_f64().unwrap_or(0.0)
                            } else {
                                args[1].as_i64().unwrap_or(0) as f64
                            };
                            let c = arr
                                .data
                                .iter()
                                .filter(|v| {
                                    if is_float {
                                        (**v - target).abs() < 1e-9
                                    } else {
                                        **v == target
                                    }
                                })
                                .count();
                            Ok(py_int(c as i64))
                        } else {
                            Err(PyError::runtime_error("count on non-array"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "clear".to_string(),
                    func: |args| {
                        if args.len() != 1 {
                            return Err(PyError::type_error(format!(
                                "clear() takes no arguments ({} given)",
                                args.len() - 1
                            )));
                        }
                        if let PyObject::Array(arr) = &mut *args[0].borrow_mut() {
                            arr.data.clear();
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "byteswap" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "byteswap".to_string(),
                    func: |args| {
                        if let PyObject::Array(arr) = &mut *args[0].borrow_mut() {
                            let isz = mv_itemsize(&arr.typecode.to_string());
                            if isz > 1 {
                                for v in &mut arr.data {
                                    let n = *v as i64;
                                    let swapped = match isz {
                                        2 => ((n as i16).swap_bytes() as i64) as f64,
                                        4 => ((n as i32).swap_bytes() as i64) as f64,
                                        8 => (n.swap_bytes() as f64),
                                        _ => n as f64,
                                    };
                                    *v = swapped;
                                }
                            }
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__setitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__setitem__".to_string(),
                    func: |args| {
                        if args.len() < 3 {
                            return Err(PyError::type_error("__setitem__ takes 2 arguments"));
                        }
                        let idx = args[1].as_i64().unwrap_or(0);
                        let is_float = array_typecode_is_float(match &*args[0].borrow() {
                            PyObject::Array(a) => a.typecode,
                            _ => 'B',
                        });
                        let v = if is_float {
                            args[2].as_f64().unwrap_or(0.0)
                        } else {
                            args[2].as_i64().unwrap_or(0) as f64
                        };
                        if let PyObject::Array(arr) = &mut *args[0].borrow_mut() {
                            let len = arr.data.len() as i64;
                            let i = if idx < 0 { len + idx } else { idx };
                            if i < 0 || i >= len {
                                return Err(PyError::index_error(
                                    "array assignment index out of range",
                                ));
                            }
                            arr.data[i as usize] = v;
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__delitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__delitem__".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("__delitem__ takes 1 argument"));
                        }
                        let idx = args[1].as_i64().unwrap_or(0);
                        if let PyObject::Array(arr) = &mut *args[0].borrow_mut() {
                            let len = arr.data.len() as i64;
                            let i = if idx < 0 { len + idx } else { idx };
                            if i < 0 || i >= len {
                                return Err(PyError::index_error(
                                    "array assignment index out of range",
                                ));
                            }
                            arr.data.remove(i as usize);
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "frombytes" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "frombytes".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "frombytes() takes exactly one argument",
                            ));
                        }
                        let (typecode, isz, is_float) =
                            if let PyObject::Array(arr) = &*args[0].borrow() {
                                (
                                    arr.typecode,
                                    mv_itemsize(&arr.typecode.to_string()),
                                    array_typecode_is_float(arr.typecode),
                                )
                            } else {
                                return Err(PyError::runtime_error("frombytes on non-array"));
                            };
                        let bytes: Vec<u8> = match &*args[1].borrow() {
                            PyObject::Bytes(b) => b.clone(),
                            PyObject::ByteArray(b) => b.clone(),
                            _ => {
                                return Err(PyError::type_error(
                                    "a bytes-like object is required, not this type",
                                ))
                            }
                        };
                        if bytes.len() % isz != 0 {
                            return Err(PyError::value_error(
                                "bytes length not a multiple of item size",
                            ));
                        }
                        let mut vals = Vec::with_capacity(bytes.len() / isz);
                        for chunk in bytes.chunks(isz) {
                            let v = if is_float {
                                match isz {
                                    4 => f32::from_ne_bytes(chunk.try_into().unwrap()) as f64,
                                    _ => f64::from_ne_bytes(chunk.try_into().unwrap()),
                                }
                            } else {
                                match isz {
                                    1 => {
                                        if typecode == 'b' {
                                            chunk[0] as i8 as f64
                                        } else {
                                            chunk[0] as f64
                                        }
                                    }
                                    2 => {
                                        if typecode.is_uppercase() {
                                            u16::from_ne_bytes(chunk.try_into().unwrap()) as f64
                                        } else {
                                            i16::from_ne_bytes(chunk.try_into().unwrap()) as f64
                                        }
                                    }
                                    4 => {
                                        if typecode.is_uppercase() || typecode == 'L' {
                                            u32::from_ne_bytes(chunk.try_into().unwrap()) as f64
                                        } else {
                                            i32::from_ne_bytes(chunk.try_into().unwrap()) as f64
                                        }
                                    }
                                    _ => {
                                        if typecode.is_uppercase() {
                                            u64::from_ne_bytes(chunk.try_into().unwrap()) as f64
                                        } else {
                                            i64::from_ne_bytes(chunk.try_into().unwrap()) as f64
                                        }
                                    }
                                }
                            };
                            vals.push(v);
                        }
                        if let PyObject::Array(arr) = &mut *args[0].borrow_mut() {
                            arr.data.extend(vals);
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "fromfile" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "fromfile".to_string(),
                    func: |args| {
                        if args.len() < 3 {
                            return Err(PyError::type_error(
                                "fromfile() takes exactly 2 arguments",
                            ));
                        }
                        let isz = if let PyObject::Array(arr) = &*args[0].borrow() {
                            mv_itemsize(&arr.typecode.to_string())
                        } else {
                            return Err(PyError::runtime_error("fromfile on non-array"));
                        };
                        let n = args[2].as_i64().unwrap_or(0).max(0) as usize;
                        let nbytes = n * isz;
                        let read_method = args[1].borrow().get_attribute("read")?;
                        let data = {
                            let borrowed = read_method.borrow();
                            match &*borrowed {
                                PyObject::BuiltinMethod { func, .. }
                                | PyObject::BuiltinFunction { func, .. } => {
                                    let f = *func;
                                    let so = args[1].clone();
                                    drop(borrowed);
                                    f(&[so, py_int(nbytes as i64)])?
                                }
                                PyObject::Closure(cl) => {
                                    let cl = cl.clone();
                                    let so = args[1].clone();
                                    drop(borrowed);
                                    cl(&[so, py_int(nbytes as i64)])?
                                }
                                _ => {
                                    return Err(PyError::type_error(
                                        "fromfile() argument must have a read() method",
                                    ));
                                }
                            }
                        };
                        let bytes: Vec<u8> = match &*data.borrow() {
                            PyObject::Bytes(b) => b.clone(),
                            PyObject::ByteArray(b) => b.clone(),
                            PyObject::Str(s) => s.as_bytes().to_vec(),
                            _ => return Err(PyError::type_error("read() didn't return bytes")),
                        };
                        // Real `array.fromfile` still appends whatever
                        // complete items it managed to read before
                        // raising `EOFError` on a short read (CPython:
                        // "the items that were read are inserted into
                        // the array") — trim to a whole number of items
                        // first so a trailing partial item isn't fed to
                        // `frombytes` (which requires an exact multiple).
                        let short_read = bytes.len() < nbytes;
                        let usable_len = (bytes.len() / isz) * isz;
                        let usable_bytes = bytes[..usable_len].to_vec();
                        let frombytes = args[0].borrow().get_attribute("frombytes")?;
                        let borrowed = frombytes.borrow();
                        let append_result = if let PyObject::BuiltinMethod { func, .. } = &*borrowed
                        {
                            let f = *func;
                            let so = args[0].clone();
                            drop(borrowed);
                            f(&[so, PyObjectRef::imm(PyObject::Bytes(usable_bytes))])
                        } else {
                            Err(PyError::runtime_error("frombytes lookup failed"))
                        };
                        append_result?;
                        if short_read {
                            return Err(PyError::Exception(
                                "EOFError".to_string(),
                                PyObjectRef::new(PyObject::Exception {
                                    typ: "EOFError".to_string(),
                                    args: vec![py_str("read() didn't return enough bytes")],
                                    cause: None,
                                    suppress_context: false,
                                    context: None,
                                    traceback: None,
                                    extra: None,
                                }),
                            ));
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "tofile" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "tofile".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("tofile() takes exactly one argument"));
                        }
                        let is_float = if let PyObject::Array(arr) = &*args[0].borrow() {
                            array_typecode_is_float(arr.typecode)
                        } else {
                            return Err(PyError::runtime_error("tofile on non-array"));
                        };
                        let tobytes = args[0].borrow().get_attribute("tobytes")?;
                        let bytes_obj = {
                            let borrowed = tobytes.borrow();
                            match &*borrowed {
                                PyObject::BuiltinMethod { func, .. }
                                | PyObject::BuiltinFunction { func, .. } => {
                                    let f = *func;
                                    let so = args[0].clone();
                                    drop(borrowed);
                                    f(&[so])?
                                }
                                PyObject::Closure(cl) => {
                                    let cl = cl.clone();
                                    let so = args[0].clone();
                                    drop(borrowed);
                                    cl(&[so])?
                                }
                                _ => return Err(PyError::runtime_error("tobytes lookup failed")),
                            }
                        };
                        let _ = is_float;
                        let write_method = args[1].borrow().get_attribute("write")?;
                        let borrowed = write_method.borrow();
                        match &*borrowed {
                            PyObject::BuiltinMethod { func, .. }
                            | PyObject::BuiltinFunction { func, .. } => {
                                let f = *func;
                                let so = args[1].clone();
                                drop(borrowed);
                                f(&[so, bytes_obj])?;
                            }
                            PyObject::Closure(cl) => {
                                let cl = cl.clone();
                                let so = args[1].clone();
                                drop(borrowed);
                                cl(&[so, bytes_obj])?;
                            }
                            _ => {
                                return Err(PyError::type_error(
                                    "tofile() argument must have a write() method",
                                ));
                            }
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "index".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("index() takes at least 1 argument"));
                        }
                        if let PyObject::Array(arr) = &*args[0].borrow() {
                            let is_float = array_typecode_is_float(arr.typecode);
                            let target = if is_float {
                                args[1].as_f64().unwrap_or(0.0)
                            } else {
                                args[1].as_i64().unwrap_or(0) as f64
                            };
                            for (i, v) in arr.data.iter().enumerate() {
                                let eq = if is_float {
                                    (*v - target).abs() < 1e-9
                                } else {
                                    *v == target
                                };
                                if eq {
                                    return Ok(py_int(i as i64));
                                }
                            }
                            Err(PyError::value_error("array.index(x): x not in array"))
                        } else {
                            Err(PyError::runtime_error("index on non-array"))
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
        _ => unreachable!(
            "{} called with non-{} object",
            stringify!(get),
            o.type_name()
        ),
    }
}
