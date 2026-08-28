// Auto-extracted from src/object/attrs/mod.rs lines 4498-4985
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::File { file: f_rc, .. } => {
                match name {
                    "buffer" => {
                        // `sys.stdin.buffer`/`sys.stdout.buffer`/`stderr.
                        // buffer` — the binary view of a text stream (real
                        // trigger: quopri.py's `main`, run via `-mquopri`,
                        // does `fp = sys.stdin.buffer`). Return a File
                        // sharing the SAME underlying handle, in binary mode.
                        if let PyObject::File {
                            file, name: fname, ..
                        } = o
                        {
                            Ok(PyObjectRef::new(PyObject::File {
                                file: file.clone(),
                                name: fname.clone(),
                                binary: true,
                                pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                                closed: false,
                            }))
                        } else {
                            Err(PyError::runtime_error("buffer access on non-file"))
                        }
                    }
                    "name" => {
                        if let PyObject::File { name: fname, .. } = o {
                            Ok(py_str(fname))
                        } else {
                            Err(PyError::runtime_error("name access on non-file"))
                        }
                    }
                    "closed" => {
                        if let PyObject::File { closed, .. } = o {
                            Ok(py_bool(*closed))
                        } else {
                            Err(PyError::runtime_error("closed access on non-file"))
                        }
                    }
                    "fileno" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "fileno".to_string(),
                        func: |args| {
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                use std::os::unix::io::AsRawFd;
                                Ok(py_int(file.borrow().as_raw_fd() as i64))
                            } else {
                                Err(PyError::runtime_error("fileno on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "read" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "read".to_string(),
                        func: |args| {
                            use std::io::Read;
                            if let PyObject::File {
                                file,
                                binary,
                                pending,
                                ..
                            } = &*args[0].borrow()
                            {
                                // Was: unconditional `read_to_string`, always
                                // returning `str` — completely ignored an
                                // explicit `size` argument (`f.read(n)`, real
                                // trigger: `dbm/dumb.py`'s own `__getitem__`,
                                // `f.read(siz)` to read exactly one stored
                                // value's byte range out of a shared data
                                // file — got the ENTIRE rest of the file
                                // instead of just `siz` bytes every time),
                                // AND never returned `bytes` even for a file
                                // opened in binary (`'rb'`) mode.
                                let size = args.get(1).and_then(|a| a.as_i64());
                                let buf: Vec<u8> = match size {
                                    Some(n) if n >= 0 => {
                                        let mut buf = vec![0u8; n as usize];
                                        let read = file
                                            .borrow_mut()
                                            .read(&mut buf)
                                            .map_err(|e| PyError::os_error_from_io(&e))?;
                                        buf.truncate(read);
                                        buf
                                    }
                                    _ => {
                                        let mut buf = Vec::new();
                                        file.borrow_mut()
                                            .read_to_end(&mut buf)
                                            .map_err(|e| PyError::os_error_from_io(&e))?;
                                        buf
                                    }
                                };
                                if *binary {
                                    Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
                                } else {
                                    // Text-mode streaming decode: a size-limited
                                    // read must return whole CHARACTERS, so if
                                    // the chunk ends mid-multibyte-sequence,
                                    // keep reading bytes until the character
                                    // completes (or EOF) — otherwise
                                    // `f.read(1)`-at-a-time over a UTF-8 file
                                    // corrupted `¡¢` into `����` (each byte
                                    // lossy-decoded in isolation) and, worse,
                                    // returned "" before a char was ready,
                                    // which breaks the ubiquitous
                                    // `iter(f.read, "")` sentinel idiom
                                    // (`test_netrc.py::test_token_value_non_ascii`).
                                    let mut full: Vec<u8> =
                                        std::mem::take(&mut *pending.borrow_mut());
                                    full.extend_from_slice(&buf);
                                    loop {
                                        match std::str::from_utf8(&full) {
                                            Ok(s) => return Ok(py_str(s)),
                                            Err(e) if e.error_len().is_none() && size.is_some() => {
                                                // Incomplete trailing sequence
                                                // and this was a size-limited
                                                // read: pull more bytes to
                                                // finish the character.
                                                let mut extra = [0u8; 1];
                                                match file.borrow_mut().read(&mut extra) {
                                                    Ok(0) => {
                                                        // EOF — decode what we
                                                        // have lossily rather
                                                        // than hang forever.
                                                        return Ok(py_str(
                                                            &String::from_utf8_lossy(&full),
                                                        ));
                                                    }
                                                    Ok(_) => full.push(extra[0]),
                                                    Err(e) => {
                                                        return Err(PyError::os_error_from_io(&e))
                                                    }
                                                }
                                            }
                                            Err(_) => {
                                                // Genuinely invalid bytes, or
                                                // an incomplete tail at EOF:
                                                // lossy-decode everything
                                                // (preserving the pre-existing
                                                // lossy behavior so no existing
                                                // caller regresses).
                                                return Ok(py_str(&String::from_utf8_lossy(&full)));
                                            }
                                        }
                                    }
                                }
                            } else {
                                Err(PyError::runtime_error("read on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `readline()`/`readlines()`/iteration (`for line in f:`)
                    // were missing entirely — one of the single most common
                    // real-Python file-reading idioms. `std::fs::File` has
                    // no built-in line buffering, so this reads byte-by-byte
                    // via the file's OWN current position (the same handle
                    // `seek`/`tell` already operate on, so interleaving
                    // `readline()` with `seek()`/`tell()` stays consistent),
                    // stopping at (and including) `\n` or at EOF. Confirmed
                    // missing via `dbm/dumb.py`'s own `_update` (`for line
                    // in f:` over its index file) — `TypeError: 'file'
                    // object is not iterable` — but the gap is completely
                    // general, not dbm-specific.
                    "readline" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "readline".to_string(),
                        func: |args| {
                            use std::io::Read;
                            if let PyObject::File { file, binary, .. } = &*args[0].borrow() {
                                let limit = args.get(1).and_then(|a| a.as_i64()).filter(|&n| n >= 0).map(|n| n as usize);
                                let mut buf = Vec::new();
                                let mut byte = [0u8; 1];
                                loop {
                                    if let Some(lim) = limit {
                                        if buf.len() >= lim {
                                            break;
                                        }
                                    }
                                    match file.borrow_mut().read(&mut byte) {
                                        Ok(0) => break,
                                        Ok(_) => {
                                            buf.push(byte[0]);
                                            if byte[0] == b'\n' {
                                                break;
                                            }
                                        }
                                        Err(e) => return Err(PyError::os_error_from_io(&e)),
                                    }
                                }
                                if *binary {
                                    Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
                                } else {
                                    Ok(py_str(&String::from_utf8_lossy(&buf)))
                                }
                            } else {
                                Err(PyError::runtime_error("readline on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "readlines" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "readlines".to_string(),
                        func: |args| {
                            use std::io::Read;
                            if let PyObject::File { file, binary, .. } = &*args[0].borrow() {
                                let mut rest = Vec::new();
                                file.borrow_mut()
                                    .read_to_end(&mut rest)
                                    .map_err(|e| PyError::os_error_from_io(&e))?;
                                let mut lines: Vec<PyObjectRef> = Vec::new();
                                let mut current: Vec<u8> = Vec::new();
                                for byte in rest {
                                    current.push(byte);
                                    if byte == b'\n' {
                                        lines.push(if *binary {
                                            PyObjectRef::imm(PyObject::Bytes(current.clone()))
                                        } else {
                                            py_str(&String::from_utf8_lossy(&current))
                                        });
                                        current.clear();
                                    }
                                }
                                if !current.is_empty() {
                                    lines.push(if *binary {
                                        PyObjectRef::imm(PyObject::Bytes(current.clone()))
                                    } else {
                                        py_str(&String::from_utf8_lossy(&current))
                                    });
                                }
                                Ok(py_list(lines))
                            } else {
                                Err(PyError::runtime_error("readlines on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__next__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__next__".to_string(),
                        func: |args| {
                            use std::io::Read;
                            if let PyObject::File { file, binary, .. } = &*args[0].borrow() {
                                let mut buf = Vec::new();
                                let mut byte = [0u8; 1];
                                loop {
                                    match file.borrow_mut().read(&mut byte) {
                                        Ok(0) => break,
                                        Ok(_) => {
                                            buf.push(byte[0]);
                                            if byte[0] == b'\n' {
                                                break;
                                            }
                                        }
                                        Err(e) => return Err(PyError::os_error_from_io(&e)),
                                    }
                                }
                                if buf.is_empty() {
                                    return Err(PyError::StopIteration);
                                }
                                if *binary {
                                    Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
                                } else {
                                    Ok(py_str(&String::from_utf8_lossy(&buf)))
                                }
                            } else {
                                Err(PyError::runtime_error("__next__ on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "write" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "write".to_string(),
                        func: |args| {
                            use std::io::Write;
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "write() takes exactly one argument",
                                ));
                            }
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                // A binary-mode file's `write()` takes real
                                // `bytes` — was always calling `.str()` on
                                // the argument (a `bytes` value's `str()` is
                                // its Python REPR, `"b'...'"`, quotes/escapes
                                // and all — writing that literal text into
                                // the file instead of the actual raw bytes).
                                let data: Vec<u8> = match &*args[1].borrow() {
                                    PyObject::Bytes(b) => b.clone(),
                                    PyObject::ByteArray(b) => b.clone(),
                                    other => other.str().into_bytes(),
                                };
                                file.borrow_mut()
                                    .write_all(&data)
                                    .map_err(|e| PyError::os_error_from_io(&e))?;
                                Ok(py_int(data.len() as i64))
                            } else {
                                Err(PyError::runtime_error("write on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "flush" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "flush".to_string(),
                        func: |args| {
                            use std::io::Write;
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                file.borrow_mut()
                                    .flush()
                                    .map_err(|e| PyError::os_error_from_io(&e))?;
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("flush on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "close".to_string(),
                        func: |args| {
                            if let PyObject::File { file, closed, .. } = &mut *args[0].borrow_mut()
                            {
                                *closed = true;
                                // Flush and drop by replacing with a closed file
                                let _ = std::mem::replace(
                                    &mut *file.borrow_mut(),
                                    std::fs::File::create("/dev/null").unwrap_or(
                                        std::fs::File::open("/dev/null")
                                            .unwrap_or_else(|_| panic!()),
                                    ),
                                );
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("close on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            // args[0] = file_obj (normal path via LOAD_ATTR) or py_none (exception path via WITH_EXIT)
                            // args[1] = exc_type (normal) or file_obj (exception via BoundMethod wrapper)
                            // Find the file object: check args[0], then args[1]
                            let file_obj_idx = if args.len() > 0
                                && matches!(&*args[0].borrow(), PyObject::File { .. })
                            {
                                0
                            } else if args.len() > 1
                                && matches!(&*args[1].borrow(), PyObject::File { .. })
                            {
                                1
                            } else {
                                return Ok(py_none());
                            };
                            // Sync and flush data to disk
                            if let PyObject::File { file, .. } = &*args[file_obj_idx].borrow() {
                                let _ = file.borrow().sync_all();
                            }
                            // Replace with /dev/null to close the actual file descriptor
                            if let PyObject::File { file, closed, .. } =
                                &mut *args[file_obj_idx].borrow_mut()
                            {
                                *closed = true;
                                let _ = std::mem::replace(
                                    &mut *file.borrow_mut(),
                                    std::fs::File::open("/dev/null").unwrap_or_else(|_| {
                                        std::fs::File::create("/dev/null").unwrap()
                                    }),
                                );
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "seek" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "seek".to_string(),
                        func: |args| {
                            use std::io::SeekFrom;
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "seek() requires at least 1 argument",
                                ));
                            }
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                use std::io::Seek;
                                let offset = args[1].as_i64().unwrap_or(0);
                                let whence = if args.len() > 2 {
                                    args[2].as_i64().unwrap_or(0) as i32
                                } else {
                                    0
                                };
                                let pos = file
                                    .borrow_mut()
                                    .seek(match whence {
                                        1 => SeekFrom::Current(offset),
                                        2 => SeekFrom::End(offset),
                                        _ => SeekFrom::Start(offset as u64),
                                    })
                                    .map_err(|e| PyError::os_error_from_io(&e))?;
                                Ok(py_int(pos as i64))
                            } else {
                                Err(PyError::runtime_error("seek on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "tell" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "tell".to_string(),
                        func: |args| {
                            use std::io::Seek;
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                let pos = file
                                    .borrow_mut()
                                    .stream_position()
                                    .map_err(|e| PyError::os_error_from_io(&e))?;
                                Ok(py_int(pos as i64))
                            } else {
                                Err(PyError::runtime_error("tell on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isatty" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isatty".to_string(),
                        func: |args| {
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                let fd = {
                                    use std::os::unix::io::AsRawFd;
                                    file.borrow().as_raw_fd()
                                };
                                extern "C" {
                                    fn isatty(fd: i32) -> i32;
                                }
                                let is_tty = unsafe { isatty(fd) } != 0;
                                Ok(py_bool(is_tty))
                            } else {
                                Err(PyError::runtime_error("isatty on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "readable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "readable".to_string(),
                        func: |_| Ok(py_bool(true)),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "writable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "writable".to_string(),
                        func: |_| Ok(py_bool(true)),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "truncate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "truncate".to_string(),
                        func: |args| {
                            if let PyObject::File { file, .. } = &*args[0].borrow() {
                                let size = args.get(1).and_then(|a| a.as_i64()).map(|n| n as u64);
                                let mut f = file.borrow_mut();
                                use std::io::Seek;
                                let pos = f.stream_position().unwrap_or(0);
                                let target = size.unwrap_or(pos);
                                f.set_len(target).map_err(|e| PyError::os_error_from_io(&e))?;
                                if pos > target {
                                    f.seek(std::io::SeekFrom::Start(target)).ok();
                                }
                                Ok(py_int(target as i64))
                            } else {
                                Err(PyError::runtime_error("truncate on non-file"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "seekable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "seekable".to_string(),
                        func: |_| Ok(py_bool(true)),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'file' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
