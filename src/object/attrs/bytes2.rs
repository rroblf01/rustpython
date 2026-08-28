// Auto-extracted from src/object/attrs/mod.rs lines 2821-3519
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Bytes(_v) => {
                match name {
                    "strip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "strip".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let chars = if args.len() > 1
                                    && !matches!(&*args[1].borrow(), PyObject::None)
                                {
                                    arg_bytes(&args[1])
                                } else {
                                    None
                                };
                                let is_strip = |c: &u8| match &chars {
                                    Some(cs) => cs.contains(c),
                                    None => c.is_ascii_whitespace(),
                                };
                                let start = b.iter().position(|c| !is_strip(c)).unwrap_or(b.len());
                                let end = b
                                    .iter()
                                    .rposition(|c| !is_strip(c))
                                    .map(|i| i + 1)
                                    .unwrap_or(start);
                                Ok(PyObjectRef::imm(PyObject::Bytes(
                                    b[start..end.max(start)].to_vec(),
                                )))
                            } else {
                                Err(PyError::runtime_error("strip on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lstrip".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let chars = if args.len() > 1
                                    && !matches!(&*args[1].borrow(), PyObject::None)
                                {
                                    arg_bytes(&args[1])
                                } else {
                                    None
                                };
                                let is_strip = |c: &u8| match &chars {
                                    Some(cs) => cs.contains(c),
                                    None => c.is_ascii_whitespace(),
                                };
                                let start = b.iter().position(|c| !is_strip(c)).unwrap_or(b.len());
                                Ok(PyObjectRef::imm(PyObject::Bytes(b[start..].to_vec())))
                            } else {
                                Err(PyError::runtime_error("lstrip on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rstrip".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let chars = if args.len() > 1
                                    && !matches!(&*args[1].borrow(), PyObject::None)
                                {
                                    arg_bytes(&args[1])
                                } else {
                                    None
                                };
                                let is_strip = |c: &u8| match &chars {
                                    Some(cs) => cs.contains(c),
                                    None => c.is_ascii_whitespace(),
                                };
                                let end = b
                                    .iter()
                                    .rposition(|c| !is_strip(c))
                                    .map(|i| i + 1)
                                    .unwrap_or(0);
                                Ok(PyObjectRef::imm(PyObject::Bytes(b[..end].to_vec())))
                            } else {
                                Err(PyError::runtime_error("rstrip on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "join() takes exactly one argument",
                                ));
                            }
                            let sep = if let PyObject::Bytes(b) = &*args[0].borrow() {
                                b.clone()
                            } else {
                                return Err(PyError::runtime_error("join on non-bytes"));
                            };
                            let iterator = crate::object::builtin_iter(&[args[1].clone()])?;
                            let mut parts: Vec<Vec<u8>> = Vec::new();
                            loop {
                                match crate::object::builtin_next(&[iterator.clone()]) {
                                    Ok(v) => parts.push(arg_bytes(&v).ok_or_else(|| {
                                        PyError::type_error(
                                            "sequence item: expected a bytes-like object",
                                        )
                                    })?),
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            Ok(PyObjectRef::imm(PyObject::Bytes(
                                parts.join(sep.as_slice()),
                            )))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "upper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "upper".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(PyObjectRef::imm(PyObject::Bytes(
                                    b.iter().map(|c| c.to_ascii_uppercase()).collect(),
                                )))
                            } else {
                                Err(PyError::runtime_error("upper on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lower".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(PyObjectRef::imm(PyObject::Bytes(
                                    b.iter().map(|c| c.to_ascii_lowercase()).collect(),
                                )))
                            } else {
                                Err(PyError::runtime_error("lower on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "swapcase" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "swapcase".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(PyObjectRef::imm(PyObject::Bytes(
                                    b.iter()
                                        .map(|c| {
                                            if c.is_ascii_uppercase() {
                                                c.to_ascii_lowercase()
                                            } else if c.is_ascii_lowercase() {
                                                c.to_ascii_uppercase()
                                            } else {
                                                *c
                                            }
                                        })
                                        .collect(),
                                )))
                            } else {
                                Err(PyError::runtime_error("swapcase on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "capitalize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "capitalize".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let mut result: Vec<u8> =
                                    b.iter().map(|c| c.to_ascii_lowercase()).collect();
                                if let Some(first) = result.first_mut() {
                                    *first = first.to_ascii_uppercase();
                                }
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("capitalize on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "title" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "title".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let mut result = Vec::with_capacity(b.len());
                                let mut prev_cased = false;
                                for &c in b.iter() {
                                    if c.is_ascii_alphabetic() {
                                        result.push(if prev_cased {
                                            c.to_ascii_lowercase()
                                        } else {
                                            c.to_ascii_uppercase()
                                        });
                                        prev_cased = true;
                                    } else {
                                        result.push(c);
                                        prev_cased = false;
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("title on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalpha" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalpha".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(py_bool(
                                    !b.is_empty() && b.iter().all(|c| c.is_ascii_alphabetic()),
                                ))
                            } else {
                                Err(PyError::runtime_error("isalpha on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdigit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdigit".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(py_bool(
                                    !b.is_empty() && b.iter().all(|c| c.is_ascii_digit()),
                                ))
                            } else {
                                Err(PyError::runtime_error("isdigit on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalnum" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalnum".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(py_bool(
                                    !b.is_empty() && b.iter().all(|c| c.is_ascii_alphanumeric()),
                                ))
                            } else {
                                Err(PyError::runtime_error("isalnum on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isspace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isspace".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(py_bool(
                                    !b.is_empty() && b.iter().all(|c| c.is_ascii_whitespace()),
                                ))
                            } else {
                                Err(PyError::runtime_error("isspace on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isupper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isupper".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(py_bool(
                                    b.iter().any(|c| c.is_ascii_alphabetic())
                                        && b.iter().all(|c| !c.is_ascii_lowercase()),
                                ))
                            } else {
                                Err(PyError::runtime_error("isupper on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "islower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "islower".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                Ok(py_bool(
                                    b.iter().any(|c| c.is_ascii_alphabetic())
                                        && b.iter().all(|c| !c.is_ascii_uppercase()),
                                ))
                            } else {
                                Err(PyError::runtime_error("islower on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "istitle" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "istitle".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let mut prev_cased = false;
                                let mut is_title = true;
                                let mut saw_alpha = false;
                                for &c in b.iter() {
                                    if c.is_ascii_uppercase() {
                                        saw_alpha = true;
                                        if prev_cased {
                                            is_title = false;
                                            break;
                                        }
                                        prev_cased = true;
                                    } else if c.is_ascii_lowercase() {
                                        saw_alpha = true;
                                        if !prev_cased {
                                            is_title = false;
                                            break;
                                        }
                                        prev_cased = true;
                                    } else {
                                        prev_cased = false;
                                    }
                                }
                                Ok(py_bool(is_title && saw_alpha))
                            } else {
                                Err(PyError::runtime_error("istitle on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "partition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "partition".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "partition() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let sep = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                if sep.is_empty() {
                                    return Err(PyError::value_error("empty separator"));
                                }
                                match b.windows(sep.len()).position(|w| w == sep.as_slice()) {
                                    Some(idx) => Ok(py_tuple(vec![
                                        PyObjectRef::imm(PyObject::Bytes(b[..idx].to_vec())),
                                        PyObjectRef::imm(PyObject::Bytes(sep.clone())),
                                        PyObjectRef::imm(PyObject::Bytes(
                                            b[idx + sep.len()..].to_vec(),
                                        )),
                                    ])),
                                    None => Ok(py_tuple(vec![
                                        PyObjectRef::imm(PyObject::Bytes(b.clone())),
                                        PyObjectRef::imm(PyObject::Bytes(Vec::new())),
                                        PyObjectRef::imm(PyObject::Bytes(Vec::new())),
                                    ])),
                                }
                            } else {
                                Err(PyError::runtime_error("partition on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rpartition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rpartition".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "rpartition() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let sep = arg_bytes(&args[1]).ok_or_else(|| {
                                    PyError::type_error("argument should be a bytes-like object")
                                })?;
                                if sep.is_empty() {
                                    return Err(PyError::value_error("empty separator"));
                                }
                                match b.windows(sep.len()).rposition(|w| w == sep.as_slice()) {
                                    Some(idx) => Ok(py_tuple(vec![
                                        PyObjectRef::imm(PyObject::Bytes(b[..idx].to_vec())),
                                        PyObjectRef::imm(PyObject::Bytes(sep.clone())),
                                        PyObjectRef::imm(PyObject::Bytes(
                                            b[idx + sep.len()..].to_vec(),
                                        )),
                                    ])),
                                    None => Ok(py_tuple(vec![
                                        PyObjectRef::imm(PyObject::Bytes(Vec::new())),
                                        PyObjectRef::imm(PyObject::Bytes(Vec::new())),
                                        PyObjectRef::imm(PyObject::Bytes(b.clone())),
                                    ])),
                                }
                            } else {
                                Err(PyError::runtime_error("rpartition on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "splitlines" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "splitlines".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let keepends = args.get(1).map(|v| v.truthy()).unwrap_or(false);
                                let mut lines = Vec::new();
                                let mut start = 0;
                                let mut i = 0;
                                while i < b.len() {
                                    if b[i] == b'\n' || b[i] == b'\r' {
                                        let end = if b[i] == b'\r'
                                            && i + 1 < b.len()
                                            && b[i + 1] == b'\n'
                                        {
                                            i + 2
                                        } else {
                                            i + 1
                                        };
                                        lines.push(if keepends {
                                            b[start..end].to_vec()
                                        } else {
                                            b[start..i].to_vec()
                                        });
                                        start = end;
                                        i = end;
                                    } else {
                                        i += 1;
                                    }
                                }
                                if start < b.len() {
                                    lines.push(b[start..].to_vec());
                                }
                                Ok(py_list(
                                    lines
                                        .into_iter()
                                        .map(|v| PyObjectRef::imm(PyObject::Bytes(v)))
                                        .collect(),
                                ))
                            } else {
                                Err(PyError::runtime_error("splitlines on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "expandtabs" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "expandtabs".to_string(),
                        func: |args| {
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let tabsize = if args.len() > 1 {
                                    args[1].as_i64().unwrap_or(8).max(0) as usize
                                } else {
                                    8
                                };
                                let mut result = Vec::with_capacity(b.len());
                                let mut col = 0usize;
                                for &c in b.iter() {
                                    if c == b'\t' {
                                        if tabsize > 0 {
                                            let spaces = tabsize - (col % tabsize);
                                            result.extend(std::iter::repeat(b' ').take(spaces));
                                            col += spaces;
                                        }
                                    } else if c == b'\n' || c == b'\r' {
                                        result.push(c);
                                        col = 0;
                                    } else {
                                        result.push(c);
                                        col += 1;
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("expandtabs on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "zfill" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "zfill".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "zfill() takes exactly 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let w = args[1].as_i64().unwrap_or(0).max(0) as usize;
                                if w <= b.len() {
                                    return Ok(PyObjectRef::imm(PyObject::Bytes(b.clone())));
                                }
                                let has_sign = matches!(b.first(), Some(b'+') | Some(b'-'));
                                let (sign, rest): (&[u8], &[u8]) = if has_sign {
                                    (&b[..1], &b[1..])
                                } else {
                                    (&b[..0], &b[..])
                                };
                                let pad = w - b.len();
                                let mut result = sign.to_vec();
                                result.extend(std::iter::repeat(b'0').take(pad));
                                result.extend_from_slice(rest);
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("zfill on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "ljust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "ljust".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "ljust() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let w = args[1].as_i64().unwrap_or(0).max(0) as usize;
                                let fill = if args.len() > 2 {
                                    arg_bytes(&args[2])
                                        .and_then(|v| v.first().copied())
                                        .unwrap_or(b' ')
                                } else {
                                    b' '
                                };
                                let mut result = b.clone();
                                if w > b.len() {
                                    result.extend(std::iter::repeat(fill).take(w - b.len()));
                                }
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("ljust on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rjust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rjust".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "rjust() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let w = args[1].as_i64().unwrap_or(0).max(0) as usize;
                                let fill = if args.len() > 2 {
                                    arg_bytes(&args[2])
                                        .and_then(|v| v.first().copied())
                                        .unwrap_or(b' ')
                                } else {
                                    b' '
                                };
                                if w <= b.len() {
                                    return Ok(PyObjectRef::imm(PyObject::Bytes(b.clone())));
                                }
                                let mut result: Vec<u8> =
                                    std::iter::repeat(fill).take(w - b.len()).collect();
                                result.extend_from_slice(b);
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("rjust on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "center" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "center".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "center() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                let w = args[1].as_i64().unwrap_or(0).max(0) as usize;
                                let fill = if args.len() > 2 {
                                    arg_bytes(&args[2])
                                        .and_then(|v| v.first().copied())
                                        .unwrap_or(b' ')
                                } else {
                                    b' '
                                };
                                if w <= b.len() {
                                    return Ok(PyObjectRef::imm(PyObject::Bytes(b.clone())));
                                }
                                let pad = w - b.len();
                                let left = pad / 2;
                                let right = pad - left;
                                let mut result: Vec<u8> =
                                    std::iter::repeat(fill).take(left).collect();
                                result.extend_from_slice(b);
                                result.extend(std::iter::repeat(fill).take(right));
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("center on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "translate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "translate".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "translate() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Bytes(b) = &*args[0].borrow() {
                                // Keyword args arrive as a trailing dict
                                // (`bytes.translate(None, delete=b'...')` — the
                                // exact idiom shlex.quote's safe-check uses).
                                let mut delete_arg: Option<PyObjectRef> = None;
                                let mut table_arg = args.get(1).cloned();
                                if let Some(last) = args.last() {
                                    if let PyObject::Dict(d) = &*last.borrow() {
                                        for (k, v) in d.items() {
                                            if k.str() == "delete" {
                                                delete_arg = Some(v);
                                            }
                                        }
                                        if table_arg.is_some()
                                            && table_arg.as_ref().unwrap().is(last)
                                        {
                                            table_arg = None;
                                        }
                                    }
                                }
                                let table = match &table_arg {
                                    Some(t) if matches!(&*t.borrow(), PyObject::None) => None,
                                    Some(t) => arg_bytes(t),
                                    None => None,
                                };
                                let delete = match &delete_arg {
                                    Some(d) => arg_bytes(d).unwrap_or_default(),
                                    None => Vec::new(),
                                };
                                let mut result = Vec::with_capacity(b.len());
                                for &c in b.iter() {
                                    if delete.contains(&c) {
                                        continue;
                                    }
                                    match &table {
                                        Some(t) if t.len() == 256 => result.push(t[c as usize]),
                                        _ => result.push(c),
                                    }
                                }
                                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                            } else {
                                Err(PyError::runtime_error("translate on non-bytes"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "maketrans" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "maketrans".to_string(),
                        func: |a| crate::object::bytes_maketrans_builtin(&a[1..]),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Same gap, same fix, as `list`'s own `__getitem__` arm
                    // (see its doc comment) — `bytes` is a real migrated
                    // type too, but the dunder wasn't directly callable by
                    // name, only via the `[0]` subscript syntax itself.
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
                    "__iter__" => {
                        let bytes_clone = _v.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                crate::object::builtin_iter(&[PyObjectRef::new(PyObject::Bytes(bytes_clone.clone()))])
                            },
                        ))))
                    }
                    "__buffer__" => {
                        let b_clone = _v.clone();
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "__buffer__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__buffer__() takes exactly one argument"));
                                }
                                let flags = crate::object::extract_flags_for_buffer(&args[1])?;
                                crate::object::check_buffer_flags(flags)?;
                                let len = if let PyObject::Bytes(b) = &*args[0].borrow() { b.len() } else { 0 };
                                let view = PyObjectRef::new(PyObject::MemoryView { source: args[0].clone(), format: "B".to_string(), shape: vec![len], itemsize: 1, offset: 0, readonly: true, released: false });
                                crate::object::track_view_exporter(&view, args[0].clone());
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
                                // no view marking here; caller handles it
                                Ok(py_none())
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'bytes' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
