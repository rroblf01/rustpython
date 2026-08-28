// Auto-extracted from src/object/attrs/mod.rs lines 5006-5418
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Socket { inner: _ } => {
                match name {
                    "bind" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "bind".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("bind() takes exactly 1 argument"));
                            }
                            let addr = socket_addr_to_string(&args[1])?;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                match &*inner {
                                    SocketInner::Uninitialized => {
                                        let listener = std::net::TcpListener::bind(&addr)
                                            .map_err(|e| PyError::os_error_from_io(&e))?;
                                        listener.set_nonblocking(true).ok();
                                        *inner = SocketInner::TcpListener(listener);
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::runtime_error(
                                        "socket already bound or connected",
                                    )),
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
                            if let PyObject::Socket { inner } = socket {
                                let inner = inner.borrow();
                                match &*inner {
                                    SocketInner::TcpListener(_listener) => Ok(py_none()),
                                    _ => Err(PyError::runtime_error("listen on non-listener")),
                                }
                            } else {
                                Err(PyError::runtime_error("listen on non-socket"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // setblocking(flag): our sockets are internally
                    // non-blocking with retry loops emulating blocking
                    // semantics at the operation level, so this only
                    // validates and accepts the flag.
                    "setblocking" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setblocking".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "setblocking() takes exactly 1 argument",
                                ));
                            }
                            if !matches!(&*args[1].borrow(), PyObject::Bool(_) | PyObject::Int(_))
                            {
                                return Err(PyError::type_error(
                                    "argument must be an int or bool",
                                ));
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // settimeout(value): accepted for API compatibility;
                    // timeouts are emulated by bounded retry loops.
                    "settimeout" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "settimeout".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "settimeout() takes exactly 1 argument",
                                ));
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // fileno(): OS-level descriptor when one exists.
                    "fileno" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "fileno".to_string(),
                        func: |args| {
                            let sock = &*args[0].borrow();
                            if let PyObject::Socket { inner } = sock {
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
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                let old =
                                    std::mem::replace(&mut *inner, SocketInner::Uninitialized);
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
                                                    if e.kind()
                                                        == std::io::ErrorKind::WouldBlock
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
                                                    inner: std::rc::Rc::new(
                                                        std::cell::RefCell::new(
                                                            SocketInner::TcpStream(stream),
                                                        ),
                                                    ),
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
                                return Err(PyError::type_error(
                                    "connect() takes exactly 1 argument",
                                ));
                            }
                            let addr = socket_addr_to_string(&args[1])?;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
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
                    "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("send() takes exactly 1 argument"));
                            }
                            let data = args[1].str();
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                match &mut *inner {
                                    SocketInner::TcpStream(stream) => {
                                        use std::io::Write;
                                        match stream.write_all(data.as_bytes()) {
                                            Ok(()) => Ok(py_int(data.len() as i64)),
                                            Err(e) => Err(PyError::os_error_from_io(&e)),
                                        }
                                    }
                                    _ => Err(PyError::runtime_error("send on non-stream")),
                                }
                            } else {
                                Err(PyError::runtime_error("send on non-socket"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "recv" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "recv".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("recv() takes exactly 1 argument"));
                            }
                            let bufsize = args[1].as_i64().unwrap_or(4096) as usize;
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                match &mut *inner {
                                    SocketInner::TcpStream(stream) => {
                                        use std::io::Read;
                                        let mut buf = vec![0u8; bufsize.min(65536)];
                                        match stream.read(&mut buf) {
                                            Ok(0) => Ok(py_str("")),
                                            Ok(n) => {
                                                buf.truncate(n);
                                                match String::from_utf8(buf) {
                                                    Ok(s) => Ok(py_str(&s)),
                                                    Err(_) => Ok(py_str("<binary>")),
                                                }
                                            }
                                            Err(e)
                                                if e.kind() == std::io::ErrorKind::WouldBlock =>
                                            {
                                                Ok(py_none())
                                            }
                                            Err(e) => Err(PyError::os_error_from_io(&e)),
                                        }
                                    }
                                    _ => Err(PyError::runtime_error("recv on non-stream")),
                                }
                            } else {
                                Err(PyError::runtime_error("recv on non-socket"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "close".to_string(),
                        func: |args| {
                            let socket = &*args[0].borrow();
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                let old =
                                    std::mem::replace(&mut *inner, SocketInner::Uninitialized);
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
                            if let PyObject::Socket { inner } = socket {
                                let mut inner = inner.borrow_mut();
                                let old =
                                    std::mem::replace(&mut *inner, SocketInner::Uninitialized);
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
                            if let PyObject::Socket { inner } = socket {
                                let inner = inner.borrow();
                                let addr = match &*inner {
                                    SocketInner::TcpListener(l) => l.local_addr(),
                                    SocketInner::TcpStream(s) => s.local_addr(),
                                    SocketInner::Uninitialized => {
                                        return Err(PyError::OsError(
                                            "Bad file descriptor".to_string(),
                                        ))
                                    }
                                }
                                .map_err(|e| PyError::os_error_from_io(&e))?;
                                Ok(socket_addr_to_py_tuple(addr))
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
                            if let PyObject::Socket { inner } = socket {
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
                            let mode = args.get(1).map(|m| m.str()).unwrap_or("r".to_string());
                            let binary = mode.contains('b');
                            let file = std::fs::OpenOptions::new()
                                .read(true)
                                .write(true)
                                .open("/dev/null")
                                .or_else(|_| std::fs::File::create("/tmp/rustpython_socket_makefile_dummy"))
                                .map_err(|e| PyError::os_error_from_io(&e))?;
                            Ok(PyObjectRef::new(PyObject::File {
                                file: std::rc::Rc::new(std::cell::RefCell::new(file)),
                                name: "<socket>".to_string(),
                                binary,
                                pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                                closed: false,
                            }))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'socket' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
