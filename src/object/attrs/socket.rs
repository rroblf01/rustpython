// Auto-extracted from src/object/attrs/mod.rs lines 5006-5418
use super::*;
use crate::interner;
use crate::object::*;
use std::rc::Rc;

// --- shared helpers for the byte-exact socket I/O methods -----------------

/// Every bytestring-ish argument CPython's socket methods accept (`bytes`,
/// `bytearray`, `array`, any memoryview) plus `str`, which it encodes as
/// UTF-8 (CPython sends `str` through the socket's own encoding, latin-1,
/// but UTF-8 keeps non-ASCII payloads from raising).
fn sock_arg_bytes(v: &PyObjectRef) -> PyResult<Vec<u8>> {
    if let Some(b) = arg_bytes(v) {
        return Ok(b);
    }
    match &*v.borrow() {
        PyObject::Str(s) => Ok(s.as_bytes().to_vec()),
        _ => Err(PyError::type_error(
            "a bytes-like object is required, not 'str'",
        )),
    }
}

/// The timeout `settimeout()` last recorded for this socket's fd. Kept
/// out of `PyObject::Socket` (24 construction sites) and keyed by raw fd,
/// which is exactly what a real `SO_RCVTIMEO` is.
fn sock_timeout_for(socket: &PyObjectRef) -> Option<f64> {
    match sock_mode_for(socket) {
        SockMode::Deadline(seconds) => Some(seconds),
        _ => None,
    }
}

fn set_sock_mode(socket: &PyObjectRef, mode: SockMode) {
    use std::os::fd::AsRawFd;
    let fd = {
        let borrowed = socket.borrow();
        let inner = match &*borrowed {
            PyObject::Socket { inner, .. } => inner,
            _ => return,
        };
        let guard = inner.borrow();
        match &*guard {
            SocketInner::TcpStream(s) => {
                match mode {
                    SockMode::Blocking => {
                        let _ = s.set_read_timeout(None);
                        let _ = s.set_write_timeout(None);
                        let _ = s.set_nonblocking(false);
                    }
                    SockMode::NonBlocking => {
                        let _ = s.set_read_timeout(None);
                        let _ = s.set_write_timeout(None);
                        let _ = s.set_nonblocking(true);
                    }
                    SockMode::Deadline(seconds) => {
                        let duration = if seconds <= 0.0 {
                            std::time::Duration::from_nanos(1)
                        } else {
                            std::time::Duration::from_secs_f64(seconds)
                        };
                        let _ = s.set_read_timeout(Some(duration));
                        let _ = s.set_write_timeout(Some(duration));
                        let _ = s.set_nonblocking(false);
                    }
                }
                s.as_raw_fd()
            }
            SocketInner::UnixStream(s) => {
                match mode {
                    SockMode::Blocking => {
                        let _ = s.set_read_timeout(None);
                        let _ = s.set_write_timeout(None);
                        let _ = s.set_nonblocking(false);
                    }
                    SockMode::NonBlocking => {
                        let _ = s.set_nonblocking(true);
                    }
                    SockMode::Deadline(seconds) => {
                        let duration = if seconds <= 0.0 {
                            std::time::Duration::from_nanos(1)
                        } else {
                            std::time::Duration::from_secs_f64(seconds)
                        };
                        let _ = s.set_read_timeout(Some(duration));
                        let _ = s.set_write_timeout(Some(duration));
                        let _ = s.set_nonblocking(false);
                    }
                }
                s.as_raw_fd()
            }
            SocketInner::TcpListener(l) => l.as_raw_fd(),
            SocketInner::UnixListener(l) => l.as_raw_fd(),
            SocketInner::Uninitialized => return,
        }
    };
    SOCKET_MODES.with(|m| m.borrow_mut().insert(fd, mode));
}

/// CPython's socket modes: blocking (the default), explicitly
/// non-blocking (`setblocking(False)`, as `selectors`/`asyncore` want), or
/// deadline-bounded (`settimeout(x)`). Every native socket here is created
/// non-blocking at the OS level so `select`/`poll` can drive it, so this
/// table is what decides whether a `WouldBlock` retry loops, raises
/// `BlockingIOError`, or becomes `socket.timeout`.
#[derive(Clone, Copy, PartialEq)]
enum SockMode {
    Blocking,
    NonBlocking,
    Deadline(f64),
}

