use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::rc::Rc;

pub fn create_http_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Create HTTPStatus type with string status constants
    let mut status_dict = HashMap::new();
    status_dict.insert_str("OK", py_str("200 OK"));
    status_dict.insert_str("NOT_FOUND", py_str("404 NOT_FOUND"));
    status_dict.insert_str("SERVER_ERROR", py_str("500 Internal Server Error"));

    let http_status_type = PyObjectRef::new(PyObject::Type {
        name: "HTTPStatus".to_string(),
        dict: Box::new(str_map_to_typedict(status_dict)),
        bases: vec![],
        mro: vec![],
    });

    d.insert_str("HTTPStatus", http_status_type);
    d
}

// ---------------------------------------------------------------------------
// http.client module - HTTPConnection class
// ---------------------------------------------------------------------------

/// Standalone read() method for HTTPResponse instances.
/// `args[0]` is the HTTPResponse instance (auto-bound by BuiltinMethod).
fn http_response_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "read() missing required 'self' argument",
        ));
    }
    let borrowed = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*borrowed {
        if let Some(body) = dict.get_str("_body") {
            return Ok(body.clone());
        }
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(vec![])))
}

pub fn create_http_client_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Real CPython's `http.client.HTTP_PORT`/`HTTPS_PORT` — plain integer
    // constants, missing entirely. Real trigger: CPython's own
    // `http/cookiejar.py`, `from http.client import HTTP_PORT`.
    d.insert_str("HTTP_PORT", py_int(80));
    d.insert_str("HTTPS_PORT", py_int(443));

    // Minimal exception hierarchy — real code commonly catches
    // `http.client.HTTPException` (or a specific subclass) around request
    // handling. Plain marker classes (no custom `__init__`/state) are
    // enough for `except HTTPException:`/`isinstance` purposes.
    fn make_http_exc(name: &str, base: Option<PyObjectRef>) -> PyObjectRef {
        let bases = base.map(|b| vec![b]).unwrap_or_default();
        PyObjectRef::new(crate::object::PyObject::Type {
            name: name.to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: bases.clone(),
            mro: bases,
        })
    }
    let http_exception = make_http_exc("HTTPException", None);
    d.insert_str("HTTPException", http_exception.clone());
    for name in [
        "NotConnected",
        "InvalidURL",
        "UnknownProtocol",
        "UnknownTransferEncoding",
        "UnimplementedFileMode",
        "IncompleteRead",
        "ImproperConnectionState",
        "CannotSendRequest",
        "CannotSendHeader",
        "ResponseNotReady",
        "BadStatusLine",
        "LineTooLong",
        "RemoteDisconnected",
    ] {
        d.insert(
            name.to_string(),
            make_http_exc(name, Some(http_exception.clone())),
        );
    }

    // HTTP status code to phrase mapping
    let responses = crate::object::py_dict();
    if let crate::object::PyObject::Dict(ref mut resp_dict) = &mut *responses.borrow_mut() {
        let codes = [
            (200, "OK"),
            (201, "Created"),
            (202, "Accepted"),
            (204, "No Content"),
            (301, "Moved Permanently"),
            (302, "Found"),
            (303, "See Other"),
            (304, "Not Modified"),
            (307, "Temporary Redirect"),
            (400, "Bad Request"),
            (401, "Unauthorized"),
            (403, "Forbidden"),
            (404, "Not Found"),
            (405, "Method Not Allowed"),
            (408, "Request Timeout"),
            (418, "I'm a Teapot"),
            (429, "Too Many Requests"),
            (500, "Internal Server Error"),
            (502, "Bad Gateway"),
            (503, "Service Unavailable"),
            (504, "Gateway Timeout"),
        ];
        for (code, phrase) in &codes {
            let _ = resp_dict.set(crate::object::py_int(*code), crate::object::py_str(phrase));
        }
    }
    d.insert_str("responses", responses);

    // ---- HTTPMessage type (minimal header container) ----
    // CPython's HTTPMessage inherits from email.message.Message; we provide
    // a minimal standalone type with the interface code actually uses.
    {
        let mut msg_dict = HashMap::new();
        // __init__(self, headers=None)
        msg_dict.insert(
            "__init__".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error(
                            "__init__() missing 1 required positional argument: 'self'",
                        ));
                    }
                    let self_obj = &args[0];
                    let headers = if args.len() > 1 {
                        args[1].clone()
                    } else {
                        py_list(vec![])
                    };
                    if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                        dict.insert_str("_headers", headers);
                    }
                    Ok(py_none())
                },
            }),
        );
        // getallmatchingheaders(self, name)
        msg_dict.insert(
            "getallmatchingheaders".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getallmatchingheaders".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error(
                            "getallmatchingheaders() missing 2 required positional arguments: 'self' and 'name'",
                        ));
                    }
                    let self_obj = &args[0];
                    let name = args[1].str().to_lowercase();
                    let borrowed = self_obj.borrow();
                    let headers = if let PyObject::Instance { dict, .. } = &*borrowed {
                        dict.get_str("_headers").cloned().unwrap_or_else(|| py_list(vec![]))
                    } else {
                        py_list(vec![])
                    };
                    let mut result = vec![];
                    if let PyObject::List(items) = &*headers.borrow() {
                        for item in items {
                            if let PyObject::Tuple(pair) = &*item.borrow() {
                                if pair.len() >= 2 {
                                    let hname = pair[0].str().to_lowercase();
                                    if hname == name {
                                        result.push(item.clone());
                                    }
                                }
                            }
                        }
                    }
                    Ok(py_list(result))
                },
            }),
        );
        // __repr__(self)
        msg_dict.insert(
            "__repr__".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error(
                            "__repr__() missing 1 required positional argument: 'self'",
                        ));
                    }
                    Ok(py_str("<http.client.HTTPMessage>"))
                },
            }),
        );
        // parse_header_lines(header_lines, _class=HTTPMessage) — module-level helper
        d.insert(
            "parse_headers".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "parse_headers".to_string(),
                func: |_args| {
                    // parse_headers(fp, _class=HTTPMessage)
                    // Stub: return an empty HTTPMessage instance
                    Ok(py_none())
                },
            }),
        );
        let http_msg_type = PyObjectRef::new(PyObject::Type {
            name: "HTTPMessage".to_string(),
            dict: Box::new(str_map_to_typedict(msg_dict)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str("HTTPMessage", http_msg_type);
    }

    // ---- HTTPResponse type ----
    let mut resp_dict = HashMap::new();
    resp_dict.insert(
        "read".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "read".to_string(),
            func: http_response_read,
        }),
    );
    let http_resp_type = PyObjectRef::new(PyObject::Type {
        name: "HTTPResponse".to_string(),
        dict: Box::new(str_map_to_typedict(resp_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("HTTPResponse", http_resp_type.clone());

    // ---- HTTPConnection class ----
    let mut conn_dict = HashMap::new();

    // __init__(self, host, port=80)
    conn_dict.insert(
        "__init__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "HTTPConnection() missing 1 required positional argument: 'host'",
                    ));
                }
                let self_obj = &args[0];
                let host = args[1].str();
                let port = if args.len() > 2 {
                    args[2].as_i64().unwrap_or(80) as u16
                } else {
                    80u16
                };
                if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                    dict.insert_str("_host", py_str(&host));
                    dict.insert_str("_port", py_int(port as i64));
                }
                Ok(py_none())
            },
        }),
    );

    // request(self, method, url, body=None, headers=None)
    conn_dict.insert(
        "request".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "request".to_string(),
            func: |args| {
                if args.len() < 3 {
                    return Err(PyError::type_error(
                        "request() missing 2 required positional arguments: 'method' and 'url'",
                    ));
                }
                let self_obj = &args[0];
                let method = args[1].str();
                let url = args[2].str();

                // Extract body (optional, arg 3)
                let body = if args.len() > 3 {
                    let b = &args[3];
                    let b_borrowed = b.borrow();
                    match &*b_borrowed {
                        PyObject::Bytes(bytes) => Some(bytes.clone()),
                        PyObject::None => None,
                        _ => Some(b.str().into_bytes()),
                    }
                } else {
                    None
                };

                // Extract headers (optional, arg 4) - passed as PyDict
                let headers: HashMap<String, String> = if args.len() > 4 {
                    let h = &args[4];
                    let h_borrowed = h.borrow();
                    let mut result = HashMap::new();
                    if let PyObject::Dict(pydict) = &*h_borrowed {
                        for (k, v) in pydict.items() {
                            result.insert(k.str(), v.str());
                        }
                    }
                    result
                } else {
                    HashMap::new()
                };

                // Read host and port from instance dict
                let (host, port) = {
                    let borrowed = self_obj.borrow();
                    if let PyObject::Instance { dict, .. } = &*borrowed {
                        let host = dict
                            .get("_host")
                            .map(|h| h.str())
                            .unwrap_or_else(|| "localhost".to_string());
                        let port = dict.get("_port").and_then(|p| p.as_i64()).unwrap_or(80) as u16;
                        (host, port)
                    } else {
                        return Err(PyError::runtime_error("invalid HTTPConnection instance"));
                    }
                };

                // Connect via TcpStream with a bounded timeout — an
                // unreachable/firewalled host (common in sandboxed/offline
                // environments) can otherwise leave a bare `TcpStream::connect`
                // blocking for the OS's own connect timeout (which may be
                // minutes, or effectively indefinite if packets are silently
                // dropped), hanging the whole interpreter with no way for
                // Python-level code to recover.
                let addr = format!("{}:{}", host, port);
                let stream = {
                    let mut last_err: Option<std::io::Error> = None;
                    let mut connected = None;
                    match addr.to_socket_addrs() {
                        Ok(addrs) => {
                            for sock_addr in addrs {
                                match TcpStream::connect_timeout(
                                    &sock_addr,
                                    std::time::Duration::from_secs(10),
                                ) {
                                    Ok(s) => {
                                        connected = Some(s);
                                        break;
                                    }
                                    Err(e) => {
                                        last_err = Some(e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            last_err = Some(e);
                        }
                    }
                    match connected {
                        Some(s) => s,
                        None => {
                            return Err(PyError::OsError(format!(
                                "Could not connect to {}: {}",
                                addr,
                                last_err
                                    .map(|e| e.to_string())
                                    .unwrap_or_else(|| "unknown error".to_string())
                            )));
                        }
                    }
                };

                // Build HTTP request path
                let path = if url.starts_with("http://") || url.starts_with("https://") {
                    let after_proto = if url.starts_with("https://") {
                        &url[8..]
                    } else {
                        &url[7..]
                    };
                    if let Some(slash_pos) = after_proto.find('/') {
                        &after_proto[slash_pos..]
                    } else {
                        "/"
                    }
                } else {
                    url.as_str()
                };

                let mut request = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", method, path, host);
                for (k, v) in &headers {
                    request.push_str(&format!("{}: {}\r\n", k, v));
                }
                if let Some(ref body_bytes) = body {
                    request.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
                }
                request.push_str("Connection: close\r\n\r\n");

                let mut full_request = request.into_bytes();
                if let Some(ref body_bytes) = body {
                    full_request.extend_from_slice(body_bytes);
                }

                // Send request
                if let Err(e) = (&stream).write_all(&full_request) {
                    return Err(PyError::OsError(format!("Failed to send request: {}", e)));
                }

                // Store stream in instance dict as a Socket object
                let sock = PyObjectRef::new(PyObject::Socket {
                    inner: Rc::new(RefCell::new(SocketInner::TcpStream(stream))),
                });
                if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                    dict.insert_str("_stream", sock);
                }

                Ok(py_none())
            },
        }),
    );

    // getresponse(self) -> HTTPResponse
    conn_dict.insert(
        "getresponse".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getresponse".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "getresponse() missing required 'self' argument",
                    ));
                }
                let self_obj = &args[0];

                // Take the Socket out of the instance dict
                let sock = {
                    let mut borrowed = self_obj.borrow_mut();
                    if let PyObject::Instance { dict, .. } = &mut *borrowed {
                        dict.remove("_stream").ok_or_else(|| {
                            PyError::runtime_error("no request made yet - call request() first")
                        })?
                    } else {
                        return Err(PyError::runtime_error("invalid HTTPConnection instance"));
                    }
                };

                // Extract TcpStream from Socket via try_clone
                let mut stream = {
                    let sock_borrowed = sock.borrow();
                    if let PyObject::Socket { inner } = &*sock_borrowed {
                        let inner_borrowed = inner.borrow();
                        match &*inner_borrowed {
                            SocketInner::TcpStream(s) => s.try_clone().map_err(|e| {
                                PyError::OsError(format!("Failed to clone stream: {}", e))
                            })?,
                            _ => {
                                return Err(PyError::runtime_error("no active HTTP connection"));
                            }
                        }
                    } else {
                        return Err(PyError::runtime_error(
                            "internal error: stream socket not found",
                        ));
                    }
                };

                // Read response status line
                use std::io::BufRead;
                let mut reader = std::io::BufReader::new(&mut stream);
                let mut status_line = String::new();
                if reader
                    .read_line(&mut status_line)
                    .map_err(|e| PyError::OsError(format!("Failed to read response: {}", e)))?
                    == 0
                {
                    return Err(PyError::runtime_error("connection closed"));
                }

                let status_line = status_line.trim();
                let status_code: i64 = status_line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                // Skip headers
                loop {
                    let mut line = String::new();
                    if reader
                        .read_line(&mut line)
                        .map_err(|e| PyError::OsError(format!("Failed to read header: {}", e)))?
                        == 0
                    {
                        break;
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                }

                // Read body (rest of stream)
                let mut body = Vec::new();
                reader
                    .read_to_end(&mut body)
                    .map_err(|e| PyError::OsError(format!("Failed to read body: {}", e)))?;

                // Build fresh HTTPResponse type (no captures for fn pointer)
                let local_resp_type = PyObjectRef::new(PyObject::Type {
                    name: "HTTPResponse".to_string(),
                    dict: {
                        let mut rd = HashMap::new();
                        rd.insert(
                            "read".to_string(),
                            PyObjectRef::new(PyObject::BuiltinFunction {
                                name: "read".to_string(),
                                func: http_response_read,
                            }),
                        );
                        Box::new(str_map_to_typedict(rd))
                    },
                    bases: vec![],
                    mro: vec![],
                });

                // Build HTTPResponse instance
                let mut inst_dict = AttrMap::new();
                inst_dict.insert_str("status", py_int(status_code));
                inst_dict.insert_str("_body", PyObjectRef::imm(PyObject::Bytes(body)));

                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: local_resp_type,
                    dict: inst_dict,
                }))
            },
        }),
    );

    // close(self)
    conn_dict.insert(
        "close".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: |args| {
                let self_obj = &args[0];
                if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                    let _ = dict.remove("_stream");
                }
                Ok(py_none())
            },
        }),
    );

    let http_conn_type = PyObjectRef::new(PyObject::Type {
        name: "HTTPConnection".to_string(),
        dict: Box::new(str_map_to_typedict(conn_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("HTTPConnection", http_conn_type);

    d
}

// ---------------------------------------------------------------------------
// smtplib module - SMTP class (stub)
// ---------------------------------------------------------------------------

pub fn create_smtplib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    let mut smtp_dict = HashMap::new();

    // __init__(self, host, port=25)
    smtp_dict.insert(
        "__init__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "SMTP() missing 1 required positional argument: 'host'",
                    ));
                }
                let self_obj = &args[0];
                let host = args[1].str();
                let port = if args.len() > 2 {
                    args[2].as_i64().unwrap_or(25) as u16
                } else {
                    25u16
                };
                if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                    dict.insert_str("_host", py_str(&host));
                    dict.insert_str("_port", py_int(port as i64));
                }
                Ok(py_none())
            },
        }),
    );

    // sendmail(self, from_addr, to_addrs, msg) -> {} (stub)
    smtp_dict.insert(
        "sendmail".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "sendmail".to_string(),
            func: |_args| Ok(py_dict()),
        }),
    );

    // quit(self) -> None (stub)
    smtp_dict.insert(
        "quit".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "quit".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    let smtp_type = PyObjectRef::new(PyObject::Type {
        name: "SMTP".to_string(),
        dict: Box::new(str_map_to_typedict(smtp_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("SMTP", smtp_type);

    d
}
