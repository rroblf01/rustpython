use crate::bytecode::{needs_arg, CodeObject};
use crate::interner;
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

mod buffered;
use self::buffered::register_buffered_classes;

/// Set a freshly built native type's MRO to itself followed by its base's
/// linearization (builders can't name the class inside its own
/// initializer). Hand-built MROs here used to contain only the base —
/// so `TextIOWrapper.__mro__` was `['TextIOBase']` and any Python
/// subclass got an inverted MRO (`[S, TextIOBase, TextIOWrapper]`),
/// making `super().__init__` resolve to the wrong initializer and
/// silently skip real native state setup (real trigger: argparse's
/// `StdIOBuffer(TextIOWrapper)`, whose fileno()/isatty() then raised
/// AttributeError instead of OSError). Fix bottom-up so each base's
/// own MRO is already correct when a derived class chains onto it.
pub(crate) fn fixup_self_mro(cls: &PyObjectRef) {
    let chained: Vec<PyObjectRef> = {
        let borrowed = cls.borrow();
        if let PyObject::Type { bases, .. } = &*borrowed {
            let mut m = Vec::with_capacity(bases.len() + 1);
            m.push(cls.clone());
            for b in bases {
                if let PyObject::Type { mro: base_mro, .. } = &*b.borrow() {
                    for entry in base_mro {
                        if !m.iter().any(|e| e.is(entry)) {
                            m.push(entry.clone());
                        }
                    }
                } else if !m.iter().any(|e| e.is(b)) {
                    m.push(b.clone());
                }
            }
            m
        } else {
            return;
        }
    };
    if let PyObject::Type { mro, .. } = &mut *cls.borrow_mut() {
        *mro = chained;
    }
}

/// `IOBase.__exit__` marker — real logic lives in `iobase_exit_with_vm`
/// below and is dispatched to from `src/vm/call.rs` via `fn_addr_eq`
/// (needs a live `&mut VirtualMachine` to invoke `self.close()`, which the
/// plain `fn(&[PyObjectRef])` builtin calling convention can't provide —
/// see the `with_vm_mut`-is-unsound-from-live-bytecode gotcha). This bare
/// function must never actually run: reaching it means some caller invoked
/// `__exit__` through a path that skipped the `call.rs` interception.
pub fn iobase_exit_marker(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Deliberately distinct code from every other trivial stub in this file
    // (a bare `Ok(py_bool(false))` body is identical to several other
    // stubs and gets folded onto the SAME address by thin-LTO in the
    // `release` profile — a known hazard here, see the
    // `cpython_test_suite_compat` notes on "LTO identical-code-folding" —
    // which would make `fn_addr_eq` in `call.rs` false-negative-match this
    // against an unrelated stub and vice versa).
    Err(PyError::runtime_error(
        "IOBase.__exit__ marker reached directly (interception in call.rs missing)",
    ))
}

/// Real `IOBase.__exit__` body: call `self.close()` (through normal
/// attribute lookup, so a Python subclass's OVERRIDDEN `close()` —
/// e.g. `zipfile._ZipWriteFile.close()`, which flushes the compressor and
/// writes the final header/CRC — actually runs) and swallow no exception
/// (return `False`), matching CPython's `IOBase.__exit__`.
pub fn iobase_exit_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    let this = args
        .first()
        .ok_or_else(|| PyError::type_error("__exit__ missing 'self'"))?;
    // `get_attribute` alone returns `close` UNBOUND (a bare
    // `PyObject::BuiltinFunction`/`Function`, no `self` attached) — calling
    // it directly the way `LOAD_ATTR`'s normal dot-call syntax would raised
    // "close() missing 1 required positional argument: 'self'". Reuse the
    // same rebind-to-`target` helper `print()`'s own `flush()`/`write()`
    // calls use instead of hand-rolling attribute binding again here.
    crate::object::call_method_rebound(vm, this, "close", vec![])?;
    Ok(py_bool(false))
}

