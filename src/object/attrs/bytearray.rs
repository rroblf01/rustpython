// Auto-extracted from src/object/attrs/mod.rs lines 1797-2452
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::ByteArray(_b) => {
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
                    "__mod__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mod__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error("__mod__() too few args"));
                            }
                            let fmt = match &*args[1].borrow() {
                                PyObject::ByteArray(b) => b.clone(),
                                _ => {
                                    return Err(PyError::runtime_error("__mod__ on non-bytearray"))
                                }
                            };
                            let result = bytes_interpolate(&fmt, &args[2]).map_err(|e| {
                                if e.contains("too big") || e.contains("[overflow]") {
                                    PyError::overflow_error(
                                        e.trim_end_matches(" [overflow]").to_string(),
                                    )
                                } else if e.contains("unsupported format character")
                                    || e == "incomplete format"
                                {
                                    PyError::value_error(e)
                                } else {
                                    PyError::type_error(e)
                                }
                            })?;
                            Ok(PyObjectRef::new(PyObject::ByteArray(result)))
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
                            let val = args[1].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| {
                                    PyError::value_error("byte value out of range")
                                })?;
                                if n < 0 || n > 255 {
                                    return Err(PyError::value_error(
                                        "byte must be in range(0, 256)",
                                    ));
                                }
                                if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    bytes.push(n as u8);
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("append on non-bytearray"))
                                }
                            } else {
                                Err(PyError::type_error("argument must be an integer"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "extend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "extend".to_string(),
                        func: |args| {
                                                        if crate::object::is_bytearray_exported(&args[0]) {
                                return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized"));
                            }
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "extend() takes exactly one argument",
                                ));
                            }
                            let it = builtin_iter(&[args[1].clone()])?;
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => {
                                        let vv = v.borrow();
                                        if let PyObject::Int(i) = &*vv {
                                            let n = i.to_i64().ok_or_else(|| {
                                                PyError::value_error("byte value out of range")
                                            })?;
                                            if n < 0 || n > 255 {
                                                return Err(PyError::value_error(
                                                    "byte must be in range(0, 256)",
                                                ));
                                            }
                                            if let PyObject::ByteArray(bytes) =
                                                &mut *args[0].borrow_mut()
                                            {
                                                bytes.push(n as u8);
                                            } else {
                                                return Err(PyError::runtime_error(
                                                    "extend on non-bytearray",
                                                ));
                                            }
                                        } else {
                                            return Err(PyError::type_error(
                                                "argument must be iterable of integers",
                                            ));
                                        }
                                    }
                                    Err(PyError::StopIteration) => return Ok(py_none()),
                                    Err(e) => return Err(e),
                                }
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "insert" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "insert".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "insert() takes exactly 2 arguments",
                                ));
                            }
                            let idx = args[1].as_i64().unwrap_or(0) as usize;
                            let val = args[2].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| {
                                    PyError::value_error("byte value out of range")
                                })?;
                                if n < 0 || n > 255 {
                                    return Err(PyError::value_error(
                                        "byte must be in range(0, 256)",
                                    ));
                                }
                                if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    let idx = idx.min(bytes.len());
                                    bytes.insert(idx, n as u8);
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("insert on non-bytearray"))
                                }
                            } else {
                                Err(PyError::type_error("argument must be an integer"))
                            }
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
                            let val = args[1].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| {
                                    PyError::value_error("byte value out of range")
                                })? as u8;
                                if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    let pos =
                                        bytes.iter().position(|&x| x == n).ok_or_else(|| {
                                            PyError::value_error(format!(
                                                "value {} not found in bytearray",
                                                n
                                            ))
                                        })?;
                                    bytes.remove(pos);
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("remove on non-bytearray"))
                                }
                            } else {
                                Err(PyError::type_error("argument must be an integer"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                let idx = if args.len() > 1 {
                                    let i = args[1].as_i64().ok_or_else(|| {
                                        PyError::type_error("pop index must be an integer")
                                    })?;
                                    let len = bytes.len() as i64;
                                    if i < 0 {
                                        len + i
                                    } else {
                                        i
                                    }
                                } else {
                                    bytes.len() as i64 - 1
                                };
                                if idx < 0 || idx >= bytes.len() as i64 {
                                    return Err(PyError::index_error("pop index out of range"));
                                }
                                let val = bytes.remove(idx as usize);
                                Ok(PyObjectRef::imm(PyObject::Int(BigInt::from(val))))
                            } else {
                                Err(PyError::runtime_error("pop on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__getitem__() requires an index"));
                            }
                            py_getitem(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__setitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setitem__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "__setitem__() requires an index and value",
                                ));
                            }
                            py_setitem(&args[0], &args[1], args[2].clone())?;
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__len__".to_string(),
                        func: |args| {
                            let b = args[0].borrow();
                            if let PyObject::ByteArray(bytes) = &*b {
                                Ok(py_int(bytes.len() as i64))
                            } else {
                                Err(PyError::runtime_error("__len__ on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__str__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__str__".to_string(),
                        func: |args| {
                            if let PyObject::ByteArray(bytes) = &*args[0].borrow() {
                                let hex: String =
                                    bytes.iter().map(|b| format!("{:02x}", b)).collect();
                                Ok(py_str(&hex))
                            } else {
                                Err(PyError::runtime_error("__str__ on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::ByteArray(bytes) = &*args[0].borrow() {
                                Ok(py_int(33 + bytes.len() as i64))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removeprefix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removeprefix".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "removeprefix() takes exactly 1 argument",
                                ));
                            }
                            if let PyObject::ByteArray(b) = &*args[0].borrow() {
                                let prefix = args[1].borrow();
                                if let PyObject::Bytes(p) = &*prefix {
                                    if b.starts_with(p.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[p.len()..].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else if let PyObject::ByteArray(p) = &*prefix {
                                    if b.starts_with(p.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[p.len()..].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else {
                                    Err(PyError::type_error(
                                        "removeprefix() argument must be bytes-like",
                                    ))
                                }
                            } else {
                                Err(PyError::runtime_error("removeprefix on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removesuffix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removesuffix".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "removesuffix() takes exactly 1 argument",
                                ));
                            }
                            if let PyObject::ByteArray(b) = &*args[0].borrow() {
                                let suffix = args[1].borrow();
                                if let PyObject::Bytes(s) = &*suffix {
                                    if b.ends_with(s.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[..b.len() - s.len()].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else if let PyObject::ByteArray(s) = &*suffix {
                                    if b.ends_with(s.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[..b.len() - s.len()].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else {
                                    Err(PyError::type_error(
                                        "removesuffix() argument must be bytes-like",
                                    ))
                                }
                            } else {
                                Err(PyError::runtime_error("removesuffix on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Delegate to `bytes`'s implementation — see
                    // `bytearray_delegate`'s doc comment above.
                    "startswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "startswith".to_string(),
                        func: |args| bytearray_delegate("startswith", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "endswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "endswith".to_string(),
                        func: |args| bytearray_delegate("endswith", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "find" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "find".to_string(),
                        func: |args| bytearray_delegate("find", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rfind" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rfind".to_string(),
                        func: |args| bytearray_delegate("rfind", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "index".to_string(),
                        func: |args| bytearray_delegate("index", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rindex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rindex".to_string(),
                        func: |args| bytearray_delegate("rindex", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| bytearray_delegate("count", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "replace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "replace".to_string(),
                        func: |args| bytearray_delegate("replace", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "split" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "split".to_string(),
                        func: |args| bytearray_delegate("split", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rsplit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rsplit".to_string(),
                        func: |args| bytearray_delegate("rsplit", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "strip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "strip".to_string(),
                        func: |args| bytearray_delegate("strip", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lstrip".to_string(),
                        func: |args| bytearray_delegate("lstrip", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rstrip".to_string(),
                        func: |args| bytearray_delegate("rstrip", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| bytearray_delegate("join", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "upper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "upper".to_string(),
                        func: |args| bytearray_delegate("upper", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lower".to_string(),
                        func: |args| bytearray_delegate("lower", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "title" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "title".to_string(),
                        func: |args| bytearray_delegate("title", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "capitalize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "capitalize".to_string(),
                        func: |args| bytearray_delegate("capitalize", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "swapcase" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "swapcase".to_string(),
                        func: |args| bytearray_delegate("swapcase", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalpha" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalpha".to_string(),
                        func: |args| bytearray_delegate("isalpha", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdigit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdigit".to_string(),
                        func: |args| bytearray_delegate("isdigit", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalnum" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalnum".to_string(),
                        func: |args| bytearray_delegate("isalnum", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isspace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isspace".to_string(),
                        func: |args| bytearray_delegate("isspace", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isupper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isupper".to_string(),
                        func: |args| bytearray_delegate("isupper", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "islower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "islower".to_string(),
                        func: |args| bytearray_delegate("islower", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "istitle" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "istitle".to_string(),
                        func: |args| bytearray_delegate("istitle", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "partition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "partition".to_string(),
                        func: |args| bytearray_delegate("partition", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rpartition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rpartition".to_string(),
                        func: |args| bytearray_delegate("rpartition", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "splitlines" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "splitlines".to_string(),
                        func: |args| bytearray_delegate("splitlines", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "expandtabs" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "expandtabs".to_string(),
                        func: |args| bytearray_delegate("expandtabs", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "zfill" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "zfill".to_string(),
                        func: |args| bytearray_delegate("zfill", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "ljust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "ljust".to_string(),
                        func: |args| bytearray_delegate("ljust", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rjust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rjust".to_string(),
                        func: |args| bytearray_delegate("rjust", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "center" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "center".to_string(),
                        func: |args| bytearray_delegate("center", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "translate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "translate".to_string(),
                        func: |args| bytearray_delegate("translate", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "decode" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "decode".to_string(),
                        func: |args| bytearray_delegate("decode", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "hex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "hex".to_string(),
                        func: |args| bytearray_delegate("hex", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => {
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "clear".to_string(),
                            func: |args| {
                                if crate::object::is_bytearray_exported(&args[0]) {
                                    return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized"));
                                }
                                if let PyObject::ByteArray(b) = &mut *args[0].borrow_mut() {
                                    b.clear();
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("clear on non-bytearray"))
                                }
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    "__buffer__" => {
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "__buffer__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__buffer__() takes exactly one argument"));
                                }
                                let flags = crate::object::extract_flags_for_buffer(&args[1])?;
                                crate::object::check_buffer_flags(flags)?;
                                let len = if let PyObject::ByteArray(b) = &*args[0].borrow() { b.len() } else {
                                    if let Some(backing) = crate::object::native_backing_of(&args[0]) {
                                        if let PyObject::ByteArray(b) = &*backing.borrow() { b.len() } else { 0 }
                                    } else { 0 }
                                };
                                let view = PyObjectRef::new(PyObject::MemoryView { source: args[0].clone(), format: "B".to_string(), shape: vec![len], itemsize: 1, offset: 0, readonly: false, released: false });
                                crate::object::track_view_exporter(&view, args[0].clone());
                                crate::object::increment_bytearray_export(&args[0]);
                                Ok(view)
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    "__release_buffer__" => {
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "__release_buffer__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__release_buffer__() takes exactly one argument"));
                                }
                                let view = &args[1];
                                if let PyObject::MemoryView { released, .. } = &*view.borrow() {
                                    if *released {
                                        return Err(PyError::value_error("buffer already released"));
                                    }
                                }
                                // Check if view's source matches self's backing - if not, raise ValueError (mismatched buffer)
                                {
                                    let view_source = {
                                        let b = view.borrow();
                                        if let PyObject::MemoryView { source, .. } = &*b {
                                            Some(source.clone())
                                        } else { None }
                                    };
                                    if let Some(vs) = view_source {
                                        let self_backing = crate::object::native_backing_of(&args[0]).unwrap_or_else(|| args[0].clone());
                                        let is_same = vs.is(&self_backing) || vs.is(&args[0]);
                                        // Also check via bytes equality? For bytes vs bytearray, they are different objects, so not same
                                        if !is_same {
                                            // For bytearray subclass that returned a different buffer (e.g. bytes), mismatch should raise ValueError
                                            // Check if view's source is bytearray/bytes of same length but different object -> raise
                                            // We check if view's source type is Bytes vs ByteArray mismatch
                                            let vs_is_bytes = matches!(&*vs.borrow(), PyObject::Bytes(_));
                                            let self_is_bytearray = matches!(&*self_backing.borrow(), PyObject::ByteArray(_));
                                            if vs_is_bytes || self_is_bytearray {
                                                // If sources are not same object, raise
                                                return Err(PyError::value_error("buffer mismatch: view not from this object"));
                                            }
                                        }
                                    }
                                }
                                {
                                    let mut b = view.borrow_mut();
                                    if let PyObject::MemoryView { released, .. } = &mut *b {
                                        *released = true;
                                    }
                                }
                                Ok(py_none())
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'bytearray' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