thread_local! {
    static SOCKET_MODES: std::cell::RefCell<std::collections::HashMap<i32, SockMode>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn sock_mode_for(socket: &PyObjectRef) -> SockMode {
    use std::os::fd::AsRawFd;
    let fd = {
        let borrowed = socket.borrow();
        let inner = match &*borrowed {
            PyObject::Socket { inner, .. } => inner,
            _ => return SockMode::Blocking,
        };
        let guard = inner.borrow();
        match &*guard {
            SocketInner::TcpStream(s) => s.as_raw_fd(),
            SocketInner::TcpListener(l) => l.as_raw_fd(),
            SocketInner::UnixStream(s) => s.as_raw_fd(),
            SocketInner::UnixListener(l) => l.as_raw_fd(),
            SocketInner::Uninitialized => return SockMode::Blocking,
        }
    };
    SOCKET_MODES
        .with(|m| m.borrow().get(&fd).copied())
        .unwrap_or(SockMode::Blocking)
}

/// AF_UNIX addresses arrive as a plain path string (or bytes).
fn unix_path_arg(arg: &PyObjectRef) -> PyResult<std::path::PathBuf> {
    match &*arg.borrow() {
        PyObject::Bytes(b) => Ok(std::path::PathBuf::from(
            String::from_utf8_lossy(b).to_string(),
        )),
        PyObject::ByteArray(b) => Ok(std::path::PathBuf::from(
            String::from_utf8_lossy(b).to_string(),
        )),
        _ => Ok(std::path::PathBuf::from(arg.str())),
    }
}

/// An AF_UNIX socket address is a filesystem path; the empty string is what
/// an unbound/unnamed socket reports, exactly as CPython does.
fn path_or_empty(addr: &std::os::unix::net::SocketAddr) -> PyObjectRef {
    match addr.as_pathname() {
        Some(path) => py_str(&path.to_string_lossy()),
        None => py_str(""),
    }
}

/// `socket.timeout` is an alias of the builtin `TimeoutError` since Python
/// 3.10, which this codebase models as an `OSError` carrying errno
/// ETIMEDOUT (110) so `except socket.timeout` catches it.
fn timeout_error(msg: &str) -> PyError {
    let _ = msg;
    PyError::os_error_from_io(&std::io::Error::from_raw_os_error(110))
}

/// Raw byte read used by `recv`/`recv_into` and by the socket file object.
/// The mode decides what a `WouldBlock` means: a blocking socket retries,
/// an explicitly non-blocking one raises `BlockingIOError`, and a deadline
/// becomes `socket.timeout`. (The retry loop lives OUTSIDE the `SocketInner`
/// borrow — a blocking retry has to release it between attempts, and the
/// borrow guard cannot be reassigned while the `match` arm still holds it.)
fn sock_recv_bytes(socket: &PyObjectRef, bufsize: usize) -> PyResult<Vec<u8>> {
    use std::io::Read;
    if bufsize == 0 {
        return Ok(Vec::new());
    }
    let mode = sock_mode_for(socket);
    loop {
        let outcome = {
            let borrowed = socket.borrow();
            let inner = match &*borrowed {
                PyObject::Socket { inner, .. } => inner,
                _ => return Err(PyError::runtime_error("recv on non-socket")),
            };
            let mut guard = inner.borrow_mut();
            match &mut *guard {
                SocketInner::TcpStream(stream) => {
                    // A blocking socket must not park inside `read()`: this
                    // interpreter's threads are cooperative, so a parked
                    // reader starves the very thread that would send it
                    // data. A short OS-level deadline makes the wait
                    // interruptible so the scheduler can run.
                    if mode == SockMode::Blocking {
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(20)));
                    }
                    let mut buf = vec![0u8; bufsize];
                    loop {
                        match stream.read(&mut buf) {
                            Ok(n) => {
                                if mode == SockMode::Blocking {
                                    let _ = stream.set_read_timeout(None);
                                }
                                buf.truncate(n);
                                break Ok(buf);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    || e.kind() == std::io::ErrorKind::TimedOut =>
                            {
                                break Err(std::io::ErrorKind::WouldBlock)
                            }
                            Err(e) => break Err(e.kind()),
                        }
                    }
                }
                SocketInner::UnixStream(stream) => {
                    if mode == SockMode::Blocking {
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(20)));
                    }
                    let mut buf = vec![0u8; bufsize];
                    loop {
                        match stream.read(&mut buf) {
                            Ok(n) => {
                                if mode == SockMode::Blocking {
                                    let _ = stream.set_read_timeout(None);
                                }
                                buf.truncate(n);
                                break Ok(buf);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    || e.kind() == std::io::ErrorKind::TimedOut =>
                            {
                                break Err(std::io::ErrorKind::WouldBlock)
                            }
                            Err(e) => break Err(e.kind()),
                        }
                    }
                }
                _ => return Err(PyError::runtime_error("recv on non-stream")),
            }
        };
        match outcome {
            Ok(data) => return Ok(data),
            Err(std::io::ErrorKind::WouldBlock) => match mode {
                SockMode::Deadline(_) => return Err(timeout_error("The read operation timed out")),
                SockMode::NonBlocking => {
                    return Err(PyError::os_error_from_io(
                        &std::io::Error::from_raw_os_error(11),
                    ))
                }
                SockMode::Blocking => {
                    crate::modules::coop_threads_drain();
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
            },
            Err(kind) => return Err(PyError::os_error_from_io(&std::io::Error::from(kind))),
        }
    }
}

/// Raw byte write returning the count actually written (what `send`
/// reports), plus the all-or-error variant `sendall` wants.
fn sock_write_bytes(socket: &PyObjectRef, data: &[u8], all: bool) -> PyResult<i64> {
    use std::io::Write;
    if data.is_empty() {
        return Ok(0);
    }
    let mode = sock_mode_for(socket);
    loop {
        let outcome = {
            let borrowed = socket.borrow();
            let inner = match &*borrowed {
                PyObject::Socket { inner, .. } => inner,
                _ => return Err(PyError::runtime_error("send on non-socket")),
            };
            let mut guard = inner.borrow_mut();
            match &mut *guard {
                SocketInner::TcpStream(stream) => {
                    if mode == SockMode::Blocking {
                        let _ =
                            stream.set_write_timeout(Some(std::time::Duration::from_millis(20)));
                    }
                    if all {
                        match stream.write_all(data) {
                            Ok(()) => {
                                if mode == SockMode::Blocking {
                                    let _ = stream.set_write_timeout(None);
                                }
                                Ok(data.len() as i64)
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                                Err(std::io::ErrorKind::Interrupted)
                            }
                            Err(e) => Err(e.kind()),
                        }
                    } else {
                        match stream.write(data) {
                            Ok(n) => {
                                if mode == SockMode::Blocking {
                                    let _ = stream.set_write_timeout(None);
                                }
                                Ok(n as i64)
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                                Err(std::io::ErrorKind::Interrupted)
                            }
                            Err(e) => Err(e.kind()),
                        }
                    }
                }
                SocketInner::UnixStream(stream) => {
                    if mode == SockMode::Blocking {
                        let _ =
                            stream.set_write_timeout(Some(std::time::Duration::from_millis(20)));
                    }
                    loop {
                        let result = if all {
                            stream.write_all(data).map(|_| data.len() as i64)
                        } else {
                            stream.write(data).map(|n| n as i64)
                        };
                        match result {
                            Ok(n) => {
                                if mode == SockMode::Blocking {
                                    let _ = stream.set_write_timeout(None);
                                }
                                break Ok(n);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    || e.kind() == std::io::ErrorKind::TimedOut =>
                            {
                                break Err(std::io::ErrorKind::WouldBlock)
                            }
                            Err(e) => break Err(e.kind()),
                        }
                    }
                }
                _ => return Err(PyError::runtime_error("send on non-stream")),
            }
        };
        match outcome {
            Ok(n) => return Ok(n),
            Err(std::io::ErrorKind::Interrupted) => continue,
            Err(std::io::ErrorKind::WouldBlock) => match mode {
                SockMode::Deadline(_) => {
                    return Err(timeout_error("The write operation timed out"))
                }
                SockMode::NonBlocking => {
                    return Err(PyError::os_error_from_io(
                        &std::io::Error::from_raw_os_error(11),
                    ))
                }
                SockMode::Blocking => {
                    // Same cooperative yield as the read path.
                    crate::modules::coop_threads_drain();
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
            },
            Err(kind) => return Err(PyError::os_error_from_io(&std::io::Error::from(kind))),
        }
    }
}

/// `recv_into`/`readinto`'s in-place copy. The bytearray/array/memoryview
/// variants this codebase models are all owned by the borrowed `PyObject`,
/// so writing through a mutable borrow is in-place for the caller too.
fn sock_fill_buffer(target: &PyObjectRef, data: &[u8]) -> PyResult<usize> {
    let n = data.len();
    {
        let mut borrowed = target.borrow_mut();
        match &mut *borrowed {
            PyObject::ByteArray(buf) => {
                let n = n.min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                return Ok(n);
            }
            PyObject::Array(arr) if arr.typecode == 'B' || arr.typecode == 'b' => {
                let n = n.min(arr.data.len());
                for (slot, byte) in arr.data.iter_mut().zip(data.iter()) {
                    *slot = *byte as f64;
                }
                return Ok(n);
            }
            _ => {}
        }
    }
    Err(PyError::type_error(format!(
        "expected a writable bytes-like object, got '{}'",
        target.get_type_name()
    )))
}

// --- socket-backed file objects (`socket.makefile`) -----------------------
//
// `makefile` used to hand back a `PyObject::File` opened on /dev/null, so
// every stdlib protocol client that reads its response through it (`http.
// client`, `ftplib`, `poplib`, `smtplib`, `imaplib`, `nntplib`) saw an
// immediate EOF and could not complete a single exchange. This is a real
// file-like object over the socket instead: a read-ahead buffer kept in the
// instance dict, text mode decoding through the requested encoding, and the
// `io`-ish surface those clients use (`read`, `readline`, `read1`,
// `readinto`, `write`, `flush`, `close`, `fileno`, `isatty`, `readable`,
// `writable`, `seekable`, iteration, context manager).

const SOCKFILE_READ_CHUNK: usize = 8192;

/// CPython-shaped `UnicodeDecodeError(encoding, object, start, end, reason)`.
fn unicode_decode_error(encoding: &str, data: &[u8], valid_up_to: usize) -> PyError {
    let end = valid_up_to + 1;
    let byte_text = match data.get(valid_up_to) {
        Some(b) => format!("byte 0x{:02x}", b),
        None => "byte".to_string(),
    };
    PyError::Exception(
        "UnicodeDecodeError".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "UnicodeDecodeError".to_string(),
            args: vec![
                py_str(encoding),
                PyObjectRef::imm(PyObject::Bytes(data.to_vec())),
                py_int(valid_up_to as i64),
                py_int(end as i64),
                py_str(&format!("invalid start byte")),
            ],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: {
                let mut extra = std::collections::HashMap::new();
                extra.insert("object".to_string(), py_str(&byte_text));
                Some(extra)
            },
        }),
    )
}

