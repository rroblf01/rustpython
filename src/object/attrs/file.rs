// Auto-extracted from src/object/attrs/mod.rs lines 4498-4985
use super::*;
use crate::interner;
use crate::object::*;
use std::rc::Rc;

/// A `UnicodeDecodeError` for `buf`, which `encoding` could not decode.
pub fn unicode_decode_error(encoding: &str, buf: &[u8]) -> PyError {
    let (valid_up_to, start, reason) = match std::str::from_utf8(buf) {
        Err(e) => {
            let valid_up_to = e.valid_up_to();
            match e.error_len() {
                Some(len) => (valid_up_to, valid_up_to + len + 1, "invalid start byte"),
                None => (valid_up_to, valid_up_to + 1, "unexpected end of data"),
            }
        }
        Ok(_) => (0, 1, "invalid start byte"),
    };
    PyError::Exception(
        "UnicodeDecodeError".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "UnicodeDecodeError".to_string(),
            args: vec![
                py_str(encoding),
                PyObjectRef::new(PyObject::Bytes(buf.to_vec())),
                py_int(valid_up_to as i64),
                py_int(start as i64),
                py_str(reason),
            ],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

/// Decode bytes to a Python string, respecting the file's encoding.
/// Returns UnicodeDecodeError for non-UTF-8 encodings when bytes are invalid.
pub fn decode_with_encoding(buf: &[u8], encoding: &Option<String>) -> PyResult<PyObjectRef> {
    let enc = encoding.as_deref().unwrap_or("utf-8");
    if enc == "ascii" || enc == "us-ascii" {
        for &b in buf {
            if b >= 0x80 {
                return Err(PyError::Exception(
                    "UnicodeDecodeError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "UnicodeDecodeError".to_string(),
                        args: vec![
                            py_str("ascii"),
                            py_str(&String::from_utf8_lossy(buf)),
                            py_int(b as i64),
                            py_int(1),
                            py_str("ordinal not in range(128)"),
                        ],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    }),
                ));
            }
        }
        Ok(py_str(&String::from_utf8_lossy(buf)))
    } else {
        // UTF-8 (the default) is decoded STRICTLY, like CPython's text mode:
        // an undecodable byte must raise `UnicodeDecodeError`, not come back
        // as U+FFFD. Code depends on that — `tokenize.open` is how CPython
        // reads source honouring PEP 263, and a file whose declared encoding
        // does not fit its bytes has to FAIL there, which is what makes
        // `linecache.getline` return '' for a badly-encoded file (and what
        // makes `compile()` reject it) instead of silently reinterpreting it.
        if enc.eq_ignore_ascii_case("utf-8") || enc.eq_ignore_ascii_case("utf8") {
            return match std::str::from_utf8(buf) {
                Ok(text) => Ok(py_str(text)),
                Err(e) => {
                    let valid_up_to = e.valid_up_to();
                    let start = e.error_len().map(|l| l + valid_up_to).unwrap_or(0) + 1;
                    Err(PyError::Exception(
                        "UnicodeDecodeError".to_string(),
                        PyObjectRef::new(PyObject::Exception {
                            typ: "UnicodeDecodeError".to_string(),
                            args: vec![
                                py_str("utf-8"),
                                PyObjectRef::new(PyObject::Bytes(buf.to_vec())),
                                py_int(valid_up_to as i64),
                                py_int(start as i64),
                                py_str("invalid start byte"),
                            ],
                            cause: None,
                            suppress_context: false,
                            context: None,
                            traceback: None,
                            extra: None,
                        }),
                    ))
                }
            };
        }
        // Any other codec we do not implement: keep the historical lossy
        // behaviour rather than refusing to read the file at all.
        Ok(py_str(&String::from_utf8_lossy(buf)))
    }
}

