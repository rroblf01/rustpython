use crate::object::*;
use std::collections::HashMap;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::rc::Rc;

mod html;
pub use html::*;
mod subprocess;
pub use subprocess::*;
mod http_client;
pub use http_client::*;

pub const HTTP_SOURCE: &str = "";

pub fn create_select_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sel_func {
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

    // ---- poll type helpers (disallow direct instantiation, factory returns instance) ----
    fn make_poll_type(name: &str) -> PyObjectRef {
        let mut dict = HashMap::new();
        let name_owned = name.to_string();
        // __new__ must raise TypeError: cannot create 'select.poll' instances
        let name_clone = name_owned.clone();
        dict.insert(
            "__new__".to_string(),
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(move |_args: &[PyObjectRef]| {
                Err(PyError::type_error(format!("cannot create '{}' instances", name_clone)))
            }))),
        );
        let typ = PyObjectRef::new(PyObject::Type {
            name: name_owned.clone(),
            dict: Box::new(str_map_to_typedict(dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
            *mro = vec![typ.clone()];
        }
        typ
    }
    let poll_type = make_poll_type("select.poll");
    let devpoll_type = make_poll_type("select.devpoll");
    // for closure captures
    let poll_type_for_factory = poll_type.clone();
    let devpoll_type_for_factory = devpoll_type.clone();

    // Helper to build OSError with EBADF (errno 9)
    fn ebadf_error() -> PyError {
        let mut extra = std::collections::HashMap::new();
        extra.insert("errno".to_string(), py_int(9));
        extra.insert("strerror".to_string(), py_str("Bad file descriptor"));
        PyError::Exception(
            "OSError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "OSError".to_string(),
                args: vec![py_int(9), py_str("Bad file descriptor")],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: Some(extra),
            }),
        )
    }

    // poll(2) FFI
    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    const POLLIN: i16 = 0x0001;
    const POLLPRI: i16 = 0x0002;
    const POLLOUT: i16 = 0x0004;
    const POLLERR: i16 = 0x0008;
    const POLLHUP: i16 = 0x0010;
    const POLLNVAL: i16 = 0x0020;
    extern "C" {
        fn poll(fds: *mut PollFd, nfds: u32, timeout: i32) -> i32;
    }

    fn is_sequence(obj: &PyObjectRef) -> bool {
        matches!(&*obj.borrow(), PyObject::List(_) | PyObject::Tuple(_))
    }

    fn get_fd(obj: &PyObjectRef) -> Result<i32, PyError> {
        if let Some(i) = obj.as_i64() {
            if i < 0 || i > i32::MAX as i64 {
                return Err(PyError::value_error("filedescriptor out of range in select()"));
            }
            return Ok(i as i32);
        }
        // try fileno()
        let fileno_attr = match obj.borrow().get_attribute("fileno") {
            Ok(m) => m,
            Err(_) => {
                return Err(PyError::type_error(
                    "argument must be an int, or have a fileno() method.",
                ))
            }
        };
        let result = crate::object::call_bound_method(fileno_attr, obj.clone(), vec![])?;
        if let Some(i) = result.as_i64() {
            if i < 0 || i > i32::MAX as i64 {
                return Err(PyError::value_error("filedescriptor out of range in select()"));
            }
            Ok(i as i32)
        } else {
            Err(PyError::type_error("fileno() returned a non-integer"))
        }
    }

    fn collect_fds(obj: &PyObjectRef) -> Result<Vec<(PyObjectRef, i32)>, PyError> {
        if !is_sequence(obj) {
            return Err(PyError::type_error("arguments 1-3 must be sequences"));
        }
        let mut out = Vec::new();
        let mut idx: usize = 0;
        loop {
            let len = {
                let b = obj.borrow();
                match &*b {
                    PyObject::List(v) => v.len(),
                    PyObject::Tuple(v) => v.len(),
                    _ => 0,
                }
            };
            if idx >= len {
                break;
            }
            let item = {
                let b = obj.borrow();
                match &*b {
                    PyObject::List(v) => v[idx].clone(),
                    PyObject::Tuple(v) => v[idx].clone(),
                    _ => unreachable!(),
                }
            };
            let fd = get_fd(&item)?;
            out.push((item, fd));
            idx += 1;
        }
        Ok(out)
    }

    sel_func!("select", move |args| {
        let (positional, kwargs) = match args.last() {
            Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
                if let PyObject::Dict(d) = &*last.borrow() {
                    let has_timeout = d.get(&py_str("timeout")).ok().flatten().is_some();
                    if has_timeout || args.len() > 4 {
                        (&args[..args.len()-1], Some(d.clone()))
                    } else {
                        (args, None)
                    }
                } else { (args, None) }
            }
            _ => (args, None),
        };
        let pos = positional;
        if pos.len() < 3 {
            return Err(PyError::type_error("select() takes at least 3 arguments"));
        }
        if pos.len() > 4 && kwargs.is_none() {
            return Err(PyError::type_error(format!("select() takes at most 4 arguments ({} given)", pos.len())));
        }
        let rlist = &pos[0];
        let wlist = &pos[1];
        let xlist = &pos[2];

        let r_vec = collect_fds(rlist)?;
        let w_vec = collect_fds(wlist)?;
        let x_vec = collect_fds(xlist)?;

        let timeout_opt: Option<f64> = if let Some(kw) = kwargs {
            if let Some(v) = kw.get(&py_str("timeout")).ok().flatten() {
                if matches!(&*v.borrow(), PyObject::None) { None }
                else if let Some(f) = v.as_f64() { Some(f) }
                else { return Err(PyError::type_error("timeout must be a float or None")); }
            } else if pos.len() >= 4 {
                let t = &pos[3];
                if matches!(&*t.borrow(), PyObject::None) { None }
                else {
                    let is_numeric = matches!(&*t.borrow(), PyObject::Int(_) | PyObject::Float(_)) || matches!(t, PyObjectRef::SmallInt(_) | PyObjectRef::SmallBool(_) | PyObjectRef::SmallFloat(_));
                    if !is_numeric { return Err(PyError::type_error("timeout must be a float or None")); }
                    if let Some(f) = t.as_f64() { Some(f) } else { return Err(PyError::type_error("timeout must be a float or None")); }
                }
            } else { None }
        } else if pos.len() >= 4 {
            let t = &pos[3];
            if matches!(&*t.borrow(), PyObject::None) { None }
            else {
                let is_numeric = matches!(&*t.borrow(), PyObject::Int(_) | PyObject::Float(_)) || matches!(t, PyObjectRef::SmallInt(_) | PyObjectRef::SmallBool(_) | PyObjectRef::SmallFloat(_));
                if !is_numeric { return Err(PyError::type_error("timeout must be a float or None")); }
                if let Some(f) = t.as_f64() { Some(f) } else { return Err(PyError::type_error("timeout must be a float or None")); }
            }
        } else { None };

        if let Some(f) = timeout_opt {
            if f < 0.0 {
                return Err(PyError::value_error("timeout must be non-negative"));
            }
        }

        let timeout_ms: i32 = match timeout_opt {
            None => -1,
            Some(f) => {
                if f == 0.0 { 0 } else {
                    let ms = (f * 1000.0).trunc() as i64;
                    if ms > i32::MAX as i64 { i32::MAX } else if ms < 0 { 0 } else { ms as i32 }
                }
            }
        };

        if r_vec.is_empty() && w_vec.is_empty() && x_vec.is_empty() {
            if let Some(f) = timeout_opt {
                if f > 0.0 {
                    std::thread::sleep(std::time::Duration::from_secs_f64(f));
                }
            }
            return Ok(py_tuple(vec![py_list(vec![]), py_list(vec![]), py_list(vec![])]));
        }

        let mut fd_to_events: std::collections::HashMap<i32, i16> = std::collections::HashMap::new();
        for (_, fd) in &r_vec { *fd_to_events.entry(*fd).or_insert(0) |= POLLIN; }
        for (_, fd) in &w_vec { *fd_to_events.entry(*fd).or_insert(0) |= POLLOUT; }
        for (_, fd) in &x_vec { *fd_to_events.entry(*fd).or_insert(0) |= POLLPRI; }

        let mut poll_fds: Vec<PollFd> = fd_to_events.iter().map(|(&fd, &ev)| PollFd { fd, events: ev, revents: 0 }).collect();
        let rc = unsafe { poll(poll_fds.as_mut_ptr(), poll_fds.len() as u32, timeout_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(9) {
                return Err(ebadf_error());
            }
            return Err(PyError::os_error_from_io(&err));
        }
        for pfd in &poll_fds {
            if pfd.revents & POLLNVAL != 0 {
                return Err(ebadf_error());
            }
        }
        let mut fd_to_revents: std::collections::HashMap<i32, i16> = std::collections::HashMap::new();
        for pfd in poll_fds {
            fd_to_revents.insert(pfd.fd, pfd.revents);
        }

        let mut ready_r = Vec::new();
        for (obj, fd) in r_vec {
            let rev = fd_to_revents.get(&fd).copied().unwrap_or(0);
            if rev & POLLIN != 0 || rev & (POLLERR | POLLHUP) != 0 {
                ready_r.push(obj);
            } else if rev & POLLPRI != 0 {
                // exceptional also counts as readable for select's rlist? CPython treats POLLPRI as both
                ready_r.push(obj);
            }
        }
        let mut ready_w = Vec::new();
        for (obj, fd) in w_vec {
            let rev = fd_to_revents.get(&fd).copied().unwrap_or(0);
            if rev & POLLOUT != 0 || rev & (POLLERR | POLLHUP) != 0 {
                ready_w.push(obj);
            }
        }
        let mut ready_x = Vec::new();
        for (obj, fd) in x_vec {
            let rev = fd_to_revents.get(&fd).copied().unwrap_or(0);
            if rev & POLLPRI != 0 || rev & POLLERR != 0 {
                ready_x.push(obj);
            }
        }

        Ok(py_tuple(vec![py_list(ready_r), py_list(ready_w), py_list(ready_x)]))
    });

    // ---- poll / devpoll factories and types (Closure so they can capture the Type) ----
    {
        let pt = poll_type_for_factory.clone();
        d.insert(
            "poll".to_string(),
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(move |args: &[PyObjectRef]| {
                if args.is_empty() {
                } else if args.len() == 1 {
                    if let PyObject::Dict(d) = &*args[0].borrow() {
                        if !d.is_empty() {
                            return Err(PyError::type_error("poll() takes no arguments"));
                        }
                    } else {
                        return Err(PyError::type_error("poll() takes no arguments"));
                    }
                } else {
                    return Err(PyError::type_error("poll() takes no arguments"));
                }
                Ok(PyObjectRef::new(PyObject::Instance { typ: pt.clone(), dict: AttrMap::new() }))
            }))),
        );
    }
    {
        let pt = devpoll_type_for_factory.clone();
        d.insert(
            "devpoll".to_string(),
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(move |args: &[PyObjectRef]| {
                if args.is_empty() {
                } else if args.len() == 1 {
                    if let PyObject::Dict(d) = &*args[0].borrow() { if !d.is_empty() { return Err(PyError::type_error("devpoll() takes no arguments")); } } else { return Err(PyError::type_error("devpoll() takes no arguments")); }
                } else { return Err(PyError::type_error("devpoll() takes no arguments")); }
                Ok(PyObjectRef::new(PyObject::Instance { typ: pt.clone(), dict: AttrMap::new() }))
            }))),
        );
    }

    // constants / error alias
    d.insert_str("PIPE_BUF", py_int(4096));
    // error is OSError alias — expose as a Type named OSError so except handlers work
    let error_type = PyObjectRef::new(PyObject::Type {
        name: "OSError".to_string(),
        dict: Box::new(str_map_to_typedict(HashMap::new())),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *error_type.borrow_mut() { *mro = vec![error_type.clone()]; }
    d.insert_str("error", error_type);

    // Also expose poll constants expected by some code (optional, not needed for tests)
    d.insert_str("POLLIN", py_int(POLLIN as i64));
    d.insert_str("POLLOUT", py_int(POLLOUT as i64));
    d.insert_str("POLLPRI", py_int(POLLPRI as i64));
    d.insert_str("POLLERR", py_int(POLLERR as i64));
    d.insert_str("POLLHUP", py_int(POLLHUP as i64));
    d.insert_str("POLLNVAL", py_int(POLLNVAL as i64));

    d
}

pub fn create_socket_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sock_func {
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

    sock_func!("socket", |args| {
        let family = if args.len() > 0 {
            args[0].as_i64().unwrap_or(2)
        } else {
            2
        };
        let _sock_type = if args.len() > 1 {
            args[1].as_i64().unwrap_or(1)
        } else {
            1
        };
        let _proto = if args.len() > 2 {
            args[2].as_i64().unwrap_or(0)
        } else {
            0
        };
        if family != 2 {
            return Err(PyError::runtime_error("Only AF_INET sockets are supported"));
        }
        Ok(PyObjectRef::new(PyObject::Socket {
            inner: std::rc::Rc::new(std::cell::RefCell::new(SocketInner::Uninitialized)),
        }))
    });

    /// Wrap a raw TcpStream as our Socket object.
    fn wrap_tcp_stream(stream: std::net::TcpStream) -> PyObjectRef {
        PyObjectRef::new(PyObject::Socket {
            inner: Rc::new(RefCell::new(SocketInner::TcpStream(stream))),
        })
    }

    // socketpair(): std has no AF_UNIX pair in this codebase's socket model
    // (TcpListener/TcpStream only), so emulate with a loopback TCP pair --
    // semantically identical for the select()/send/recv patterns tests use.
    sock_func!("socketpair", |args| {
        let _family = args.get(0).and_then(|a| a.as_i64()).unwrap_or(2);
        let _stype = args.get(1).and_then(|a| a.as_i64()).unwrap_or(1);
        let l = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| PyError::OsError(format!("socketpair bind: {}", e)))?;
        let addr = l.local_addr().map_err(|e| PyError::OsError(format!("{}", e)))?;
        let c = std::net::TcpStream::connect(addr)
            .map_err(|e| PyError::OsError(format!("socketpair connect: {}", e)))?;
        let (a_end, _l) = l.accept()
            .map_err(|e| PyError::OsError(format!("socketpair accept: {}", e)))?;
        Ok(py_list(vec![wrap_tcp_stream(a_end), wrap_tcp_stream(c)]))
    });

    // Honest: this interpreter's `socket()` only supports AF_INET (see
    // below) — reporting `has_ipv6 = True` would make `test.support.
    // socket_helper`'s own `_is_ipv6_enabled()` try `socket.socket(AF_INET6,
    // ...)`, which raises `RuntimeError` (not `OSError`, the only thing it
    // catches), crashing instead of cleanly falling back to "no IPv6".
    d.insert_str("_GLOBAL_DEFAULT_TIMEOUT", py_none());
    d.insert_str("has_ipv6", py_bool(false));
    d.insert_str("AF_INET", py_int(2));
    d.insert_str("AF_INET6", py_int(10));
    d.insert_str("SOCK_STREAM", py_int(1));
    d.insert_str("SOCK_DGRAM", py_int(2));
    d.insert_str("SOL_SOCKET", py_int(1));
    d.insert_str("SO_REUSEADDR", py_int(2));

    sock_func!("gethostname", |_| {
        match std::process::Command::new("hostname").output() {
            Ok(output) => {
                let hostname = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(py_str(&hostname))
            }
            Err(_) => Ok(py_str("localhost")),
        }
    });

    sock_func!("gethostbyname", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "gethostbyname() missing required argument",
            ));
        }
        let hostname = args[0].str();
        if hostname == "localhost" || hostname == "127.0.0.1" {
            return Ok(py_str("127.0.0.1"));
        }
        // Try DNS resolution
        match std::net::ToSocketAddrs::to_socket_addrs(&(hostname.as_str(), 0)) {
            Ok(mut addrs) => {
                if let Some(addr) = addrs.find(|a| a.is_ipv4()) {
                    Ok(py_str(&addr.ip().to_string()))
                } else {
                    Ok(py_str(&hostname))
                }
            }
            Err(_) => Ok(py_str(&hostname)),
        }
    });

    d
}