fn sockfile_type() -> PyObjectRef {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    fn meth(name: &str, func: fn(&[PyObjectRef]) -> PyResult<PyObjectRef>) -> PyObjectRef {
        PyObjectRef::imm(PyObject::BuiltinMethod {
            name: name.to_string(),
            func,
            self_obj: PyObjectRef::new(PyObject::None),
        })
    }

    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    type_dict.insert("read".to_string(), meth("read", sockfile_read));
    type_dict.insert("read1".to_string(), meth("read1", sockfile_read1));
    type_dict.insert("readall".to_string(), meth("readall", sockfile_read_all));
    type_dict.insert("readline".to_string(), meth("readline", sockfile_readline));
    type_dict.insert(
        "readlines".to_string(),
        meth("readlines", sockfile_readlines),
    );
    type_dict.insert("readinto".to_string(), meth("readinto", sockfile_readinto));
    type_dict.insert("write".to_string(), meth("write", sockfile_write));
    type_dict.insert(
        "writelines".to_string(),
        meth("writelines", sockfile_writelines),
    );
    type_dict.insert("flush".to_string(), meth("flush", sockfile_flush));
    type_dict.insert("close".to_string(), meth("close", sockfile_close));
    type_dict.insert("fileno".to_string(), meth("fileno", sockfile_fileno));
    type_dict.insert("isatty".to_string(), meth("isatty", sockfile_false));
    type_dict.insert("readable".to_string(), meth("readable", sockfile_true));
    type_dict.insert("writable".to_string(), meth("writable", sockfile_true));
    type_dict.insert("seekable".to_string(), meth("seekable", sockfile_false));
    type_dict.insert("__enter__".to_string(), meth("__enter__", sockfile_self));
    type_dict.insert("__exit__".to_string(), meth("__exit__", sockfile_exit));
    type_dict.insert("__iter__".to_string(), meth("__iter__", sockfile_self));
    type_dict.insert("__next__".to_string(), meth("__next__", sockfile_next));
    type_dict.insert("__del__".to_string(), meth("__del__", sockfile_flush));

    let typ = PyObjectRef::new(PyObject::Type {
        name: "_io.BufferedReader".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
    }
    let _ = RefCell::new(0);
    let _ = Rc::new(0);
    typ
}

/// Build the file object `sock.makefile(mode, buffering, encoding, ...)`
/// returns: binary unless the caller asked for text, with a per-object
/// read-ahead buffer and the encoding recorded for text-mode decoding.
pub(crate) fn make_socket_file(
    socket: &PyObjectRef,
    binary: bool,
    encoding: Option<String>,
) -> PyObjectRef {
    let mut dict = AttrMap::new();
    dict.insert_str("_sock", socket.clone());
    dict.insert_str("_buf", PyObjectRef::imm(PyObject::Bytes(Vec::new())));
    dict.insert_str("_closed", py_bool(false));
    dict.insert_str("closed", py_bool(false));
    dict.insert_str("_binary", py_bool(binary));
    match &encoding {
        Some(enc) => {
            dict.insert_str("_encoding", py_str(enc));
        }
        None => {
            dict.insert_str("_encoding", py_none());
        }
    }
    PyObjectRef::new(PyObject::Instance {
        typ: sockfile_type(),
        dict,
    })
}

fn sockfile_dict(file: &PyObjectRef, key: &str) -> Option<PyObjectRef> {
    let borrowed = file.borrow();
    match &*borrowed {
        PyObject::Instance { dict, .. } => dict.get_str(key).cloned(),
        _ => None,
    }
}

fn sockfile_set(file: &PyObjectRef, key: &str, value: PyObjectRef) -> PyResult<()> {
    let key_obj = py_str(key);
    crate::object::subscript::py_setitem(file, &key_obj, value)
}

fn sockfile_socket(file: &PyObjectRef) -> PyResult<PyObjectRef> {
    sockfile_dict(file, "_sock")
        .ok_or_else(|| PyError::runtime_error("I/O operation on closed file"))
}

fn sockfile_is_binary(file: &PyObjectRef) -> bool {
    match sockfile_dict(file, "_binary") {
        Some(v) => matches!(&*v.borrow(), PyObject::Bool(true)),
        None => true,
    }
}

/// Read one chunk from the socket into the buffer. `Ok(false)` means EOF.
fn sockfile_fill(file: &PyObjectRef) -> PyResult<bool> {
    let socket = sockfile_socket(file)?;
    let data = match sock_recv_bytes(&socket, SOCKFILE_READ_CHUNK) {
        Ok(data) => data,
        Err(e) if e.type_name() == "OSError" && e.to_string().contains("timed out") => {
            return Err(e)
        }
        Err(e) => return Err(e),
    };
    if data.is_empty() {
        return Ok(false);
    }
    let mut buf = sockfile_dict(file, "_buf")
        .map(|b| match &*b.borrow() {
            PyObject::Bytes(v) => v.clone(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    buf.extend_from_slice(&data);
    sockfile_set(file, "_buf", PyObjectRef::imm(PyObject::Bytes(buf)))?;
    Ok(true)
}

/// Take up to `want` bytes off the front of the buffer, reading more (or
/// hitting EOF) as needed. `want == None` means "until EOF".
fn sockfile_take(file: &PyObjectRef, want: Option<usize>) -> PyResult<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    loop {
        let buf = sockfile_dict(file, "_buf")
            .map(|b| match &*b.borrow() {
                PyObject::Bytes(v) => v.clone(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        if buf.is_empty() {
            if !sockfile_fill(file)? {
                break;
            }
            continue;
        }
        match want {
            Some(0) => break,
            Some(n) => {
                let take = n.min(buf.len());
                out.extend_from_slice(&buf[..take]);
                let rest = buf[take..].to_vec();
                sockfile_set(file, "_buf", PyObjectRef::imm(PyObject::Bytes(rest)))?;
                if take < n {
                    // Short read: CPython's socket file returns what it has.
                    break;
                }
                break;
            }
            None => {
                out.extend_from_slice(&buf);
                sockfile_set(file, "_buf", PyObjectRef::imm(PyObject::Bytes(Vec::new())))?;
            }
        }
    }
    Ok(out)
}

fn sockfile_encode(file: &PyObjectRef, data: &[u8]) -> PyResult<PyObjectRef> {
    if sockfile_is_binary(file) {
        return Ok(PyObjectRef::imm(PyObject::Bytes(data.to_vec())));
    }
    let encoding = match sockfile_dict(file, "_encoding") {
        Some(v) => match &*v.borrow() {
            PyObject::Str(s) => s.as_str().to_string(),
            _ => "utf-8".to_string(),
        },
        None => "utf-8".to_string(),
    };
    decode_bytes_with_encoding(data, &encoding)
}

fn decode_bytes_with_encoding(data: &[u8], encoding: &str) -> PyResult<PyObjectRef> {
    let normalized = encoding.to_ascii_lowercase().replace(['-', '_'], "");
    let text = match normalized.as_str() {
        "latin1" | "iso88591" | "8859" | "cp1252" | "windows1252" => {
            data.iter().map(|&b| b as char).collect::<String>()
        }
        "ascii" | "usascii" => match std::str::from_utf8(data) {
            Ok(s) => s.to_string(),
            Err(e) => return Err(unicode_decode_error("ascii", data, e.valid_up_to())),
        },
        _ => match std::str::from_utf8(data) {
            Ok(s) => s.to_string(),
            Err(e) => return Err(unicode_decode_error(encoding, data, e.valid_up_to())),
        },
    };
    Ok(py_str(&text))
}

fn sockfile_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let file = &args[0];
    let want = args.get(1).and_then(|a| a.as_i64());
    let data = match want {
        Some(n) => sockfile_take(file, Some(n.max(0) as usize))?,
        None => sockfile_take(file, None)?,
    };
    sockfile_encode(file, &data)
}

fn sockfile_read1(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let file = &args[0];
    let want = args
        .get(1)
        .and_then(|a| a.as_i64())
        .unwrap_or(SOCKFILE_READ_CHUNK as i64)
        .max(0) as usize;
    let data = sockfile_take(file, Some(want))?;
    sockfile_encode(file, &data)
}

fn sockfile_read_all(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = sockfile_take(&args[0], None)?;
    sockfile_encode(&args[0], &data)
}

fn sockfile_readline(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let file = &args[0];
    let limit = args
        .get(1)
        .and_then(|a| a.as_i64())
        .filter(|n| *n >= 0)
        .map(|n| n as usize);
    let mut line: Vec<u8> = Vec::new();
    loop {
        let buf = sockfile_dict(file, "_buf")
            .map(|b| match &*b.borrow() {
                PyObject::Bytes(v) => v.clone(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        if buf.is_empty() {
            if !sockfile_fill(file)? {
                break;
            }
            continue;
        }
        match buf.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                line.extend_from_slice(&buf[..=idx]);
                let rest = buf[idx + 1..].to_vec();
                sockfile_set(file, "_buf", PyObjectRef::imm(PyObject::Bytes(rest)))?;
                break;
            }
            None => {
                line.extend_from_slice(&buf);
                sockfile_set(file, "_buf", PyObjectRef::imm(PyObject::Bytes(Vec::new())))?;
                if let Some(max) = limit {
                    if line.len() >= max {
                        break;
                    }
                }
            }
        }
    }
    sockfile_encode(file, &line)
}

fn sockfile_readlines(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let hint = args.get(1).and_then(|a| a.as_i64()).unwrap_or(0).max(0) as usize;
    let mut lines: Vec<PyObjectRef> = Vec::new();
    let mut total = 0usize;
    loop {
        let raw = sockfile_readline(&[args[0].clone()])?;
        let size = match &*raw.borrow() {
            PyObject::Str(s) => s.as_bytes().len(),
            PyObject::Bytes(b) => b.len(),
            _ => 0,
        };
        if size == 0 {
            break;
        }
        total += size;
        lines.push(raw);
        if hint > 0 && total >= hint {
            break;
        }
    }
    Ok(py_list(lines))
}

fn sockfile_readinto(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let file = &args[0];
    let target = args
        .get(1)
        .ok_or_else(|| PyError::type_error("readinto() takes exactly 1 argument"))?;
    let capacity = match arg_bytes(target) {
        Some(b) => b.len(),
        None => 65536,
    };
    let data = sockfile_take(file, Some(capacity))?;
    let written = sock_fill_buffer(target, &data)?;
    Ok(py_int(written as i64))
}

fn sockfile_write(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let file = &args[0];
    let socket: PyObjectRef = sockfile_socket(file)?;
    let payload = match &*args[1].borrow() {
        PyObject::Str(s) => s.as_bytes().to_vec(),
        _ => sock_arg_bytes(&args[1])?,
    };
    let written = sock_write_bytes(&socket, &payload, true)?;
    Ok(py_int(written as i64))
}

fn sockfile_writelines(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let items = match &*args[1].borrow() {
        PyObject::List(items) | PyObject::Tuple(items) => items.clone(),
        _ => {
            return Err(PyError::type_error(
                "writelines() argument must be a sequence",
            ))
        }
    };
    for item in items {
        sockfile_write(&[args[0].clone(), item])?;
    }
    Ok(py_none())
}

fn sockfile_flush(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(py_none())
}

fn sockfile_close(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `closed` is a plain attribute in CPython (`fp.closed` is a bool, not a
    // callable), so it lives in the instance dict alongside the internal
    // `_closed` flag rather than as a type-dict property: this codebase
    // resolves an instance-dict Property's getter through a path that hands
    // it the wrong `self`.
    sockfile_set(&args[0], "_closed", py_bool(true))?;
    sockfile_set(&args[0], "closed", py_bool(true))?;
    sockfile_set(&args[0], "_sock", PyObjectRef::new(PyObject::None))?;
    Ok(py_none())
}

fn sockfile_fileno(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let socket: PyObjectRef = sockfile_socket(&args[0])?;
    let fd = {
        let borrowed = socket.borrow();
        let inner = match &*borrowed {
            PyObject::Socket { inner, .. } => inner,
            _ => return Err(PyError::runtime_error("fileno on non-socket")),
        };
        use std::os::fd::AsRawFd;
        let guard = inner.borrow();
        match &*guard {
            SocketInner::TcpStream(s) => s.as_raw_fd(),
            SocketInner::TcpListener(l) => l.as_raw_fd(),
            SocketInner::UnixStream(s) => s.as_raw_fd(),
            SocketInner::UnixListener(l) => l.as_raw_fd(),
            SocketInner::Uninitialized => {
                return Err(PyError::OsError("Bad file descriptor".to_string()))
            }
        }
    };
    Ok(py_int(fd as i64))
}

fn sockfile_false(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(py_bool(false))
}

fn sockfile_true(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(py_bool(true))
}

fn sockfile_self(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(args[0].clone())
}

fn sockfile_exit(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    sockfile_close(&[args[0].clone()])?;
    Ok(py_bool(false))
}

fn sockfile_next(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let line = sockfile_readline(&[args[0].clone()])?;
    let empty = match &*line.borrow() {
        PyObject::Str(s) => s.as_bytes().is_empty(),
        PyObject::Bytes(b) => b.is_empty(),
        _ => true,
    };
    if empty {
        return Err(PyError::StopIteration);
    }
    Ok(line)
}

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
        PyObject::Socket {
            inner: _,
            family,
            type_,
            proto,
        } => {
            match name {
                "family" => return Ok(PyObjectRef::SmallInt(*family)),
                "type" => return Ok(PyObjectRef::SmallInt(*type_)),
                "proto" => return Ok(PyObjectRef::SmallInt(*proto)),
                // Real `detach()` leaks the underlying fd back to the
                // caller without closing it, marking this wrapper
                // closed. We don't have a raw-fd-owning socket
                // representation to hand back (no `socket.fromfd()`
                // either), but returning the fd number instead of
                // raising `AttributeError` unblocks callers that only
                // check truthiness/int-ness (real trigger:
                // `test_socket.py` setUp/tearDown helpers that call
                // `sock.detach()` unconditionally during cleanup).
                "detach" => {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "detach".to_string(),
                        func: |args| {
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner, .. } = socket {
                                use std::os::fd::AsRawFd;
                                let mut inner = inner.borrow_mut();
                                let old =
                                    std::mem::replace(&mut *inner, SocketInner::Uninitialized);
                                let fd = match &old {
                                    SocketInner::TcpListener(l) => l.as_raw_fd(),
                                    SocketInner::TcpStream(s) => s.as_raw_fd(),
                                    _ => -1,
                                };
                                std::mem::forget(old);
                                return Ok(py_int(fd as i64));
                            }
                            Err(PyError::runtime_error("detach on non-socket"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    }));
                }
                "bind" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "bind".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("bind() takes exactly 1 argument"));
                        }
                        // AF_UNIX binds a filesystem path, which cannot go
                        // through the (host, port) address parser at all.
                        let is_unix = matches!(
                            &*args[0].borrow(),
                            PyObject::Socket { family, .. } if *family == 1
                        );
                        if is_unix {
                            let path = unix_path_arg(&args[1])?;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner, .. } = socket {
                                let mut inner = inner.borrow_mut();
                                match &*inner {
                                    SocketInner::Uninitialized => {
                                        // CPython leaves a stale socket file
                                        // in place to be replaced; the OS
                                        // refuses to bind over one otherwise.
                                        let _ = std::fs::remove_file(&path);
                                        let listener =
                                            std::os::unix::net::UnixListener::bind(&path)
                                                .map_err(|e| PyError::os_error_from_io(&e))?;
                                        listener.set_nonblocking(true).ok();
                                        *inner = SocketInner::UnixListener(listener);
                                        return Ok(py_none());
                                    }
                                    _ => {
                                        return Err(PyError::runtime_error(
                                            "socket already bound or connected",
                                        ))
                                    }
                                }
                            }
                            return Err(PyError::runtime_error("bind on non-socket"));
                        }
                        let addr = socket_addr_to_string(&args[1])?;
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = socket {
                            let mut inner = inner.borrow_mut();
                            match &*inner {
                                SocketInner::Uninitialized => {
                                    let listener = std::net::TcpListener::bind(&addr)
                                        .map_err(|e| PyError::os_error_from_io(&e))?;
                                    listener.set_nonblocking(true).ok();
                                    *inner = SocketInner::TcpListener(listener);
                                    Ok(py_none())
                                }
                                _ => {
                                    Err(PyError::runtime_error("socket already bound or connected"))
                                }
                            }
                        } else {
                            Err(PyError::runtime_error("bind on non-socket"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "listen" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "listen".to_string(),
                    func: |args| {
                        let backlog = if args.len() > 1 {
                            args[1].as_i64().unwrap_or(5) as i32
                        } else {
                            5
                        };
                        let _ = backlog;
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = socket {
                            let inner = inner.borrow();
                            match &*inner {
                                SocketInner::TcpListener(_listener) => Ok(py_none()),
                                SocketInner::UnixListener(_listener) => Ok(py_none()),
                                _ => Err(PyError::runtime_error("listen on non-listener")),
                            }
                        } else {
                            Err(PyError::runtime_error("listen on non-socket"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // setblocking(flag): flips the socket between CPython's
                // blocking default and `BlockingIOError`-on-empty, which is
                // what `selectors`/`asyncore` set their fds to.
                "setblocking" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "setblocking".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "setblocking() takes exactly 1 argument",
                            ));
                        }
                        let flag = match &*args[1].borrow() {
                            PyObject::Bool(b) => *b,
                            PyObject::Int(i) => *i != BigInt::from(0),
                            _ => {
                                return Err(PyError::type_error("argument must be an int or bool"))
                            }
                        };
                        let socket = args[0].clone();
                        set_sock_mode(
                            &socket,
                            if flag {
                                SockMode::Blocking
                            } else {
                                SockMode::NonBlocking
                            },
                        );
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // settimeout(value): now really applies a read/write timeout
                // to the underlying stream (and is what `gettimeout` and
                // `getsockopt(SO_TIMEOUT)` report back), so a client that
                // sets a deadline and waits for a server that never answers
                // actually raises `socket.timeout` instead of hanging.
                "settimeout" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "settimeout".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "settimeout() takes exactly 1 argument",
                            ));
                        }
                        let seconds = match &*args[1].borrow() {
                            PyObject::None => None,
                            _ => Some(
                                args[1]
                                    .as_f64()
                                    .ok_or_else(|| PyError::type_error("a float is required"))?,
                            ),
                        };
                        let socket = args[0].clone();
                        set_sock_mode(
                            &socket,
                            match seconds {
                                Some(secs) => SockMode::Deadline(secs),
                                None => SockMode::Blocking,
                            },
                        );
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // fileno(): OS-level descriptor when one exists.
                "fileno" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "fileno".to_string(),
                    func: |args| {
                        let sock = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = sock {
                            let inner = inner.borrow();
                            use std::os::fd::AsRawFd;
                            let fd = match &*inner {
                                SocketInner::TcpListener(l) => l.as_raw_fd(),
                                SocketInner::TcpStream(s) => s.as_raw_fd(),
                                _ => -1,
                            };
                            return Ok(py_int(fd as i64));
                        }
                        Err(PyError::runtime_error("fileno on non-socket"))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "accept" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "accept".to_string(),
                    func: |args| {
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket {
                            inner,
                            family,
                            type_,
                            proto,
                        } = socket
                        {
                            let (family, type_, proto) = (*family, *type_, *proto);
                            let mut inner = inner.borrow_mut();
                            let old = std::mem::replace(&mut *inner, SocketInner::Uninitialized);
                            match old {
                                SocketInner::TcpListener(listener) => {
                                    // Every native socket here is created
                                    // non-blocking (see `bind`), but real
                                    // Python sockets default to BLOCKING
                                    // — there's no `setblocking`/`settimeout`
                                    // exposed at all yet, so nothing ever
                                    // legitimately wants `accept()` to
                                    // return `WouldBlock` immediately.
                                    // Retry with a short sleep (bounded,
                                    // to avoid a truly-never-connecting
                                    // test hanging forever) to emulate
                                    // blocking `accept()` faithfully.
                                    // Real trigger: `test_selectors.py`'s
                                    // own `socketpair()` fallback, whose
                                    // `l.accept()` call right after a
                                    // same-process `connect()` otherwise
                                    // raced the kernel's backlog queue.
                                    let deadline = std::time::Instant::now()
                                        + std::time::Duration::from_secs(5);
                                    let result = loop {
                                        match listener.accept() {
                                            Err(e)
                                                if e.kind() == std::io::ErrorKind::WouldBlock
                                                    && std::time::Instant::now() < deadline =>
                                            {
                                                std::thread::sleep(
                                                    std::time::Duration::from_millis(1),
                                                );
                                                continue;
                                            }
                                            other => break other,
                                        }
                                    };
                                    match result {
                                        Ok((stream, addr)) => {
                                            *inner = SocketInner::TcpListener(listener);
                                            let client = PyObjectRef::new(PyObject::Socket {
                                                inner: std::rc::Rc::new(std::cell::RefCell::new(
                                                    SocketInner::TcpStream(stream),
                                                )),
                                                family,
                                                type_,
                                                proto,
                                            });
                                            // Real `accept()` returns
                                            // `(host, port)`, not a
                                            // string — same fix as
                                            // `getsockname`/`getpeername`.
                                            Ok(py_tuple(vec![
                                                client,
                                                socket_addr_to_py_tuple(addr),
                                            ]))
                                        }
                                        Err(e) => {
                                            *inner = SocketInner::TcpListener(listener);
                                            Err(PyError::os_error_from_io(&e))
                                        }
                                    }
                                }
                                SocketInner::UnixListener(listener) => {
                                    let deadline = std::time::Instant::now()
                                        + std::time::Duration::from_secs(5);
                                    let result = loop {
                                        match listener.accept() {
                                            Err(e)
                                                if e.kind() == std::io::ErrorKind::WouldBlock
                                                    && std::time::Instant::now() < deadline =>
                                            {
                                                crate::modules::coop_threads_drain();
                                                std::thread::sleep(
                                                    std::time::Duration::from_millis(1),
                                                );
                                                continue;
                                            }
                                            other => break other,
                                        }
                                    };
                                    match result {
                                        Ok((stream, addr)) => {
                                            *inner = SocketInner::UnixListener(listener);
                                            let client = PyObjectRef::new(PyObject::Socket {
                                                inner: Rc::new(std::cell::RefCell::new(
                                                    SocketInner::UnixStream(stream),
                                                )),
                                                family,
                                                type_,
                                                proto,
                                            });
                                            // AF_UNIX's accept() returns a
                                            // bare path (often empty for an
                                            // unnamed peer), not a tuple.
                                            Ok(py_tuple(vec![client, path_or_empty(&addr)]))
                                        }
                                        Err(e) => {
                                            *inner = SocketInner::UnixListener(listener);
                                            Err(PyError::os_error_from_io(&e))
                                        }
                                    }
                                }
                                other => {
                                    *inner = other;
                                    Err(PyError::runtime_error("accept on non-listener"))
                                }
                            }
                        } else {
                            Err(PyError::runtime_error("accept on non-socket"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "connect" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "connect".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("connect() takes exactly 1 argument"));
                        }
                        let is_unix = matches!(&*args[0].borrow(), PyObject::Socket { family, .. } if *family == 1);
                        if is_unix {
                            let path = unix_path_arg(&args[1])?;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner, .. } = socket {
                                let mut inner = inner.borrow_mut();
                                match &*inner {
                                    SocketInner::Uninitialized => {
                                        match std::os::unix::net::UnixStream::connect(&path) {
                                            Ok(stream) => {
                                                stream.set_nonblocking(true).ok();
                                                *inner = SocketInner::UnixStream(stream);
                                                return Ok(py_none());
                                            }
                                            Err(e) => return Err(PyError::os_error_from_io(&e)),
                                        }
                                    }
                                    _ => {
                                        return Err(PyError::runtime_error(
                                            "socket already connected or listening",
                                        ))
                                    }
                                }
                            }
                            return Err(PyError::runtime_error("connect on non-socket"));
                        }
                        let addr = socket_addr_to_string(&args[1])?;
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = socket {
                            let mut inner = inner.borrow_mut();
                            match &*inner {
                                SocketInner::Uninitialized => {
                                    match std::net::TcpStream::connect(&addr) {
                                        Ok(stream) => {
                                            stream.set_nonblocking(true).ok();
                                            *inner = SocketInner::TcpStream(stream);
                                            Ok(py_none())
                                        }
                                        Err(e) => Err(PyError::os_error_from_io(&e)),
                                    }
                                }
                                _ => Err(PyError::runtime_error(
                                    "socket already connected or listening",
                                )),
                            }
                        } else {
                            Err(PyError::runtime_error("connect on non-socket"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // `send`/`sendall` used to stringify their payload (so any
                // binary body went out as `<binary>`) and `send` reported
                // `write_all`'s byte count while discarding how much a single
                // write actually moved; both are wrong for every protocol
                // client (http/ftp/pop/imap read these bytes back verbatim).
                "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "send".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("send() takes exactly 1 argument"));
                        }
                        let data = sock_arg_bytes(&args[1])?;
                        if data.is_empty() {
                            return Ok(py_int(0));
                        }
                        let sent = sock_write_bytes(&args[0], &data, false)?;
                        Ok(py_int(sent))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "sendall" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "sendall".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("sendall() takes exactly 1 argument"));
                        }
                        let data = sock_arg_bytes(&args[1])?;
                        sock_write_bytes(&args[0], &data, true)?;
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // `sendto(data, address)`/`recvfrom(bufsize)` — UDP-shaped,
                // but `smtplib`/`ftplib` datagram tests call them against the
                // SOCK_STREAM socket this interpreter hands out, so accept
                // the call and behave like send/recv rather than raising
                // TypeError.
                "sendto" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "sendto".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("sendto() takes at least 2 arguments"));
                        }
                        let data = sock_arg_bytes(&args[1])?;
                        let sent = sock_write_bytes(&args[0], &data, true)?;
                        Ok(py_int(sent))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "recvfrom" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "recvfrom".to_string(),
                    func: |args| {
                        let bufsize = args.get(1).and_then(|a| a.as_i64()).unwrap_or(4096) as usize;
                        let data = sock_recv_bytes(&args[0], bufsize)?;
                        Ok(py_tuple(vec![
                            PyObjectRef::imm(PyObject::Bytes(data)),
                            py_none(),
                        ]))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "recv_into" | "recv_into1" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: name.to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "recv_into() takes at least 1 argument",
                            ));
                        }
                        let explicit = args.get(2).and_then(|a| a.as_i64());
                        let bufsize = match explicit {
                            Some(0) | None => 65536,
                            Some(n) => n as usize,
                        };
                        let data = sock_recv_bytes(&args[0], bufsize)?;
                        let written = sock_fill_buffer(&args[1], &data)?;
                        Ok(py_int(written as i64))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "shutdown" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "shutdown".to_string(),
                    func: |args| {
                        use std::net::Shutdown;
                        let how = args.get(1).and_then(|a| a.as_i64()).unwrap_or(2);
                        let borrowed = args[0].borrow();
                        match &*borrowed {
                            PyObject::Socket { inner, .. } => {
                                let mut guard = inner.borrow_mut();
                                match &mut *guard {
                                    SocketInner::TcpStream(s) => {
                                        let mode = match how {
                                            0 => Shutdown::Read,
                                            1 => Shutdown::Write,
                                            _ => Shutdown::Both,
                                        };
                                        let _ = s.shutdown(mode);
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::runtime_error("shutdown on non-stream")),
                                }
                            }
                            _ => Err(PyError::runtime_error("shutdown on non-socket")),
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "gettimeout" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "gettimeout".to_string(),
                    func: |args| {
                        let socket = args[0].clone();
                        match sock_timeout_for(&socket) {
                            Some(seconds) => Ok(py_float(seconds)),
                            None => Ok(py_none()),
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // `recv` returned a `str` (and the literal string
                // "<binary>" for anything that wasn't valid UTF-8), so every
                // protocol client in the stdlib read garbage back. CPython's
                // contract is `bytes`, and the length is whatever arrived.
                "recv" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "recv".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("recv() takes exactly 1 argument"));
                        }
                        let bufsize = args[1].as_i64().unwrap_or(4096).max(0) as usize;
                        let data = sock_recv_bytes(&args[0], bufsize)?;
                        Ok(PyObjectRef::imm(PyObject::Bytes(data)))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "close".to_string(),
                    func: |args| {
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = socket {
                            let mut inner = inner.borrow_mut();
                            let old = std::mem::replace(&mut *inner, SocketInner::Uninitialized);
                            drop(old);
                            Ok(py_none())
                        } else {
                            Err(PyError::runtime_error("close on non-socket"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "setsockopt" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "setsockopt".to_string(),
                    func: |_| Ok(py_none()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // `getsockopt` was missing entirely. Callers overwhelmingly
                // use it to ask "is there a pending error?" (SO_ERROR, 0) or
                // to read back a timeout, so answer those truthfully and
                // report a zero-length buffer for everything else instead of
                // raising AttributeError.
                "getsockopt" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "getsockopt".to_string(),
                    func: |args| {
                        let level = args.get(1).and_then(|a| a.as_i64()).unwrap_or(0);
                        let optname = args.get(2).and_then(|a| a.as_i64()).unwrap_or(0);
                        let socket = args[0].clone();
                        let buflen = args.get(3).and_then(|a| a.as_i64()).unwrap_or(256);
                        // SO_TYPE (3) is 1; SO_ERROR (4) is 0; SO_TIMEOUT is
                        // the current timeout in seconds.
                        let (sol_socket, so_type, so_error, so_timeout) = (1, 3, 4, 20);
                        let value: PyObjectRef = if level == sol_socket && optname == so_type {
                            match &*socket.borrow() {
                                PyObject::Socket { type_, .. } => py_int(*type_),
                                _ => py_int(1),
                            }
                        } else if level == sol_socket && optname == so_error {
                            py_int(0)
                        } else if level == sol_socket && optname == so_timeout {
                            match sock_timeout_for(&socket) {
                                Some(seconds) => py_float(seconds),
                                None => py_none(),
                            }
                        } else if optname == 0 || optname == 1 {
                            py_int(0)
                        } else {
                            py_int(0)
                        };
                        let raw = value.as_i64();
                        match raw {
                            Some(i) if buflen >= 8 => Ok(py_int(i)),
                            _ => Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new()))),
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "dup" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "dup".to_string(),
                    func: |args| {
                        use std::net::TcpStream;
                        let borrowed = args[0].borrow();
                        let inner = match &*borrowed {
                            PyObject::Socket { inner, .. } => inner,
                            _ => return Err(PyError::runtime_error("dup on non-socket")),
                        };
                        let guard = inner.borrow();
                        match &*guard {
                            SocketInner::TcpStream(s) => {
                                let cloned =
                                    s.try_clone().map_err(|e| PyError::os_error_from_io(&e))?;
                                drop(guard);
                                let _ = cloned.set_nodelay(true);
                                Ok(PyObjectRef::new(PyObject::Socket {
                                    inner: Rc::new(std::cell::RefCell::new(
                                        SocketInner::TcpStream(cloned),
                                    )),
                                    family: 2,
                                    type_: 1,
                                    proto: 0,
                                }))
                            }
                            _ => Err(PyError::runtime_error("dup on non-stream")),
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "connect_ex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "connect_ex".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "connect_ex() takes exactly 1 argument",
                            ));
                        }
                        let addr = socket_addr_to_string(&args[1])?;
                        let borrowed = args[0].borrow();
                        let inner = match &*borrowed {
                            PyObject::Socket { inner, .. } => inner.clone(),
                            _ => return Err(PyError::runtime_error("connect_ex on non-socket")),
                        };
                        drop(borrowed);
                        match std::net::TcpStream::connect(addr.as_str()) {
                            Ok(stream) => {
                                *inner.borrow_mut() = SocketInner::TcpStream(stream);
                                Ok(py_int(0))
                            }
                            Err(e) => Ok(py_int(e.raw_os_error().unwrap_or(-1) as i64)),
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // Real `socket.socket` objects are context managers
                // (`__enter__` returns `self`, `__exit__` closes the
                // socket unconditionally) — this was entirely missing,
                // so `with socket.socket(...) as s:` raised
                // `AttributeError: 'socket' object has no attribute
                // '__exit__'` for every native socket use anywhere.
                // Real trigger: `test_selectors.py`'s own `socketpair()`
                // fallback helper, which every selector test transitively
                // calls via `self.make_socketpair()`.
                "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__enter__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__exit__".to_string(),
                    func: |args| {
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = socket {
                            let mut inner = inner.borrow_mut();
                            let old = std::mem::replace(&mut *inner, SocketInner::Uninitialized);
                            drop(old);
                        }
                        Ok(py_bool(false))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // Real `socket.getsockname()`/`getpeername()` return a
                // `(host, port)` tuple, not a string — missing entirely
                // before, breaking any test helper that binds/connects
                // then inspects the resulting address (e.g.
                // `test_selectors.py`'s own `socketpair()` fallback,
                // whose `l.getsockname()` call is on the hot path for
                // every selector test transitively via
                // `self.make_socketpair()`).
                "getsockname" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "getsockname".to_string(),
                    func: |args| {
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = socket {
                            let inner = inner.borrow();
                            match &*inner {
                                SocketInner::TcpListener(l) => {
                                    let addr = l
                                        .local_addr()
                                        .map_err(|e| PyError::os_error_from_io(&e))?;
                                    Ok(socket_addr_to_py_tuple(addr))
                                }
                                SocketInner::TcpStream(s) => {
                                    let addr = s
                                        .local_addr()
                                        .map_err(|e| PyError::os_error_from_io(&e))?;
                                    Ok(socket_addr_to_py_tuple(addr))
                                }
                                // AF_UNIX addresses are filesystem paths, and
                                // `getsockname()` returns the bare string
                                // (not a (host, port) tuple).
                                SocketInner::UnixListener(l) => match l.local_addr() {
                                    Ok(addr) => Ok(path_or_empty(&addr)),
                                    Err(_) => Ok(py_str("")),
                                },
                                SocketInner::UnixStream(s) => match s.local_addr() {
                                    Ok(addr) => Ok(path_or_empty(&addr)),
                                    Err(_) => Ok(py_str("")),
                                },
                                SocketInner::Uninitialized => {
                                    Err(PyError::OsError("Bad file descriptor".to_string()))
                                }
                            }
                        } else {
                            Err(PyError::runtime_error("getsockname on non-socket"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "getpeername" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "getpeername".to_string(),
                    func: |args| {
                        let socket = &*args[0].borrow();
                        if let PyObject::Socket { inner, .. } = socket {
                            let inner = inner.borrow();
                            let addr = match &*inner {
                                SocketInner::TcpStream(s) => s.peer_addr(),
                                _ => {
                                    return Err(PyError::OsError(
                                        "Socket is not connected".to_string(),
                                    ))
                                }
                            }
                            .map_err(|e| PyError::os_error_from_io(&e))?;
                            Ok(socket_addr_to_py_tuple(addr))
                        } else {
                            Err(PyError::runtime_error("getpeername on non-socket"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "makefile" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "makefile".to_string(),
                    func: |args| {
                        let mode = args
                            .get(1)
                            .map(|m| m.str())
                            .unwrap_or_else(|| "r".to_string());
                        let binary = mode.contains('b') || mode.contains('a');
                        // CPython's signature is makefile(mode='r',
                        // buffering=None, *, encoding=None, errors=None,
                        // newline=None); `encoding` forces text mode.
                        let kwargs = args
                            .last()
                            .filter(|a| matches!(&*a.borrow(), PyObject::Dict(d) if !d.is_empty()));
                        let mut encoding: Option<String> = None;
                        if let Some(kw) = kwargs {
                            let found = {
                                let borrowed = kw.borrow();
                                match &*borrowed {
                                    PyObject::Dict(_) => crate::object::subscript::py_getitem(
                                        &kw,
                                        &py_str("encoding"),
                                    )
                                    .ok(),
                                    _ => None,
                                }
                            };
                            if let Some(v) = found {
                                if !matches!(&*v.borrow(), PyObject::None) {
                                    encoding = Some(v.str());
                                }
                            }
                        }
                        let binary = binary && encoding.is_none();
                        let socket = args[0].clone();
                        Ok(make_socket_file(&socket, binary, encoding))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'socket' object has no attribute '{}'",
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
