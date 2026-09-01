use crate::object::*;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::rc::Rc;

fn gzip_bytes_arg(obj: &PyObjectRef) -> PyResult<Vec<u8>> {
    match &*obj.borrow() {
        PyObject::Bytes(b) => Ok(b.clone()),
        PyObject::ByteArray(b) => Ok(b.clone()),
        PyObject::Str(s) => Ok(s.as_bytes().to_vec()),
        _ => Err(PyError::type_error("a bytes-like object is required")),
    }
}

/// Pull `compresslevel`/`mtime` out of trailing args, which may be a bare
/// positional int (compresslevel) and/or a trailing kwargs dict (since
/// `call_function` packs keyword arguments into one dict appended after
/// positionals).
fn gzip_parse_level_mtime(rest: &[PyObjectRef]) -> (u32, Option<u32>) {
    let mut compresslevel: u32 = 9;
    let mut mtime: Option<u32> = None;
    for a in rest {
        match &*a.borrow() {
            PyObject::Int(i) => {
                if let Some(n) = i.to_i64() {
                    compresslevel = n as u32;
                }
            }
            PyObject::Dict(dct) => {
                if let Ok(Some(v)) = dct.get(&py_str("compresslevel")) {
                    if let Some(n) = v.as_i64() {
                        compresslevel = n as u32;
                    }
                }
                if let Ok(Some(v)) = dct.get(&py_str("mtime")) {
                    if let Some(n) = v.as_i64() {
                        mtime = Some(n as u32);
                    }
                }
            }
            _ => {}
        }
    }
    (compresslevel, mtime)
}

/// Build a `GzipFile`-like instance (used by both `gzip.open()` and
/// `gzip.GzipFile()`) following the BytesIO pattern: a fresh `Type` per
/// instance whose methods are `Closure`s capturing the shared native state
/// directly, rather than routing through the instance dict.
fn build_gzip_file(
    filename: &str,
    mode: &str,
    compresslevel: u32,
    mtime: Option<u32>,
    text: bool,
    encoding: &str,
) -> PyResult<PyObjectRef> {
    let writing = mode.contains('w') || mode.contains('a') || mode.contains('x');
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    type_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_: &[PyObjectRef]| Ok(py_none()),
        }),
    );
    type_dict.insert_str("mode", py_str(mode));
    type_dict.insert_str("name", py_str(filename));

    if writing {
        let file = std::fs::File::options()
            .write(true)
            .create(true)
            .append(mode.contains('a'))
            .truncate(!mode.contains('a'))
            .open(filename)
            .map_err(|e| PyError::os_error_from_io(&e))?;
        let encoder = flate2::GzBuilder::new()
            .mtime(mtime.unwrap_or(0))
            .write(file, flate2::Compression::new(compresslevel.min(9)));
        let enc_rc: Rc<RefCell<Option<flate2::write::GzEncoder<std::fs::File>>>> =
            Rc::new(RefCell::new(Some(encoder)));

        let enc_write = enc_rc.clone();
        let encoding_owned = encoding.to_string();
        type_dict.insert_str(
            "write",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error("write() takes exactly one argument"));
                }
                let bytes = if text {
                    args[0].str().into_bytes()
                } else {
                    gzip_bytes_arg(&args[0])?
                };
                let mut slot = enc_write.borrow_mut();
                let enc = slot
                    .as_mut()
                    .ok_or_else(|| PyError::value_error("I/O operation on closed file"))?;
                enc.write_all(&bytes)
                    .map_err(|e| PyError::os_error_from_io(&e))?;
                let _ = &encoding_owned;
                Ok(py_int(bytes.len() as i64))
            }))),
        );

        let enc_flush = enc_rc.clone();
        type_dict.insert_str(
            "flush",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                if let Some(enc) = enc_flush.borrow_mut().as_mut() {
                    enc.flush().map_err(|e| PyError::os_error_from_io(&e))?;
                }
                Ok(py_none())
            }))),
        );

        let enc_close = enc_rc.clone();
        type_dict.insert_str(
            "close",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                if let Some(enc) = enc_close.borrow_mut().take() {
                    enc.finish().map_err(|e| PyError::os_error_from_io(&e))?;
                }
                Ok(py_none())
            }))),
        );
    } else {
        let file = std::fs::File::open(filename).map_err(|e| PyError::os_error_from_io(&e))?;
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut data = Vec::new();
        decoder
            .read_to_end(&mut data)
            .map_err(|e| PyError::os_error_from_io(&e))?;
        let buf_rc = Rc::new(RefCell::new(data));
        let pos_rc = Rc::new(RefCell::new(0usize));
        let encoding_owned = encoding.to_string();

        let b_read = buf_rc.clone();
        let p_read = pos_rc.clone();
        let enc_read = encoding_owned.clone();
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
                if text {
                    Ok(py_str(&decode_bytes(&chunk, &enc_read)))
                } else {
                    Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
                }
            }))),
        );

        let b_readline = buf_rc.clone();
        let p_readline = pos_rc.clone();
        let enc_readline = encoding_owned.clone();
        type_dict.insert_str(
            "readline",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                let data = b_readline.borrow();
                let pos = (*p_readline.borrow()).min(data.len());
                let remaining = &data[pos..];
                let end = remaining
                    .iter()
                    .position(|&c| c == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(remaining.len());
                let chunk = remaining[..end].to_vec();
                *p_readline.borrow_mut() = pos + end;
                if text {
                    Ok(py_str(&decode_bytes(&chunk, &enc_readline)))
                } else {
                    Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
                }
            }))),
        );

        let b_readlines = buf_rc.clone();
        let p_readlines = pos_rc.clone();
        let enc_readlines = encoding_owned.clone();
        type_dict.insert_str(
            "readlines",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                let data = b_readlines.borrow();
                let pos = (*p_readlines.borrow()).min(data.len());
                let remaining = &data[pos..];
                let lines: Vec<PyObjectRef> = remaining
                    .split_inclusive(|&c| c == b'\n')
                    .map(|line| {
                        if text {
                            py_str(&decode_bytes(line, &enc_readlines))
                        } else {
                            PyObjectRef::imm(PyObject::Bytes(line.to_vec()))
                        }
                    })
                    .collect();
                *p_readlines.borrow_mut() = data.len();
                Ok(py_list(lines))
            }))),
        );

        let b_iter = buf_rc.clone();
        let p_iter = pos_rc.clone();
        let enc_iter = encoding_owned.clone();
        type_dict.insert_str(
            "__iter__",
            PyObjectRef::new(PyObject::Closure(Rc::new(
                move |self_args: &[PyObjectRef]| {
                    let _ = (&b_iter, &p_iter, &enc_iter);
                    Ok(self_args.first().cloned().unwrap_or_else(py_none))
                },
            ))),
        );
        let b_next = buf_rc.clone();
        let p_next = pos_rc.clone();
        let enc_next = encoding_owned.clone();
        type_dict.insert_str(
            "__next__",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                let data = b_next.borrow();
                let pos = (*p_next.borrow()).min(data.len());
                if pos >= data.len() {
                    return Err(PyError::StopIteration);
                }
                let remaining = &data[pos..];
                let end = remaining
                    .iter()
                    .position(|&c| c == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(remaining.len());
                let chunk = remaining[..end].to_vec();
                *p_next.borrow_mut() = pos + end;
                if text {
                    Ok(py_str(&decode_bytes(&chunk, &enc_next)))
                } else {
                    Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
                }
            }))),
        );

        type_dict.insert_str(
            "close",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(py_none())
            }))),
        );
    }

    type_dict.insert_str(
        "__enter__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |args: &[PyObjectRef]| Ok(args[0].clone()),
        }),
    );
    let close_for_exit = type_dict.get_str("close").cloned();
    if let Some(close_fn) = close_for_exit {
        type_dict.insert_str(
            "__exit__",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                call_function(&close_fn, vec![])?;
                Ok(py_bool(false))
            }))),
        );
    }

    Ok(PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Type {
            name: "GzipFile".to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        }),
        dict: AttrMap::new(),
    }))
}