/// `TextIOWrapper`'s `read`/`readline`/`readlines`/`write`/`seek`/`tell`/
/// `flush`/`close`/`truncate`/`fileno`/`isatty`/`writelines`/`name`/`mode`
/// all share this ONE marker (dispatched by the `name` field the
/// `BuiltinFunction`/`BuiltinMethod` already carries — same convention as
/// `zipimporter_method_marker`), rather than one marker per method. Real
/// logic lives in `tiw_delegate_with_vm` below and is dispatched to from
/// `src/vm/call.rs` via `fn_addr_eq` — see `iobase_exit_marker`'s doc
/// comment (right above) for why a plain `fn(&[PyObjectRef])` builtin can't
/// do this itself. This bare function must never actually run.
pub fn tiw_delegate_marker(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::runtime_error(
        "TextIOWrapper delegate marker reached directly (interception in call.rs missing)",
    ))
}

/// Real body for every `TextIOWrapper` method/property dispatched through
/// `tiw_delegate_marker`. `args[0]` is always `self`; the raw handle backing
/// it is fetched via `io_get_raw`.
pub fn tiw_delegate_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    method: &str,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    // `close()` must be a no-op (not an error) on an already-closed file,
    // and `name`/`mode` never called `io_ensure_open` either — match the
    // original per-method behavior exactly.
    if method != "close" && method != "name" && method != "mode" && method != "mode_setter" {
        crate::modules::dev::io_ensure_open(args)?;
    }
    if method == "mode_setter" {
        let value = args.get(1).cloned().unwrap_or_else(py_none);
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("_mode_override", value);
        }
        return Ok(py_none());
    }
    let raw = crate::modules::dev::io_get_raw(args)?;
    match method {
        "read" => {
            let n = args.get(1).cloned().unwrap_or_else(|| py_int(-1));
            let data = crate::object::call_method_rebound(vm, &raw, "read", vec![n])?;
            if let PyObject::Bytes(ref b) = &*data.borrow() {
                return Ok(py_str(&String::from_utf8_lossy(&b[..])));
            }
            Ok(data)
        }
        "readline" => {
            let n = args.get(1).cloned().unwrap_or_else(|| py_int(-1));
            let data = crate::object::call_method_rebound(vm, &raw, "readline", vec![n])?;
            if let PyObject::Bytes(ref b) = &*data.borrow() {
                return Ok(py_str(&String::from_utf8_lossy(&b[..])));
            }
            Ok(data)
        }
        "__next__" => {
            let data = crate::object::call_method_rebound(vm, &raw, "readline", vec![py_int(-1)])?;
            let s = if let PyObject::Bytes(ref b) = &*data.borrow() {
                py_str(&String::from_utf8_lossy(&b[..]))
            } else {
                data.clone()
            };
            let is_empty = match &*s.borrow() {
                PyObject::Str(st) => st.is_empty(),
                PyObject::Bytes(b) => b.is_empty(),
                _ => false,
            };
            if is_empty {
                Err(PyError::StopIteration)
            } else {
                Ok(s)
            }
        }
        "readlines" => {
            // Needed by linecache (traceback source lines) via
            // tokenize.open(); was missing -> AttributeError on every
            // frame, making traceback extraction ~300x slow. Loop over
            // readline until EOF.
            let mut lines = Vec::new();
            loop {
                let data =
                    crate::object::call_method_rebound(vm, &raw, "readline", vec![py_int(-1)])?;
                let s = if let PyObject::Bytes(ref b) = &*data.borrow() {
                    py_str(&String::from_utf8_lossy(&b[..]))
                } else {
                    data.clone()
                };
                let is_empty = match &*s.borrow() {
                    PyObject::Str(st) => st.is_empty(),
                    PyObject::Bytes(b) => b.is_empty(),
                    _ => false,
                };
                if is_empty {
                    break;
                }
                lines.push(s);
            }
            Ok(py_list(lines))
        }
        "write" => {
            let text = args
                .get(1)
                .cloned()
                .ok_or_else(|| PyError::type_error("write missing text"))?;
            let payload = match &*text.borrow() {
                PyObject::Str(s2) => PyObjectRef::imm(PyObject::Bytes(s2.as_bytes().to_vec())),
                _ => text.clone(),
            };
            let r = crate::object::call_method_rebound(vm, &raw, "write", vec![payload])?;
            if matches!(&*r.borrow(), PyObject::None) {
                let n = match &*text.borrow() {
                    PyObject::Str(s2) => s2.chars().count() as i64,
                    _ => 0,
                };
                Ok(py_int(n))
            } else {
                Ok(r)
            }
        }
        "seek" => {
            let pos = args.get(1).cloned().unwrap_or_else(|| py_int(0));
            let wh = args.get(2).cloned().unwrap_or_else(|| py_int(0));
            crate::object::call_method_rebound(vm, &raw, "seek", vec![pos, wh])
        }
        "tell" => crate::object::call_method_rebound(vm, &raw, "tell", vec![]),
        "flush" => crate::object::call_method_rebound(vm, &raw, "flush", vec![]),
        "close" => {
            let already = args[0]
                .borrow()
                .get_attribute("_closed")
                .ok()
                .map(|v| v.truthy())
                .unwrap_or(false);
            if !already {
                let _ = crate::object::call_method_rebound(vm, &raw, "close", vec![]);
                if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                    dict.insert_str("_closed", py_bool(true));
                }
            }
            Ok(py_none())
        }
        "truncate" => {
            let pos = args.get(1).cloned().unwrap_or_else(py_none);
            crate::object::call_method_rebound(vm, &raw, "truncate", vec![pos])
        }
        "fileno" => crate::object::call_method_rebound(vm, &raw, "fileno", vec![]),
        "isatty" => crate::object::call_method_rebound(vm, &raw, "isatty", vec![]),
        "writelines" => {
            if args.len() < 2 {
                return Err(PyError::type_error(
                    "writelines() takes exactly one argument",
                ));
            }
            let lines = &args[1];
            let iter = crate::object::builtin_iter(&[lines.clone()])?;
            loop {
                match crate::object::builtin_next(&[iter.clone()]) {
                    Ok(line) => {
                        let payload = match &*line.borrow() {
                            PyObject::Str(s2) => {
                                PyObjectRef::imm(PyObject::Bytes(s2.as_bytes().to_vec()))
                            }
                            _ => line.clone(),
                        };
                        let _ =
                            crate::object::call_method_rebound(vm, &raw, "write", vec![payload])?;
                    }
                    Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                    Err(e) => return Err(e),
                }
            }
            Ok(py_none())
        }
        "name" => crate::object::call_method_rebound(vm, &raw, "name", vec![]),
        "mode" => {
            // Real CPython's `_io.TextIOWrapper.mode` is a PLAIN settable
            // attribute, not a read-only property — `tokenize.open()`
            // (vendored verbatim from CPython) does `text.mode = 'r'`
            // right after construction (test_linecache.py's traceback
            // source-line lookups go through it, so every call failed
            // with AttributeError before this). Check for a
            // previously-stored override first; otherwise delegate to the
            // raw buffer's own mode as before.
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                if let Some(v) = dict.get_str("_mode_override") {
                    return Ok(v.clone());
                }
            }
            crate::object::call_method_rebound(vm, &raw, "mode", vec![])
        }
        _ => Err(PyError::runtime_error(
            "unknown TextIOWrapper delegate method",
        )),
    }
}

