use crate::bytecode::{needs_arg, CodeObject};
use crate::interner;
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use crate::buffered_class;

pub fn create_io_module_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! io_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // FileIO — wraps std::fs::File via builtin_open
    io_func!("FileIO", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("FileIO() missing required argument"));
        }
        let filename = args[0].str();
        let mode = if args.len() > 1 {
            args[1].str()
        } else {
            "r".to_string()
        };
        let file = if let Some(fd) = args[0].as_i64() {
            use std::os::unix::io::FromRawFd;
            if fd < 0 {
                return Err(PyError::OsError("invalid file descriptor".to_string()));
            }
            // SAFETY: from_raw_fd is inherently unsafe because the caller must
            // guarantee the fd is valid and ownership is transferred. We at least
            // verify fd >= 0 as a basic sanity check.
            unsafe { std::fs::File::from_raw_fd(fd as i32) }
        } else {
            std::fs::File::options()
                .read(mode.contains('r') || mode == "wb")
                .write(mode.contains('w') || mode.contains('a'))
                .append(mode.contains('a'))
                .create(mode.contains('w') || mode.contains('a'))
                .truncate(mode.contains('w'))
                .open(&filename)
                .map_err(|e| PyError::os_error_from_io(&e))?
        };
        Ok(PyObjectRef::new(PyObject::File {
            file: Rc::new(RefCell::new(file)),
            name: filename.clone(),
            binary: mode.contains('b'),
            pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            closed: false,
        }))
    });

    // BytesIO — in-memory bytes buffer
    io_func!("BytesIO", |args| {
        let buf = if !args.is_empty() {
            let a = args[0].borrow();
            match &*a {
                PyObject::Bytes(b) => b.clone(),
                PyObject::Str(s) => s.as_bytes().to_vec(),
                _ => vec![],
            }
        } else {
            vec![]
        };
        let buf_rc = Rc::new(RefCell::new(buf));
        let pos_rc = Rc::new(RefCell::new(0usize));
        let mut type_dict = HashMap::new();

        type_dict.insert_str(
            "__init__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |_: &[PyObjectRef]| Ok(py_none()),
            }),
        );

        let b_read = buf_rc.clone();
        let p_read = pos_rc.clone();
        type_dict.insert_str(
            "read",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                let data = b_read.borrow();
                let pos = (*p_read.borrow()).min(data.len());
                let end = if !args.is_empty() {
                    args[0]
                        .as_i64()
                        .filter(|&n| n >= 0)
                        .map(|n| (pos + n as usize).min(data.len()))
                        .unwrap_or(data.len())
                } else {
                    data.len()
                };
                let chunk = data[pos..end].to_vec();
                *p_read.borrow_mut() = end;
                Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
            }))),
        );

        // `readinto(b)` — missing entirely (`AttributeError`), a real,
        // commonly-used method (e.g. `shutil.copyfileobj`-style buffered-
        // read loops). Reads up to `len(b)` bytes into the given writable
        // buffer, returns the number of bytes actually read.
        let b_readinto = buf_rc.clone();
        let p_readinto = pos_rc.clone();
        type_dict.insert_str(
            "readinto",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error("readinto() takes exactly one argument"));
                }
                let data = b_readinto.borrow();
                let pos = (*p_readinto.borrow()).min(data.len());
                match &mut *args[0].borrow_mut() {
                    PyObject::ByteArray(dest) => {
                        let n = dest.len().min(data.len() - pos);
                        dest[..n].copy_from_slice(&data[pos..pos + n]);
                        *p_readinto.borrow_mut() = pos + n;
                        Ok(py_int(n as i64))
                    }
                    _ => Err(PyError::type_error(
                        "argument must be read-write bytes-like object",
                    )),
                }
            }))),
        );

        let b_readline = buf_rc.clone();
        let p_readline = pos_rc.clone();
        type_dict.insert_str(
            "readline",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                let data = b_readline.borrow();
                let pos = (*p_readline.borrow()).min(data.len());
                let remaining = &data[pos..];
                // Respect size limit if provided (CPython's readline(size) caps
                // the returned bytes, even if the line is longer)
                let size_limit = args.get(0).and_then(|a| a.as_i64()).filter(|&n| n >= 0).map(|n| n as usize);
                let mut end = remaining
                    .iter()
                    .position(|&c| c == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(remaining.len());
                if let Some(limit) = size_limit {
                    end = end.min(limit);
                }
                let chunk = remaining[..end].to_vec();
                *p_readline.borrow_mut() = pos + end;
                Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
            }))),
        );

        let b_write = buf_rc.clone();
        let p_write = pos_rc.clone();
        type_dict.insert_str(
            "write",
            PyObjectRef::new(PyObject::Closure(Rc::new(
                move |w_args: &[PyObjectRef]| {
                    if w_args.is_empty() {
                        return Err(PyError::type_error("write() takes exactly one argument"));
                    }
                    let data = match &*w_args[0].borrow() {
                        PyObject::Bytes(b) => b.clone(),
                        PyObject::ByteArray(b) => b.clone(),
                        _ => {
                            return Err(PyError::type_error(
                                "a bytes-like object is required, not str",
                            ))
                        }
                    };
                    let mut buf = b_write.borrow_mut();
                    let pos = *p_write.borrow();
                    if pos + data.len() > buf.len() {
                        buf.resize(pos, 0);
                        buf.extend_from_slice(&data);
                    } else {
                        buf[pos..pos + data.len()].copy_from_slice(&data);
                    }
                    *p_write.borrow_mut() = pos + data.len();
                    Ok(py_int(data.len() as i64))
                },
            ))),
        );

        let b_seek = buf_rc.clone();
        let p_seek = pos_rc.clone();
        type_dict.insert_str(
            "seek",
            PyObjectRef::new(PyObject::Closure(Rc::new(
                move |s_args: &[PyObjectRef]| {
                    let offset = s_args.first().and_then(|a| a.as_i64()).unwrap_or(0);
                    let whence = s_args.get(1).and_then(|a| a.as_i64()).unwrap_or(0);
                    let len = b_seek.borrow().len() as i64;
                    let base = match whence {
                        1 => *p_seek.borrow() as i64,
                        2 => len,
                        _ => 0,
                    };
                    let new_pos = (base + offset).max(0) as usize;
                    *p_seek.borrow_mut() = new_pos;
                    Ok(py_int(new_pos as i64))
                },
            ))),
        );

        let p_tell = pos_rc.clone();
        type_dict.insert_str(
            "tell",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(py_int(*p_tell.borrow() as i64))
            }))),
        );

        let b_getvalue = buf_rc.clone();
        type_dict.insert_str(
            "getvalue",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(PyObjectRef::imm(PyObject::Bytes(
                    b_getvalue.borrow().clone(),
                )))
            }))),
        );

        type_dict.insert_str(
            "flush",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(py_none())
            }))),
        );

        type_dict.insert_str(
            "truncate",
            PyObjectRef::new(PyObject::Closure(Rc::new({
                let b = buf_rc.clone();
                let p = pos_rc.clone();
                move |args: &[PyObjectRef]| {
                    let size = args.get(0).and_then(|a| a.as_i64()).map(|n| n.max(0) as usize).unwrap_or(*p.borrow());
                    let mut buf = b.borrow_mut();
                    if size < buf.len() {
                        buf.truncate(size);
                    } else if size > buf.len() {
                        buf.resize(size, 0);
                    }
                    if *p.borrow() > size {
                        *p.borrow_mut() = size;
                    }
                    Ok(py_int(size as i64))
                }
            }))),
        );

        type_dict.insert_str(
            "close",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(py_none())
            }))),
        );

        type_dict.insert_str(
            "__enter__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__enter__".to_string(),
                func: |args: &[PyObjectRef]| Ok(args[0].clone()),
            }),
        );
        type_dict.insert_str(
            "__exit__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__exit__".to_string(),
                func: |_: &[PyObjectRef]| Ok(py_bool(false)),
            }),
        );

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "BytesIO".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        }))
    });

    // IncrementalNewlineDecoder — stub
    io_func!("IncrementalNewlineDecoder", |_args| {
        let mut type_dict = AttrMap::new();
        type_dict.insert_str(
            "decode",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "decode".to_string(),
                func: |m_args| {
                    if m_args.len() < 2 {
                        return Err(PyError::type_error("decode() takes 1 argument"));
                    }
                    match &*m_args[1].borrow() {
                        PyObject::Bytes(b) => Ok(py_str(&String::from_utf8_lossy(&b[..]))),
                        _ => Err(PyError::type_error("decode() argument must be bytes")),
                    }
                },
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("IncrementalNewlineDecoder"),
            dict: type_dict,
        }))
    });

    io_func!("open_code", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("open_code() missing argument"));
        }
        let path = args[0].str();
        let file = std::fs::File::open(&path).map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(PyObjectRef::new(PyObject::File {
            file: Rc::new(RefCell::new(file)),
            name: path.clone(),
            binary: true,
            pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            closed: false,
        }))
    });

    io_func!("text_encoding", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("text_encoding() missing argument"));
        }
        Ok(py_str(&args[0].str()))
    });

    d.insert_str(
        "open",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "open".to_string(),
            func: builtin_open,
        }),
    );
    d.insert_str("DEFAULT_BUFFER_SIZE", py_int(8192));

    // BlockingIOError — exception type (needs to support attribute setting like __module__)
    d.insert_str(
        "BlockingIOError",
        PyObjectRef::new(PyObject::Type {
            name: "BlockingIOError".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        }),
    );

    // UnsupportedOperation — exception type (needs __module__ set by io.py)
    let mut uo_dict = HashMap::new();
    uo_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    d.insert_str(
        "UnsupportedOperation",
        PyObjectRef::new(PyObject::Type {
            name: "UnsupportedOperation".to_string(),
            dict: Box::new(str_map_to_typedict(uo_dict)),
            bases: vec![],
            mro: vec![],
        }),
    );

    // ── IO Base Classes ─────────────────────────────────────────────────────────

    // IOBase — abstract base class with close, closed, __enter__, __exit__
    let mut iobase_dict = HashMap::new();
    iobase_dict.insert_str("__doc__", py_str("IOBase abstract class"));
    iobase_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    iobase_dict.insert_str(
        "close",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    let closed_getter = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "closed".to_string(),
        func: |_: &[PyObjectRef]| Ok(py_bool(false)),
    });
    iobase_dict.insert_str(
        "closed",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(closed_getter),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    iobase_dict.insert_str(
        "__enter__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |args: &[PyObjectRef]| Ok(args[0].clone()),
        }),
    );
    iobase_dict.insert_str(
        "__exit__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__exit__".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    let iobase_cls = PyObjectRef::new(PyObject::Type {
        name: "IOBase".to_string(),
        dict: Box::new(str_map_to_typedict(iobase_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("IOBase", iobase_cls.clone());
    d.insert_str("_IOBase", iobase_cls.clone());

    // RawIOBase — extends IOBase
    let mut raw_dict = HashMap::new();
    raw_dict.insert_str("__doc__", py_str("RawIOBase abstract class"));
    raw_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    raw_dict.insert_str(
        "read",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "read".to_string(),
            func: |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(vec![]))),
        }),
    );
    raw_dict.insert_str(
        "readinto",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "readinto".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    raw_dict.insert_str(
        "write",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "write".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_int(0)),
        }),
    );
    raw_dict.insert_str(
        "close",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    raw_dict.insert_str(
        "register",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "register".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    let raw_cls = PyObjectRef::new(PyObject::Type {
        name: "RawIOBase".to_string(),
        dict: Box::new(str_map_to_typedict(raw_dict)),
        bases: vec![iobase_cls.clone()],
        mro: vec![iobase_cls.clone()],
    });
    d.insert_str("RawIOBase", raw_cls.clone());
    d.insert_str("_RawIOBase", raw_cls.clone());

    // BufferedIOBase — extends IOBase
    let mut buf_dict = HashMap::new();
    buf_dict.insert_str("__doc__", py_str("BufferedIOBase abstract class"));
    buf_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    buf_dict.insert_str(
        "read",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "read".to_string(),
            func: |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(vec![]))),
        }),
    );
    buf_dict.insert_str(
        "read1",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "read1".to_string(),
            func: |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(vec![]))),
        }),
    );
    buf_dict.insert_str(
        "write",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "write".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    buf_dict.insert_str(
        "close",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    buf_dict.insert_str(
        "register",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "register".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    let buf_cls = PyObjectRef::new(PyObject::Type {
        name: "BufferedIOBase".to_string(),
        dict: Box::new(str_map_to_typedict(buf_dict)),
        bases: vec![iobase_cls.clone()],
        mro: vec![iobase_cls.clone()],
    });
    d.insert_str("BufferedIOBase", buf_cls.clone());
    d.insert_str("_BufferedIOBase", buf_cls.clone());

    // TextIOBase — text I/O base class (extends IOBase)
    let mut text_dict = HashMap::new();
    text_dict.insert_str("__doc__", py_str("TextIOBase abstract class"));
    text_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    text_dict.insert_str(
        "read",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "read".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_str("")),
        }),
    );
    text_dict.insert_str(
        "write",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "write".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    text_dict.insert_str(
        "close",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    text_dict.insert_str(
        "register",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "register".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    let text_cls = PyObjectRef::new(PyObject::Type {
        name: "TextIOBase".to_string(),
        dict: Box::new(str_map_to_typedict(text_dict)),
        bases: vec![iobase_cls.clone()],
        mro: vec![iobase_cls.clone()],
    });
    d.insert_str("TextIOBase", text_cls.clone());
    d.insert_str("_TextIOBase", text_cls.clone());

    // StringIO — real in-memory text buffer with actual position tracking
    // (char-indexed, matching Python's own str model — NOT byte-indexed).
    // The PREVIOUS implementation was a near-total stub: `read()` ignored
    // any size argument and always returned the ENTIRE buffer regardless of
    // position, and `seek`/`tell` were hardcoded to always return 0 — no
    // position tracking existed at all. This made the extremely common
    // `while True: chunk = f.read(1)\n if not chunk: break` idiom loop
    // FOREVER (`read(1)` never shrinks, never returns `''`) — confirmed via
    // CPython's own `shlex.py` (`shlex.split(...)` hung indefinitely on any
    // input). Position is tracked in a `Rc<RefCell<usize>>` (char offset,
    // not byte offset) alongside the buffer.
    let text_cls_tiw = text_cls.clone(); // for TextIOWrapper below
    let stringio_closure: Rc<dyn Fn(&[PyObjectRef]) -> PyResult<PyObjectRef>> =
        Rc::new(move |args: &[PyObjectRef]| {
            let initial_value = if !args.is_empty() {
                match &*args[0].borrow() {
                    PyObject::Str(s) => s.to_string(),
                    _ => String::new(),
                }
            } else {
                String::new()
            };
            let buffer = Rc::new(RefCell::new(initial_value));
            let pos = Rc::new(RefCell::new(0usize));
            let mut type_dict = HashMap::new();

            // __init__ — no-op (initial_value already consumed by factory)
            type_dict.insert_str(
                "__init__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__init__".to_string(),
                    func: |_: &[PyObjectRef]| Ok(py_none()),
                }),
            );

            // Optional size arg: absent, explicit None, or negative all mean
            // "no limit" (read to end / no truncation), matching real
            // `read(size=-1)`/`truncate(size=None)` semantics.
            fn opt_size(args: &[PyObjectRef], idx: usize) -> Option<i64> {
                let a = args.get(idx)?;
                if matches!(&*a.borrow(), PyObject::None) {
                    return None;
                }
                let n = a.as_i64()?;
                if n < 0 {
                    None
                } else {
                    Some(n)
                }
            }

            // read(size=-1) — from the current position, advancing it.
            let (b, p) = (buffer.clone(), pos.clone());
            type_dict.insert_str(
                "read",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                    let chars: Vec<char> = b.borrow().chars().collect();
                    let start = (*p.borrow()).min(chars.len());
                    let end = match opt_size(args, 0) {
                        Some(n) => (start + n as usize).min(chars.len()),
                        None => chars.len(),
                    };
                    *p.borrow_mut() = end;
                    Ok(py_str(&chars[start..end].iter().collect::<String>()))
                }))),
            );

            // readline(size=-1) — up to and including the next '\n', or EOF.
            let (b, p) = (buffer.clone(), pos.clone());
            type_dict.insert_str(
                "readline",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                    let chars: Vec<char> = b.borrow().chars().collect();
                    let start = (*p.borrow()).min(chars.len());
                    let limit = opt_size(args, 0)
                        .map(|n| (start + n as usize).min(chars.len()))
                        .unwrap_or(chars.len());
                    let mut end = start;
                    while end < limit {
                        if chars[end] == '\n' {
                            end += 1;
                            break;
                        }
                        end += 1;
                    }
                    *p.borrow_mut() = end;
                    Ok(py_str(&chars[start..end].iter().collect::<String>()))
                }))),
            );

            // write(s) — overwrite at the current position (extending the
            // buffer if writing past its current end), then advance position
            // by the written length. Matches real `StringIO.write`'s
            // "positioned write", not a plain append.
            let (b, p) = (buffer.clone(), pos.clone());
            type_dict.insert_str(
                "write",
                PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |w_args: &[PyObjectRef]| {
                        let text = if !w_args.is_empty() {
                            w_args[0].str()
                        } else {
                            String::new()
                        };
                        let mut chars: Vec<char> = b.borrow().chars().collect();
                        let start = *p.borrow();
                        while chars.len() < start {
                            chars.push('\0');
                        }
                        let new_chars: Vec<char> = text.chars().collect();
                        let end = start + new_chars.len();
                        if end > chars.len() {
                            chars.truncate(start);
                            chars.extend(new_chars.iter());
                        } else {
                            chars.splice(start..end, new_chars.iter().cloned());
                        }
                        *b.borrow_mut() = chars.into_iter().collect();
                        *p.borrow_mut() = end;
                        Ok(py_int(text.chars().count() as i64))
                    },
                ))),
            );

            // getvalue — full buffer contents regardless of current position.
            let b_get = buffer.clone();
            type_dict.insert_str(
                "getvalue",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                    Ok(py_str(&b_get.borrow()))
                }))),
            );

            // close — no-op
            type_dict.insert_str(
                "close",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                    Ok(py_none())
                }))),
            );

            // seek(pos, whence=0) — 0=absolute, 1=relative, 2=from end.
            let (b, p) = (buffer.clone(), pos.clone());
            type_dict.insert_str(
                "seek",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                    let target = args.get(0).and_then(|a| a.as_i64()).unwrap_or(0);
                    let whence = args.get(1).and_then(|a| a.as_i64()).unwrap_or(0);
                    let len = b.borrow().chars().count() as i64;
                    let new_pos = match whence {
                        1 => *p.borrow() as i64 + target,
                        2 => len + target,
                        _ => target,
                    };
                    let new_pos = new_pos.max(0) as usize;
                    *p.borrow_mut() = new_pos;
                    Ok(py_int(new_pos as i64))
                }))),
            );

            // tell — current position.
            let p_tell = pos.clone();
            type_dict.insert_str(
                "tell",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                    Ok(py_int(*p_tell.borrow() as i64))
                }))),
            );

            // truncate(size=None) — cut the buffer at `size` chars (current
            // position if omitted); does NOT move the current position (matches
            // real `io.StringIO.truncate`).
            let (b, p) = (buffer.clone(), pos.clone());
            type_dict.insert_str(
                "truncate",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                    let mut chars: Vec<char> = b.borrow().chars().collect();
                    let size = opt_size(args, 0)
                        .map(|n| n as usize)
                        .unwrap_or(*p.borrow())
                        .min(chars.len());
                    chars.truncate(size);
                    *b.borrow_mut() = chars.into_iter().collect();
                    Ok(py_int(size as i64))
                }))),
            );

            // flush — no-op (real StringIO flush does nothing)
            type_dict.insert_str(
                "flush",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                    Ok(py_none())
                }))),
            );

            // readlines(hint=-1) — split remaining content into lines (each
            // keeping its trailing '\n' except possibly the last).
            let (b, p) = (buffer.clone(), pos.clone());
            type_dict.insert_str(
                "readlines",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                    let chars: Vec<char> = b.borrow().chars().collect();
                    let start = (*p.borrow()).min(chars.len());
                    let rest: String = chars[start..].iter().collect();
                    *p.borrow_mut() = chars.len();
                    let mut lines = Vec::new();
                    let mut cur = String::new();
                    for c in rest.chars() {
                        cur.push(c);
                        if c == '\n' {
                            lines.push(py_str(&cur));
                            cur.clear();
                        }
                    }
                    if !cur.is_empty() {
                        lines.push(py_str(&cur));
                    }
                    Ok(py_list(lines))
                }))),
            );

            // __iter__/__next__ — iterate remaining lines, StopIteration at EOF.
            type_dict.insert_str(
                "__iter__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__iter__".to_string(),
                    func: |args: &[PyObjectRef]| Ok(args[0].clone()),
                }),
            );
            let (b, p) = (buffer.clone(), pos.clone());
            type_dict.insert_str(
                "__next__",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                    let chars: Vec<char> = b.borrow().chars().collect();
                    let start = (*p.borrow()).min(chars.len());
                    if start >= chars.len() {
                        return Err(PyError::StopIteration);
                    }
                    let mut end = start;
                    while end < chars.len() {
                        if chars[end] == '\n' {
                            end += 1;
                            break;
                        }
                        end += 1;
                    }
                    *p.borrow_mut() = end;
                    Ok(py_str(&chars[start..end].iter().collect::<String>()))
                }))),
            );
            type_dict.insert_str(
                "__enter__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__enter__".to_string(),
                    func: |args: &[PyObjectRef]| Ok(args[0].clone()),
                }),
            );
            type_dict.insert_str(
                "__exit__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__exit__".to_string(),
                    func: |_: &[PyObjectRef]| Ok(py_bool(false)),
                }),
            );

            Ok(PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "StringIO".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![text_cls.clone()],
                    mro: vec![text_cls.clone()],
                }),
                dict: AttrMap::new(),
            }))
        });
    d.insert_str(
        "StringIO",
        PyObjectRef::new(PyObject::Closure(stringio_closure)),
    );

    let br_cls = buffered_class!("BufferedReader", buf_cls);
    d.insert_str("BufferedReader", br_cls.clone());
    let bw_cls = buffered_class!("BufferedWriter", buf_cls);
    d.insert_str("BufferedWriter", bw_cls.clone());
    let brp_cls = buffered_class!("BufferedRWPair", buf_cls);
    d.insert_str("BufferedRWPair", brp_cls.clone());
    let brnd_cls = buffered_class!("BufferedRandom", buf_cls);
    d.insert_str("BufferedRandom", brnd_cls.clone());

    // TextIOWrapper shares the delegation behavior, adding text-mode bits.
    let tiw_inner = {
        let mut td: HashMap<String, PyObjectRef> = HashMap::new();
        let bf = |name: &'static str, f: crate::object::BuiltinFunc| {
            PyObjectRef::new(PyObject::BuiltinFunction { name: name.to_string(), func: f })
        };
        td.insert("__init__".into(), bf("__init__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("missing required argument 'buffer'"));
            }
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("_raw", args[1].clone());
                dict.insert_str("_closed", py_bool(false));
                dict.insert_str("encoding", py_str("utf-8"));
                dict.insert_str("errors", py_str("strict"));
                dict.insert_str("newlines", py_none());
            }
            Ok(py_none())
        }));
        td.insert("read".into(), bf("read", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            let n = args.get(1).cloned().unwrap_or_else(|| py_int(-1));
            let data = match crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "read", vec![n])) {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(e),
            };
            if let PyObject::Bytes(ref b) = &*data.borrow() {
                return Ok(py_str(&String::from_utf8_lossy(&b[..])));
            }
            Ok(data)
        }));
        td.insert("readline".into(), bf("readline", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            let n = args.get(1).cloned().unwrap_or_else(|| py_int(-1));
            let data = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "readline", vec![n]))??;
            if let PyObject::Bytes(ref b) = &*data.borrow() {
                return Ok(py_str(&String::from_utf8_lossy(&b[..])));
            }
            Ok(data)
        }));
        td.insert("write".into(), bf("write", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let text = args.get(1).cloned().ok_or_else(|| PyError::type_error("write missing text"))?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            let payload = match &*text.borrow() {
                PyObject::Str(s2) => PyObjectRef::imm(PyObject::Bytes(s2.as_bytes().to_vec())),
                _ => text.clone(),
            };
            let r = match crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "write", vec![payload])) {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(e),
            };
            if matches!(&*r.borrow(), PyObject::None) {
                let n = match &*text.borrow() { PyObject::Str(s2) => s2.chars().count() as i64, _ => 0 };
                Ok(py_int(n))
            } else { Ok(r) }
        }));
        td.insert("reconfigure".into(), bf("reconfigure", |_a| Ok(py_none())));
        td.insert("seek".into(), bf("seek", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            let pos = args.get(1).cloned().unwrap_or_else(|| py_int(0));
            let wh = args.get(2).cloned().unwrap_or_else(|| py_int(0));
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "seek", vec![pos, wh]))?
        }));
        td.insert("tell".into(), bf("tell", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "tell", vec![]))?
        }));
        td.insert("flush".into(), bf("flush", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "flush", vec![]))?
        }));
        td.insert("close".into(), bf("close", |args: &[PyObjectRef]| {
            let already = args[0].borrow().get_attribute("_closed").ok().map(|v| v.truthy()).unwrap_or(false);
            if !already {
                let raw = crate::modules::dev::io_get_raw(args)?;
                let _ = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "close", vec![]));
                if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                    dict.insert_str("_closed", py_bool(true));
                }
            }
            Ok(py_none())
        }));
        td.insert("readable".into(), bf("readable", |_a| Ok(py_bool(true))));
        td.insert("writable".into(), bf("writable", |_a| Ok(py_bool(true))));
        td.insert("seekable".into(), bf("seekable", |_a| Ok(py_bool(true))));
        td.insert("detach".into(), bf("detach", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("_closed", py_bool(true));
            }
            Ok(raw)
        }));
        td.insert("buffer".into(), bf("buffer", |args: &[PyObjectRef]| {
            Ok(crate::modules::dev::io_get_raw(args)?)
        }));
        td.insert("__enter__".into(), bf("__enter__", |args: &[PyObjectRef]| Ok(args[0].clone())));
        td.insert("__exit__".into(), bf("__exit__", |_a| Ok(py_bool(false))));
        td
    };
    let tiw_cls = PyObjectRef::new(PyObject::Type {
        name: "TextIOWrapper".to_string(),
        dict: Box::new(str_map_to_typedict(tiw_inner)),
        bases: vec![text_cls_tiw.clone()],
        mro: vec![text_cls_tiw.clone()],
    });
    d.insert_str("TextIOWrapper", tiw_cls);

    d.insert_str("_WindowsConsoleIO", py_str("_WindowsConsoleIO"));

    d
}