/// Replace the `std::fs::File` living inside a shared `Rc<RefCell<File>>`
/// pipe/file handle with a fresh `/dev/null` placeholder, closing the real
/// OS fd exactly once — WITHOUT ever letting `std::fs::File`'s own `Drop`
/// run on the old value.
///
/// Why this exists: real CPython tolerates closing an fd that's already
/// been closed by other means (e.g. `os.close(f.fileno())` followed by
/// `f.close()` — `tests/cpython/test_subprocess.py`'s own
/// `_test_bufsize_equal_one` does exactly this, wrapping the second close
/// in `except OSError: pass`) — the raw `close(2)` syscall just returns
/// `EBADF`, which CPython surfaces as a normal, catchable `OSError`. This
/// Rust toolchain's `std::fs::File::drop` carries a `debug_assert_fd_is_open`
/// check that instead calls `rtabort!` — a hard process abort, not a panic,
/// not something any Python-level `try`/`except` can catch — the instant it
/// observes an owned fd that's already invalid. Plain
/// `std::mem::replace(&mut *file.borrow_mut(), new_file)` (the previous
/// pattern used at every close site) drops the OLD `File` normally, running
/// straight into that abort whenever something else already closed the same
/// fd out from under it (confirmed real crash: `test_subprocess.py`'s
/// `_test_bufsize_equal_one`, "IO Safety violation: owned file descriptor
/// already closed").
///
/// Fix: extract the raw fd ourselves, `ManuallyDrop` the old `File` so
/// `std::fs::File::drop` (and its debug assertion) never runs on it, and
/// close the fd via a direct libc `close(2)` call whose result is ignored —
/// matching CPython's tolerant "double close is a silent/catchable no-op"
/// semantics instead of aborting the whole interpreter. No new dependency:
/// declared directly, like the existing `raise`/`signal` FFI calls in
/// `src/main.rs`.
pub(crate) fn close_owned_file(file: &Rc<std::cell::RefCell<std::fs::File>>) {
    use std::os::unix::io::AsRawFd;
    let devnull = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
        .or_else(|_| std::fs::File::open("/dev/null"));
    if let Ok(new_file) = devnull {
        let old = std::mem::replace(&mut *file.borrow_mut(), new_file);
        let fd = old.as_raw_fd();
        // SAFETY: `old` is never used again after this — wrapping it in
        // `ManuallyDrop` prevents `std::fs::File::drop`'s own close (and
        // its abort-on-already-closed debug check) from running; we do the
        // actual close ourselves via a raw libc call and ignore its result,
        // matching CPython's tolerant EBADF-on-double-close behavior.
        let _old = std::mem::ManuallyDrop::new(old);
        extern "C" {
            fn close(fd: i32) -> i32;
        }
        unsafe {
            close(fd);
        }
    }
}

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
        PyObject::File { file: f_rc, .. } => {
            match name {
                // `file.raw` — the unbuffered `FileIO` under this stream.
                // Real `tempfile` walks `file.buffer.raw` to reach the
                // object whose writable `name` it then sets; with a single
                // native file object here, `raw` is the file itself.
                "raw" => return Ok(PyObjectRef::new(o.clone())),
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
                            closed: std::rc::Rc::new(std::cell::RefCell::new(false)),
                            encoding: None,
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
                        Ok(py_bool(*closed.borrow()))
                    } else {
                        Err(PyError::runtime_error("closed access on non-file"))
                    }
                }
                // `sys.stdin.encoding/errors` (and stdout/stderr) —
                // needed by locale/coercion subprocess checks
                // (test_c_locale_coercion) which print them. Honors
                // PYTHONIOENCODING ("enc[:errors]"); otherwise
                // CPython's C.UTF-8 stream defaults.
                "encoding" => {
                    if matches!(o, PyObject::File { .. }) {
                        if let Ok(ioenc) = std::env::var("PYTHONIOENCODING") {
                            if !ioenc.is_empty() {
                                let enc = ioenc.split(':').next().unwrap_or("utf-8");
                                if !enc.is_empty() {
                                    return Ok(py_str(&enc.to_lowercase()));
                                }
                            }
                        }
                        Ok(py_str("utf-8"))
                    } else {
                        Err(PyError::runtime_error("encoding access on non-file"))
                    }
                }
                "errors" => {
                    if let PyObject::File { name: fname, .. } = o {
                        // Explicit non-empty PYTHONIOENCODING: with an
                        // :errors part it wins everywhere; without one,
                        // stderr keeps backslashreplace (CPython always
                        // protects stderr) while others go strict; empty
                        // counts as unset.
                        if let Ok(ioenc) = std::env::var("PYTHONIOENCODING") {
                            if !ioenc.is_empty() {
                                let mut parts = ioenc.splitn(2, ':');
                                let _enc = parts.next();
                                if let Some(err) = parts.next() {
                                    if !err.is_empty() {
                                        return Ok(py_str(err));
                                    }
                                }
                                if fname == "<stderr>" {
                                    return Ok(py_str("backslashreplace"));
                                }
                                return Ok(py_str("strict"));
                            }
                        }
                        // Match CPython's C.UTF-8 stream defaults
                        // (test_c_locale_coercion checks these exactly).
                        let e = if fname == "<stderr>" {
                            "backslashreplace"
                        } else if fname == "<stdin>" || fname == "<stdout>" {
                            "surrogateescape"
                        } else {
                            "strict"
                        };
                        Ok(py_str(e))
                    } else {
                        Err(PyError::runtime_error("errors access on non-file"))
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
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "read" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "read".to_string(),
                    func: |args| {
                        use std::io::Read;
                        if let PyObject::File {
                            file,
                            binary,
                            pending,
                            encoding,
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
                                let mut full: Vec<u8> = std::mem::take(&mut *pending.borrow_mut());
                                full.extend_from_slice(&buf);
                                let enc = encoding.as_deref().unwrap_or("utf-8");
                                if enc == "ascii" || enc == "us-ascii" {
                                    return decode_with_encoding(&full, encoding);
                                }
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
                                                    // EOF: decode what we
                                                    // have rather than hang
                                                    // forever (strict for
                                                    // UTF-8, an incomplete
                                                    // tail is a decode error
                                                    // just like any other).
                                                    return decode_with_encoding(&full, encoding);
                                                }
                                                Ok(_) => full.push(extra[0]),
                                                Err(e) => {
                                                    return Err(PyError::os_error_from_io(&e))
                                                }
                                            }
                                        }
                                        Err(_) => {
                                            // Genuinely invalid bytes, or
                                            // an incomplete tail at EOF.
                                            // STRICT for UTF-8, matching
                                            // CPython's text mode: silently
                                            // substituting U+FFFD here made
                                            // `tokenize.open` (the PEP 263
                                            // reader) accept a file whose
                                            // declared encoding does not
                                            // fit its bytes, and
                                            // `linecache`/`compile()` then
                                            // read mis-decoded source
                                            // instead of failing.
                                            return decode_with_encoding(&full, encoding);
                                        }
                                    }
                                }
                            }
                        } else {
                            Err(PyError::runtime_error("read on non-file"))
                        }
                    },
                    self_obj: PyObjectRef::new(o.clone()),
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
                        if let PyObject::File {
                            file,
                            binary,
                            encoding,
                            ..
                        } = &*args[0].borrow()
                        {
                            let limit = args
                                .get(1)
                                .and_then(|a| a.as_i64())
                                .filter(|&n| n >= 0)
                                .map(|n| n as usize);
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
                                decode_with_encoding(&buf, encoding)
                            }
                        } else {
                            Err(PyError::runtime_error("readline on non-file"))
                        }
                    },
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "readlines" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "readlines".to_string(),
                    func: |args| {
                        use std::io::Read;
                        if let PyObject::File {
                            file,
                            binary,
                            encoding,
                            ..
                        } = &*args[0].borrow()
                        {
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
                                        decode_with_encoding(&current, encoding)?
                                    });
                                    current.clear();
                                }
                            }
                            if !current.is_empty() {
                                lines.push(if *binary {
                                    PyObjectRef::imm(PyObject::Bytes(current.clone()))
                                } else {
                                    decode_with_encoding(&current, encoding)?
                                });
                            }
                            Ok(py_list(lines))
                        } else {
                            Err(PyError::runtime_error("readlines on non-file"))
                        }
                    },
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__iter__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "__next__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__next__".to_string(),
                    func: |args| {
                        use std::io::Read;
                        if let PyObject::File {
                            file,
                            binary,
                            encoding,
                            ..
                        } = &*args[0].borrow()
                        {
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
                                decode_with_encoding(&buf, encoding)
                            }
                        } else {
                            Err(PyError::runtime_error("__next__ on non-file"))
                        }
                    },
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "write" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "write".to_string(),
                    func: |args| {
                        use std::io::Write;
                        if args.len() < 2 {
                            return Err(PyError::type_error("write() takes exactly one argument"));
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
                    self_obj: PyObjectRef::new(o.clone()),
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
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "close".to_string(),
                    func: |args| {
                        if let PyObject::File { file, closed, .. } = &mut *args[0].borrow_mut() {
                            *closed.borrow_mut() = true;
                            close_owned_file(file);
                            Ok(py_none())
                        } else {
                            Err(PyError::runtime_error("close on non-file"))
                        }
                    },
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__enter__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(o.clone()),
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
                            *closed.borrow_mut() = true;
                            close_owned_file(file);
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "seek" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "seek".to_string(),
                    func: |args| {
                        use std::io::SeekFrom;
                        if args.len() < 2 {
                            return Err(PyError::type_error("seek() requires at least 1 argument"));
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
                    self_obj: PyObjectRef::new(o.clone()),
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
                    self_obj: PyObjectRef::new(o.clone()),
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
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "readable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "readable".to_string(),
                    func: |_| Ok(py_bool(true)),
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "writable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "writable".to_string(),
                    func: |_| Ok(py_bool(true)),
                    self_obj: PyObjectRef::new(o.clone()),
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
                            f.set_len(target)
                                .map_err(|e| PyError::os_error_from_io(&e))?;
                            if pos > target {
                                f.seek(std::io::SeekFrom::Start(target)).ok();
                            }
                            Ok(py_int(target as i64))
                        } else {
                            Err(PyError::runtime_error("truncate on non-file"))
                        }
                    },
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                "seekable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "seekable".to_string(),
                    func: |_| Ok(py_bool(true)),
                    self_obj: PyObjectRef::new(o.clone()),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'file' object has no attribute '{}'",
                    name
                ))),
            }
        }
        _ => unreachable!(
            "{} called with non-{} object",
            stringify!(get),
            o.type_name()
        ),
    }
}