// Moved here from object.rs (was under a "---- urllib module ----" banner
// in the monolithic object.rs — see the file-splitting refactor's memory
// entry for context).
/// Create a response object for urlopen with a read() method.
/// The response body bytes are stored in the instance dict under "_body".
fn create_urlopen_response(body: Vec<u8>) -> PyObjectRef {
    use std::collections::HashMap;

    // Create the response type with a read() method
    let mut type_dict = HashMap::new();
    type_dict.insert(
        "read".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "read".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("read() missing argument"));
                }
                let body = args[0].borrow();
                if let PyObject::Instance { dict, .. } = &*body {
                    if let Some(body_val) = dict.get("_body") {
                        return Ok(body_val.clone());
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())))
            },
        }),
    );

    let resp_type = PyObjectRef::new(PyObject::Type {
        name: "HTTPResponse".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });

    let mut instance_dict = AttrMap::new();
    instance_dict.insert("_body".to_string(), PyObjectRef::imm(PyObject::Bytes(body)));
    PyObjectRef::new(PyObject::Instance {
        typ: resp_type,
        dict: instance_dict,
    })
}

pub fn create_urllib_request_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! request_func {
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

    request_func!("urlopen", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "urlopen() missing required argument 'url'",
            ));
        }
        let url_str = args[0].str();

        // Only support http:// URLs with a simple GET
        if !url_str.starts_with("http://") {
            return Err(PyError::type_error(format!(
                "urlopen() only supports http:// URLs, got: {}",
                url_str
            )));
        }

        let rest = url_str.trim_start_matches("http://");
        let (host_port, path) = match rest.find('/') {
            Some(pos) => (&rest[..pos], &rest[pos..]),
            None => (rest, "/"),
        };

        let (host, port) = if let Some(colon_pos) = host_port.find(':') {
            (
                &host_port[..colon_pos],
                host_port[colon_pos + 1..].parse::<u16>().unwrap_or(80),
            )
        } else {
            (host_port, 80u16)
        };

        if host.is_empty() {
            return Err(PyError::type_error("urlopen() invalid URL: empty host"));
        }

        // Connect via TcpStream
        let addr = format!("{}:{}", host, port);
        let stream = match std::net::TcpStream::connect(&addr) {
            Ok(s) => s,
            Err(e) => {
                return Err(PyError::runtime_error(format!(
                    "urlopen() failed to connect: {}",
                    e
                )))
            }
        };

        // Send HTTP GET request
        let request = format!(
            "GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
            path, host
        );
        {
            use std::io::Write;
            if let Err(e) = (&stream).write_all(request.as_bytes()) {
                return Err(PyError::runtime_error(format!(
                    "urlopen() write error: {}",
                    e
                )));
            }
        }

        // Read response
        let mut response = Vec::new();
        {
            use std::io::Read;
            if let Err(e) = (&stream).read_to_end(&mut response) {
                return Err(PyError::runtime_error(format!(
                    "urlopen() read error: {}",
                    e
                )));
            }
        }

        // Parse HTTP response
        let response_str = String::from_utf8_lossy(&response);
        let body = if let Some(body_start) = response_str.find("\r\n\r\n") {
            let header_end = body_start + 4;
            if header_end < response.len() {
                response[header_end..].to_vec()
            } else {
                Vec::new()
            }
        } else {
            // No headers found, return raw response as body
            response.clone()
        };

        Ok(create_urlopen_response(body))
    });

    d
}