fn decode_bytes(bytes: &[u8], encoding: &str) -> String {
    match encoding.to_ascii_lowercase().as_str() {
        "latin-1" | "latin1" | "iso-8859-1" => bytes.iter().map(|&b| b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub fn create_gzip_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! gz_func {
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

    // gzip header FLG bits (see RFC 1952)
    d.insert_str("FTEXT", py_int(1));
    d.insert_str("FHCRC", py_int(2));
    d.insert_str("FEXTRA", py_int(4));
    d.insert_str("FNAME", py_int(8));
    d.insert_str("FCOMMENT", py_int(16));

    gz_func!("open", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "open() takes at least 1 argument (filename)",
            ));
        }
        let filename = args[0].borrow().str();
        let mut mode = "rb".to_string();
        let mut encoding = "utf-8".to_string();
        for a in &args[1..] {
            match &*a.borrow() {
                PyObject::Str(s) => mode = s.to_string(),
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("mode")) {
                        mode = v.str();
                    }
                    if let Ok(Some(v)) = dct.get(&py_str("encoding")) {
                        encoding = v.str();
                    }
                }
                _ => {}
            }
        }
        let text = mode.contains('t');
        let binary_mode: String = mode.chars().filter(|&c| c != 't').collect();
        let binary_mode = if binary_mode.is_empty()
            || binary_mode == "r"
            || binary_mode == "w"
            || binary_mode == "a"
        {
            format!("{}b", binary_mode)
        } else {
            binary_mode
        };
        build_gzip_file(&filename, &binary_mode, 9, None, text, &encoding)
    });

    gz_func!("GzipFile", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("GzipFile() requires a filename"));
        }
        let filename = args[0].borrow().str();
        let mut mode = "rb".to_string();
        let mut compresslevel: u32 = 9;
        for a in &args[1..] {
            match &*a.borrow() {
                PyObject::Str(s) => mode = s.to_string(),
                PyObject::Int(i) => {
                    if let Some(n) = i.to_i64() {
                        compresslevel = n as u32;
                    }
                }
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("mode")) {
                        mode = v.str();
                    }
                    if let Ok(Some(v)) = dct.get(&py_str("compresslevel")) {
                        if let Some(n) = v.as_i64() {
                            compresslevel = n as u32;
                        }
                    }
                }
                _ => {}
            }
        }
        build_gzip_file(&filename, &mode, compresslevel, None, false, "utf-8")
    });

    gz_func!("compress", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("compress() takes at least 1 argument"));
        }
        let bytes = gzip_bytes_arg(&args[0])?;
        let (compresslevel, mtime) = gzip_parse_level_mtime(&args[1..]);
        let mut encoder = flate2::GzBuilder::new()
            .mtime(mtime.unwrap_or(0))
            .write(Vec::new(), flate2::Compression::new(compresslevel.min(9)));
        encoder
            .write_all(&bytes)
            .map_err(|e| PyError::os_error_from_io(&e))?;
        let result = encoder
            .finish()
            .map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(PyObjectRef::new(PyObject::Bytes(result)))
    });

    gz_func!("decompress", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error(
                "decompress() takes exactly one argument",
            ));
        }
        let bytes = gzip_bytes_arg(&args[0])?;
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| PyError::OsError(format!("gzip decompress error: {}", e)))?;
        Ok(PyObjectRef::new(PyObject::Bytes(out)))
    });

    d
}