pub fn create_io_module_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("SEEK_SET", py_int(0));
    d.insert_str("SEEK_CUR", py_int(1));
    d.insert_str("SEEK_END", py_int(2));
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
            closed: std::rc::Rc::new(std::cell::RefCell::new(false)),
            encoding: None,
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
                // `args[0].borrow_mut()` panics outright ("borrow_mut called
                // on non-Mut value") when the argument isn't a heap-allocated
                // mutable object at all — e.g. `readinto('')` (real CPython
                // regression `test_memoryio.py`'s `test_readinto`, which
                // expects a plain `TypeError`, not a crash) passes an inline
                // `str`. `try_borrow_mut()` reports that case as an `Err`
                // instead of panicking, so it can be turned into the same
                // "argument must be read-write bytes-like object" `TypeError`
                // the non-ByteArray-but-still-Mut case already raises below.
                match args[0].try_borrow_mut() {
                    Ok(mut guard) => match &mut *guard {
                        PyObject::ByteArray(dest) => {
                            let n = dest.len().min(data.len() - pos);
                            dest[..n].copy_from_slice(&data[pos..pos + n]);
                            *p_readinto.borrow_mut() = pos + n;
                            Ok(py_int(n as i64))
                        }
                        _ => Err(PyError::type_error(
                            "argument must be read-write bytes-like object",
                        )),
                    },
                    Err(_) => Err(PyError::type_error(
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
                let size_limit = args
                    .get(0)
                    .and_then(|a| a.as_i64())
                    .filter(|&n| n >= 0)
                    .map(|n| n as usize);
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

        // getbuffer() — was entirely missing. Real `BytesIO.getbuffer()`
        // returns a writable `memoryview` over the WHOLE internal buffer
        // (not sliced at the current position, unlike `read()`). This is
        // not a cosmetic gap: `pickle.py`'s own `_Framer.commit_frame`
        // (used by EVERY `pickle.dumps()` call at protocol>=2, since
        // `end_framing()` unconditionally force-commits the final frame)
        // calls `self.current_frame.getbuffer()` and then both `len(data)`
        // and `write(data)` on the result — with `getbuffer` missing this
        // raised `AttributeError: 'BytesIO' object has no attribute
        // 'getbuffer'` on effectively every single non-trivial pickle,
        // general (not test-specific) and severe enough that it was
        // masquerading as multiple "hangs" in test_pickle.py's own
        // bisection (large fan-out loops over many objects/protocols
        // logged as slow/timing-out because each iteration's exception
        // handling and traceback formatting under `assertRaises`/subTest
        // scaffolding added up, not because of any actual infinite loop).
        // A plain `bytes` snapshot object satisfies both operations used
        // here (`len()`, and being handed to another `write()`); it isn't
        // a true live/writable view back into this buffer, but nothing in
        // this interpreter's own pickle usage mutates through it.
        let b_getbuffer = buf_rc.clone();
        type_dict.insert_str(
            "getbuffer",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(PyObjectRef::imm(PyObject::Bytes(
                    b_getbuffer.borrow().clone(),
                )))
            }))),
        );

        // peek — returns bytes without advancing position (for BytesIO, same as read)
        let b_peek = buf_rc.clone();
        let p_peek = pos_rc.clone();
        type_dict.insert_str(
            "peek",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                let data = b_peek.borrow();
                let pos = (*p_peek.borrow()).min(data.len());
                let end = if !args.is_empty() {
                    args[0]
                        .as_i64()
                        .map(|n| {
                            if n < 0 {
                                data.len()
                            } else {
                                (pos + n as usize).min(data.len())
                            }
                        })
                        .unwrap_or(data.len())
                } else {
                    data.len()
                };
                Ok(PyObjectRef::imm(PyObject::Bytes(data[pos..end].to_vec())))
            }))),
        );

        type_dict.insert_str(
            "flush",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(py_none())
            }))),
        );

        // fileno — BytesIO doesn't have a real fd, raise UnsupportedOperation
        type_dict.insert_str(
            "fileno",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Err(PyError::os_error_from_io(&std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "fileno",
                )))
            }))),
        );

        // isatty — BytesIO is never a TTY
        type_dict.insert_str(
            "isatty",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Ok(py_bool(false))
            }))),
        );

        type_dict.insert_str(
            "truncate",
            PyObjectRef::new(PyObject::Closure(Rc::new({
                let b = buf_rc.clone();
                let p = pos_rc.clone();
                move |args: &[PyObjectRef]| {
                    let size = args
                        .get(0)
                        .and_then(|a| a.as_i64())
                        .map(|n| n.max(0) as usize)
                        .unwrap_or(*p.borrow());
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

        Ok(PyObjectRef::new({
            let typ = PyObjectRef::new(PyObject::Type {
                name: "BytesIO".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            });
            fixup_self_mro(&typ);
            PyObject::Instance {
                typ,
                dict: AttrMap::new(),
            }
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
            closed: std::rc::Rc::new(std::cell::RefCell::new(false)),
            encoding: None,
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
            func: iobase_exit_marker,
        }),
    );
    let iobase_cls = PyObjectRef::new(PyObject::Type {
        name: "IOBase".to_string(),
        dict: Box::new(str_map_to_typedict(iobase_dict)),
        bases: vec![],
        mro: vec![],
    });
    fixup_self_mro(&iobase_cls);
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
    fixup_self_mro(&raw_cls);
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
    fixup_self_mro(&buf_cls);
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
    fixup_self_mro(&text_cls);
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
            // Cached "is this buffer pure ASCII" flag, recomputed only when
            // the buffer's CONTENTS actually change (`write`/`truncate`),
            // never on every read. Without this, `readline`/`__next__`
            // materialized the ENTIRE buffer into a fresh `Vec<char>` on
            // EVERY SINGLE CALL just to walk it one line at a time — for a
            // large buffer read line-by-line (`for line in io.StringIO(...)`,
            // exactly what `configparser.read_string()` does), that's
            // O(total length) work repeated once per line, i.e. O(n^2)
            // overall for n lines. Confirmed real: `test_configparser.py`'s
            // `test_combine_error_linear_complexity` (a 50000-line
            // `ConfigParser.read_string()`) went from "completes in ~35s
            // for the whole file" to a 60s timeout the moment an unrelated
            // fix (a spurious `PyDict` "changed size during iteration"
            // error, see `pydict.rs`) stopped masking this by making the
            // parse fail fast instead of actually running to completion.
            // For the overwhelmingly common ASCII case, byte offset ==
            // char offset, so `readline`/`__next__` can scan the
            // underlying `str`'s bytes directly from the current position
            // to the next `\n` — O(that line's length), not O(everything
            // read so far) — without materializing anything. Falls back to
            // the original full `Vec<char>` walk, unchanged, for any
            // buffer containing non-ASCII content.
            let is_ascii_cache = Rc::new(RefCell::new(initial_value.is_ascii()));
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
            let (b, p, ascii_flag) = (buffer.clone(), pos.clone(), is_ascii_cache.clone());
            type_dict.insert_str(
                "readline",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                    if *ascii_flag.borrow() {
                        let data = b.borrow();
                        let bytes = data.as_bytes();
                        let start = (*p.borrow()).min(bytes.len());
                        let limit = opt_size(args, 0)
                            .map(|n| (start + n as usize).min(bytes.len()))
                            .unwrap_or(bytes.len());
                        let mut end = start;
                        while end < limit {
                            if bytes[end] == b'\n' {
                                end += 1;
                                break;
                            }
                            end += 1;
                        }
                        *p.borrow_mut() = end;
                        // Safe: pure-ASCII buffer, so any byte range is a
                        // valid UTF-8 boundary.
                        let s = std::str::from_utf8(&bytes[start..end]).unwrap().to_string();
                        return Ok(py_str(&s));
                    }
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
            let (b, p, ascii_flag) = (buffer.clone(), pos.clone(), is_ascii_cache.clone());
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
                        let new_buf: String = chars.into_iter().collect();
                        *ascii_flag.borrow_mut() = new_buf.is_ascii();
                        *b.borrow_mut() = new_buf;
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
            let (b, p, ascii_flag) = (buffer.clone(), pos.clone(), is_ascii_cache.clone());
            type_dict.insert_str(
                "truncate",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                    let mut chars: Vec<char> = b.borrow().chars().collect();
                    let size = opt_size(args, 0)
                        .map(|n| n as usize)
                        .unwrap_or(*p.borrow())
                        .min(chars.len());
                    chars.truncate(size);
                    let new_buf: String = chars.into_iter().collect();
                    *ascii_flag.borrow_mut() = new_buf.is_ascii();
                    *b.borrow_mut() = new_buf;
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
                    func: |args: &[PyObjectRef]| {
                        if args.is_empty() {
                            return Err(PyError::type_error(
                                "__iter__() missing 1 required positional argument: 'self'",
                            ));
                        }
                        Ok(args[0].clone())
                    },
                }),
            );
            let (b, p, ascii_flag) = (buffer.clone(), pos.clone(), is_ascii_cache.clone());
            type_dict.insert_str(
                "__next__",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                    if *ascii_flag.borrow() {
                        let data = b.borrow();
                        let bytes = data.as_bytes();
                        let start = (*p.borrow()).min(bytes.len());
                        if start >= bytes.len() {
                            return Err(PyError::StopIteration);
                        }
                        let mut end = start;
                        while end < bytes.len() {
                            if bytes[end] == b'\n' {
                                end += 1;
                                break;
                            }
                            end += 1;
                        }
                        *p.borrow_mut() = end;
                        let s = std::str::from_utf8(&bytes[start..end]).unwrap().to_string();
                        return Ok(py_str(&s));
                    }
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
            // isatty — StringIO is never a TTY
            type_dict.insert_str(
                "isatty",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "isatty".to_string(),
                    func: |_: &[PyObjectRef]| Ok(py_bool(false)),
                }),
            );
            // fileno — StringIO has no fd
            type_dict.insert_str(
                "fileno",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "fileno".to_string(),
                    func: |_: &[PyObjectRef]| {
                        Err(PyError::os_error_from_io(&std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "fileno",
                        )))
                    },
                }),
            );

            Ok(PyObjectRef::new({
                let typ = PyObjectRef::new(PyObject::Type {
                    name: "StringIO".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![text_cls.clone()],
                    mro: vec![text_cls.clone()],
                });
                fixup_self_mro(&typ);
                PyObject::Instance {
                    typ,
                    dict: AttrMap::new(),
                }
            }))
        });
    d.insert_str(
        "StringIO",
        PyObjectRef::new(PyObject::Closure(stringio_closure)),
    );

    register_buffered_classes(&mut d, &buf_cls);

    // TextIOWrapper shares the delegation behavior, adding text-mode bits.
    let tiw_inner = {
        let mut td: HashMap<String, PyObjectRef> = HashMap::new();
        let bf = |name: &'static str, f: crate::object::BuiltinFunc| {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: name.to_string(),
                func: f,
            })
        };
        td.insert(
            "__init__".into(),
            bf("__init__", |args| {
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
            }),
        );
        // read/readline/readlines/write/seek/tell/flush/close/truncate/
        // fileno/isatty/writelines all need to delegate to the raw object's
        // OWN (possibly Python-subclass-overridden) method, which requires a
        // live `&mut VirtualMachine` (`call_method_rebound`). These are
        // reachable directly from live bytecode (`f.read()`, `for line in
        // f`, ...), so grabbing a SECOND `&mut VirtualMachine` here via
        // `with_vm_mut`'s raw-pointer dereference while the interpreter's
        // own `&mut vm` is already live on the call stack is exactly the
        // unsound-aliasing pattern documented in CLAUDE.md's "Gotchas"
        // section (and already fixed once for `IOBase.__exit__` — see
        // `iobase_exit_marker`'s doc comment for the same story). It doesn't
        // just risk crashing: under `release`/`release-lite` optimizations
        // the compiler may assume the two `&mut` never alias and reorder or
        // miscompile around that assumption, which is what actually turned
        // into a real, if rare and non-deterministic, bug — a busy-spinning
        // infinite loop (not a blocked wait: confirmed via /proc's
        // per-thread utime climbing steadily, not stuck in a syscall) that
        // reproduced ~1-in-8 runs of `tests/cpython/test_mailbox.py`'s full
        // real test order (never in isolation), at a DIFFERENT test each
        // time (`TestMMDF.test_get_string_from`, `TestMH.test_set_item`,
        // ...) — the signature of true undefined behavior, not a
        // deterministic algorithmic bug. Fixed the same way as
        // `iobase_exit_marker`/`zipimporter_method_marker`: one shared
        // marker function dispatched by name from `src/vm/call.rs`, which
        // already holds the genuinely live `&mut VirtualMachine` and can
        // pass it straight through — no second alias ever created.
        for name in [
            "read",
            "readline",
            "readlines",
            "write",
            "seek",
            "tell",
            "flush",
            "close",
            "truncate",
            "fileno",
            "isatty",
            "writelines",
        ] {
            td.insert(name.into(), bf(name, tiw_delegate_marker));
        }
        // `__iter__`/`__next__` — `TextIOWrapper` (and thus any file opened
        // in text mode) must support `for line in fp:`. Missing entirely
        // before (`TypeError: 'TextIOWrapper' object is not iterable`),
        // breaking `linecache`/`tokenize`-based source retrieval
        // (test_linecache.py's `test_getline` iterates a real file handle
        // directly). `__next__` delegates to the same `readline` path as
        // the explicit method, raising `StopIteration` on an empty read
        // (EOF) exactly like real Python's file iterator protocol.
        td.insert(
            "__iter__".into(),
            bf("__iter__", |args: &[PyObjectRef]| {
                args.first().cloned().ok_or_else(|| {
                    PyError::type_error("__iter__() missing 1 required positional argument: 'self'")
                })
            }),
        );
        // `__next__` deliberately does NOT go through `tiw_delegate_marker`
        // (unlike `read`/`readline`/...): those all reach `call.rs`'s
        // interception through a live bytecode `CALL`, which holds the real
        // `&mut VirtualMachine` to pass through. `__next__` is different —
        // it is ALSO invoked from plain `fn(&[PyObjectRef])` builtins with
        // no VM at all (`builtin_next`'s `Instance` branch calls the raw
        // dunder function pointer directly; `enumerate()`/`zip()`/`list()`
        // etc. all route through that same vm-less `builtin_next`). Reusing
        // the marker there would hit "delegate marker reached directly"
        // (test_linecache.py's `for line in fp:` inside `enumerate(fp)`).
        // So this calls the raw buffer's `readline` via the plain,
        // vm-free `builtin_call` instead of `call_method_rebound` — this
        // loses `call_method_rebound`'s "resolve a Python-subclass's
        // overridden readline" support, but the raw object here is always
        // this module's own native `FileIO`/`BufferedReader`/`BytesIO`
        // (never user Python subclasses in the tested paths), whose
        // `readline` is itself a plain `Closure`/`BuiltinMethod` needing no
        // VM either.
        td.insert(
            "__next__".into(),
            bf("__next__", |args: &[PyObjectRef]| {
                crate::modules::dev::io_ensure_open(args)?;
                let raw = crate::modules::dev::io_get_raw(args)?;
                let readline = raw.borrow().get_attribute("readline")?;
                let bound = match &*readline.borrow() {
                    PyObject::BuiltinMethod {
                        func,
                        name,
                        self_obj,
                    } if matches!(&*self_obj.borrow(), PyObject::None) => {
                        PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: name.clone(),
                            func: *func,
                            self_obj: raw.clone(),
                        })
                    }
                    _ => readline.clone(),
                };
                let data = crate::object::builtin_call(&bound, &[py_int(-1)])?;
                let s = if let PyObject::Bytes(ref b) = &*data.borrow() {
                    py_str(&String::from_utf8_lossy(&b[..]))
                } else {
                    data.clone()
                };
                let is_empty = match &*s.borrow() {
                    PyObject::Str(st) => st.is_empty(),
                    PyObject::Bytes(b) => b.is_empty(),
                    _ => false,
                };
                if is_empty {
                    Err(PyError::StopIteration)
                } else {
                    Ok(s)
                }
            }),
        );
        td.insert("reconfigure".into(), bf("reconfigure", |_a| Ok(py_none())));
        td.insert("readable".into(), bf("readable", |_a| Ok(py_bool(true))));
        td.insert("writable".into(), bf("writable", |_a| Ok(py_bool(true))));
        td.insert("seekable".into(), bf("seekable", |_a| Ok(py_bool(true))));
        td.insert(
            "detach".into(),
            bf("detach", |args: &[PyObjectRef]| {
                crate::modules::dev::io_ensure_open(args)?;
                let raw = crate::modules::dev::io_get_raw(args)?;
                if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                    dict.insert_str("_closed", py_bool(true));
                }
                Ok(raw)
            }),
        );
        td.insert(
            "buffer".into(),
            PyObjectRef::new(PyObject::Property(Box::new(crate::object::PropertyData {
                getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "buffer".to_string(),
                    func: |args: &[PyObjectRef]| Ok(crate::modules::dev::io_get_raw(args)?),
                })),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
        // closed — property
        td.insert(
            "closed".into(),
            PyObjectRef::new(PyObject::Property(Box::new(crate::object::PropertyData {
                getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "closed".to_string(),
                    func: |args: &[PyObjectRef]| {
                        let closed = args[0]
                            .borrow()
                            .get_attribute("_closed")
                            .ok()
                            .map(|v| v.truthy())
                            .unwrap_or(false);
                        Ok(py_bool(closed))
                    },
                })),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
        // name — property (see the big comment above `read`/`readline`/... for
        // why this can't just call `with_vm_mut` directly)
        td.insert(
            "name".into(),
            PyObjectRef::new(PyObject::Property(Box::new(crate::object::PropertyData {
                getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "name".to_string(),
                    func: tiw_delegate_marker,
                })),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
        // mode — property. Real CPython's `TextIOWrapper.mode` is a plain
        // settable attribute (see `tiw_delegate_with_vm`'s "mode"/
        // "mode_setter" branches for why this needs a real setter, not
        // `None` — found via `tokenize.open()`'s `text.mode = 'r'`).
        td.insert(
            "mode".into(),
            PyObjectRef::new(PyObject::Property(Box::new(crate::object::PropertyData {
                getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "mode".to_string(),
                    func: tiw_delegate_marker,
                })),
                setter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "mode_setter".to_string(),
                    func: tiw_delegate_marker,
                })),
                deleter: None,
                doc: None,
            }))),
        );
        // errors — property
        td.insert(
            "errors".into(),
            PyObjectRef::new(PyObject::Property(Box::new(crate::object::PropertyData {
                getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "errors".to_string(),
                    func: |_args: &[PyObjectRef]| Ok(py_none()),
                })),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
        td.insert(
            "__enter__".into(),
            bf("__enter__", |args: &[PyObjectRef]| Ok(args[0].clone())),
        );
        td.insert("__exit__".into(), bf("__exit__", |_a| Ok(py_bool(false))));
        td
    };
    let tiw_cls = PyObjectRef::new(PyObject::Type {
        name: "TextIOWrapper".to_string(),
        dict: Box::new(str_map_to_typedict(tiw_inner)),
        bases: vec![text_cls_tiw.clone()],
        mro: vec![text_cls_tiw.clone()],
    });
    fixup_self_mro(&tiw_cls);
    d.insert_str("TextIOWrapper", tiw_cls);

    d.insert_str("_WindowsConsoleIO", py_str("_WindowsConsoleIO"));

    d
}