/// Percent-encode a character (for quote)
fn percent_encode_byte(byte: u8) -> String {
    format!("%{:02X}", byte)
}

/// Percent-decode a string (for unquote). Routes through
/// [`percent_decode_to_bytes`] and re-decodes as UTF-8 — decoding a char at
/// a time (the previous approach) mangled multi-byte percent sequences:
/// `%C3%A9` must become the single character 'é', not two mojibake chars
/// from pushing each raw byte value as its own `char`.
fn percent_decode(s: &str) -> String {
    String::from_utf8_lossy(&percent_decode_to_bytes(s)).into_owned()
}

/// Check if a byte should be encoded in URL (for quote)
fn needs_percent_encode(byte: u8, safe: &str) -> bool {
    // Always safe: unreserved characters per RFC 3986
    if byte.is_ascii_alphanumeric() {
        return false;
    }
    // Also safe: these unreserved chars
    if matches!(byte, b'_' | b'-' | b'.' | b'~') {
        return false;
    }
    // Check user-provided safe chars
    if safe.as_bytes().contains(&byte) {
        return false;
    }
    true
}

/// Percent-decode into raw bytes (the primitive both `unquote`'s
/// str-returning form and `unquote_to_bytes` build on) — decoding straight
/// to `Vec<u8>` instead of `String` avoids mangling non-ASCII percent
/// sequences a char at a time (e.g. `%C3%A9` must become the single
/// UTF-8-decoded character 'é', not two separate mojibake chars).
fn percent_decode_to_bytes(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    result.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    result
}

pub fn create_urllib_parse_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! parse_func {
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

    // urlparse(url, scheme='', allow_fragments=True)
    parse_func!("urlparse", |args| {
        let url = if args.len() > 0 {
            args[0].str()
        } else {
            return Err(PyError::type_error(
                "urlparse() missing required argument 'url'",
            ));
        };
        let scheme_default = if args.len() > 1 {
            args[1].str()
        } else {
            String::new()
        };

        let mut scheme = scheme_default;
        let mut netloc = String::new();
        let mut params = String::new();
        let mut query = String::new();
        let mut fragment = String::new();
        let mut path = String::new();

        // Split fragment (allow_fragments defaults to true)
        let allow_fragments = if args.len() > 2 {
            args[2].truthy()
        } else {
            true
        };
        let remaining = if allow_fragments {
            if let Some(pos) = url.find('#') {
                fragment = url[pos + 1..].to_string();
                url[..pos].to_string()
            } else {
                url.clone()
            }
        } else {
            url.clone()
        };

        // Split query
        let remaining = if let Some(pos) = remaining.find('?') {
            query = remaining[pos + 1..].to_string();
            remaining[..pos].to_string()
        } else {
            remaining
        };

        // Extract scheme
        if let Some(pos) = remaining.find("://") {
            scheme = remaining[..pos].to_string();
            let after_scheme = &remaining[pos + 3..];
            // Extract netloc (host:port or host)
            if let Some(slash_pos) = after_scheme.find('/') {
                netloc = after_scheme[..slash_pos].to_string();
                path = after_scheme[slash_pos..].to_string();
            } else {
                netloc = after_scheme.to_string();
            }
        } else {
            path = remaining;
        }

        // Split params from path (last semicolon in path segment)
        if let Some(pos) = path.rfind(';') {
            params = path[pos + 1..].to_string();
            path = path[..pos].to_string();
        }

        // Create result type with scheme, netloc, path, params, query, fragment attributes
        let type_dict = HashMap::new();
        let parse_type = PyObjectRef::new(PyObject::Type {
            name: "ParseResult".to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        });

        let mut instance_dict = AttrMap::new();
        instance_dict.insert("scheme".to_string(), py_str(&scheme));
        instance_dict.insert("netloc".to_string(), py_str(&netloc));
        instance_dict.insert("path".to_string(), py_str(&path));
        instance_dict.insert("params".to_string(), py_str(&params));
        instance_dict.insert("query".to_string(), py_str(&query));
        instance_dict.insert("fragment".to_string(), py_str(&fragment));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: parse_type,
            dict: instance_dict,
        }))
    });

    // urlencode(query, doseq=False)
    parse_func!("urlencode", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "urlencode() missing required argument 'query'",
            ));
        }
        let _doseq = if args.len() > 1 {
            args[1].truthy()
        } else {
            false
        };

        let obj = args[0].borrow();
        let mut pairs: Vec<(String, String)> = Vec::new();

        match &*obj {
            PyObject::Dict(dict) => {
                for (k, v) in dict.items() {
                    let key = k.str();
                    let val = v.str();
                    pairs.push((key, val));
                }
            }
            PyObject::List(items) | PyObject::Tuple(items) => {
                for item in items {
                    let item_ref = item.borrow();
                    if let PyObject::Tuple(pair) = &*item_ref {
                        if pair.len() >= 2 {
                            let key = pair[0].str();
                            let val = pair[1].str();
                            pairs.push((key, val));
                        }
                    } else if let PyObject::List(pair) = &*item_ref {
                        if pair.len() >= 2 {
                            let key = pair[0].str();
                            let val = pair[1].str();
                            pairs.push((key, val));
                        }
                    } else {
                        // Try to iterate
                        let key = item.str();
                        pairs.push((key, String::new()));
                    }
                }
            }
            _ => {
                return Err(PyError::type_error(
                    "urlencode() argument must be dict, list of tuples, or list of lists",
                ));
            }
        }

        // Percent-encode both keys and values
        let encoded: Vec<String> = pairs
            .into_iter()
            .map(|(k, v)| {
                let enc_key: String = k
                    .bytes()
                    .map(|b| {
                        if needs_percent_encode(b, "") {
                            percent_encode_byte(b)
                        } else {
                            (b as char).to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .concat();
                let enc_val: String = v
                    .bytes()
                    .map(|b| {
                        if needs_percent_encode(b, "") {
                            percent_encode_byte(b)
                        } else {
                            (b as char).to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .concat();
                format!("{}={}", enc_key, enc_val)
            })
            .collect();

        Ok(py_str(&encoded.join("&")))
    });

    // quote(string, safe='/')
    parse_func!("quote", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "quote() missing required argument 'string'",
            ));
        }
        let s = args[0].str();
        let safe = if args.len() > 1 {
            args[1].str()
        } else {
            "/".to_string()
        };

        let encoded: String = s
            .bytes()
            .map(|b| {
                if needs_percent_encode(b, &safe) {
                    percent_encode_byte(b)
                } else {
                    (b as char).to_string()
                }
            })
            .collect::<Vec<_>>()
            .concat();

        Ok(py_str(&encoded))
    });

    // unquote(string, encoding='utf-8', errors='replace')
    parse_func!("unquote", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "unquote() missing required argument 'string'",
            ));
        }
        let s = args[0].str();
        Ok(py_str(&percent_decode(&s)))
    });

    d
}

pub fn create_urllib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "request",
        create_module("urllib.request", create_urllib_request_dict()),
    );
    d.insert_str(
        "parse",
        create_module("urllib.parse", create_urllib_parse_dict()),
    );
    d
}
