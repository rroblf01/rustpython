use crate::object::*;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;

pub fn create_select_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sel_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    sel_func!("select", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("select() takes at least 3 arguments"));
        }
        let rlist = &args[0];
        let _wlist = &args[1];
        let _xlist = &args[2];
        let mut readable = Vec::new();
        let rlist_b = rlist.borrow();
        if let PyObject::List(items) = &*rlist_b {
            for item in items {
                readable.push(item.clone());
            }
        }
        Ok(py_tuple(vec![py_list(readable), py_list(vec![]), py_list(vec![])]))
    });

    d
}

pub fn create_socket_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sock_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    sock_func!("socket", |args| {
        let family = if args.len() > 0 { args[0].as_i64().unwrap_or(2) } else { 2 };
        let _sock_type = if args.len() > 1 { args[1].as_i64().unwrap_or(1) } else { 1 };
        let _proto = if args.len() > 2 { args[2].as_i64().unwrap_or(0) } else { 0 };
        if family != 2 {
            return Err(PyError::runtime_error("Only AF_INET sockets are supported"));
        }
        Ok(PyObjectRef::new(PyObject::Socket {
            inner: std::rc::Rc::new(std::cell::RefCell::new(SocketInner::Uninitialized)),
        }))
    });

    d.insert("AF_INET".to_string(), py_int(2));
    d.insert("SOCK_STREAM".to_string(), py_int(1));
    d.insert("SOCK_DGRAM".to_string(), py_int(2));
    d.insert("SOL_SOCKET".to_string(), py_int(1));
    d.insert("SO_REUSEADDR".to_string(), py_int(2));

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
            return Err(PyError::type_error("gethostbyname() missing required argument"));
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

pub fn create_subprocess_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sub_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    sub_func!("run", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("run() missing required argument"));
        }
        let shell = if args.len() > 1 { args[1].truthy() } else { false };
        let cmd_str = args[0].str();
        if cmd_str.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let output = if shell {
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&cmd_str)
                .output()
                .map_err(|e| PyError::OsError(format!("{}", e)))?
        } else {
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                vec![cmd_str]
            };
            if cmd_args.is_empty() {
                return Err(PyError::ValueError("empty command".to_string()));
            }
            std::process::Command::new(&cmd_args[0])
                .args(&cmd_args[1..])
                .output()
                .map_err(|e| PyError::OsError(format!("{}", e)))?
        };
        let returncode = output.status.code().unwrap_or(-1) as i64;
        let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
        let result = py_dict();
        if let PyObject::Dict(dict) = &mut *result.borrow_mut() {
            dict.set(py_str("returncode"), py_int(returncode)).ok();
            dict.set(py_str("stdout"), py_str(&stdout_str)).ok();
            dict.set(py_str("stderr"), py_str(&stderr_str)).ok();
        }
        Ok(result)
    });

    sub_func!("check_output", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("check_output() missing required argument"));
        }
        let shell = if args.len() > 1 { args[1].truthy() } else { false };
        let cmd_str = args[0].str();
        if cmd_str.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let output = if shell {
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&cmd_str)
                .output()
                .map_err(|e| PyError::OsError(format!("{}", e)))?
        } else {
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                vec![cmd_str]
            };
            if cmd_args.is_empty() {
                return Err(PyError::ValueError("empty command".to_string()));
            }
            std::process::Command::new(&cmd_args[0])
                .args(&cmd_args[1..])
                .output()
                .map_err(|e| PyError::OsError(format!("{}", e)))?
        };
        if !output.status.success() {
            return Err(PyError::runtime_error(format!("Command returned non-zero exit status")));
        }
        // Return stdout as bytes
        Ok(PyObjectRef::imm(PyObject::Bytes(output.stdout)))
    });

    // Constants
    d.insert("PIPE".to_string(), py_int(-1));

    d
}

pub fn create_http_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Create HTTPStatus type with string status constants
    let mut status_dict = HashMap::new();
    status_dict.insert("OK".to_string(), py_str("200 OK"));
    status_dict.insert("NOT_FOUND".to_string(), py_str("404 NOT_FOUND"));
    status_dict.insert("SERVER_ERROR".to_string(), py_str("500 Internal Server Error"));

    let http_status_type = PyObjectRef::new(PyObject::Type {
        name: "HTTPStatus".to_string(),
        dict: status_dict,
        bases: vec![],
        mro: vec![],
    });

    d.insert("HTTPStatus".to_string(), http_status_type);
    d
}

pub fn create_html_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! html_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    html_func!("escape", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("escape() missing required argument"));
        }
        let s = args[0].str();
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => result.push_str("&amp;"),
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                '"' => result.push_str("&quot;"),
                '\'' => result.push_str("&#x27;"),
                _ => result.push(c),
            }
        }
        Ok(py_str(&result))
    });

    html_func!("unescape", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unescape() missing required argument"));
        }
        let s = args[0].str();
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        let mut result = String::with_capacity(s.len());
        let mut i = 0;
        while i < len {
            if chars[i] == '&' {
                // Find the closing semicolon
                if let Some(end) = chars[i..].iter().position(|&c| c == ';') {
                    let entity: String = chars[i+1..i+end].iter().collect();
                    let decoded: Option<String> = match entity.as_str() {
                        "amp" => Some("&".to_string()),
                        "lt" => Some("<".to_string()),
                        "gt" => Some(">".to_string()),
                        "quot" => Some("\"".to_string()),
                        "#x27" | "#39" => Some("'".to_string()),
                        "nbsp" => Some("\u{00A0}".to_string()),
                        _ => {
                            // Try numeric character reference
                            if entity.starts_with('#') {
                                let codepoint: Option<u32> = if entity.starts_with("#x") || entity.starts_with("#X") {
                                    u32::from_str_radix(&entity[2..], 16).ok()
                                } else {
                                    entity[1..].parse().ok()
                                };
                                codepoint.and_then(|cp| char::from_u32(cp)).map(|c| c.to_string())
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(replacement) = decoded {
                        result.push_str(&replacement);
                        i += end + 1;
                        continue;
                    }
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        Ok(py_str(&result))
    });

    d
}

pub fn create_html_entities_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Build the html5 dict of entity name -> character
    let pairs: &[(&str, &str)] = &[
        ("amp", "&"), ("lt", "<"), ("gt", ">"), ("quot", "\""), ("apos", "'"),
        ("nbsp", "\u{00A0}"), ("iexcl", "\u{00A1}"), ("cent", "\u{00A2}"),
        ("pound", "\u{00A3}"), ("curren", "\u{00A4}"), ("yen", "\u{00A5}"),
        ("brvbar", "\u{00A6}"), ("sect", "\u{00A7}"), ("uml", "\u{00A8}"),
        ("copy", "\u{00A9}"), ("ordf", "\u{00AA}"), ("laquo", "\u{00AB}"),
        ("not", "\u{00AC}"), ("shy", "\u{00AD}"), ("reg", "\u{00AE}"),
        ("macr", "\u{00AF}"), ("deg", "\u{00B0}"), ("plusmn", "\u{00B1}"),
        ("sup2", "\u{00B2}"), ("sup3", "\u{00B3}"), ("acute", "\u{00B4}"),
        ("micro", "\u{00B5}"), ("para", "\u{00B6}"), ("middot", "\u{00B7}"),
        ("cedil", "\u{00B8}"), ("sup1", "\u{00B9}"), ("ordm", "\u{00BA}"),
        ("raquo", "\u{00BB}"), ("frac14", "\u{00BC}"), ("frac12", "\u{00BD}"),
        ("frac34", "\u{00BE}"), ("iquest", "\u{00BF}"), ("times", "\u{00D7}"),
        ("divide", "\u{00F7}"), ("OElig", "\u{0152}"), ("oelig", "\u{0153}"),
        ("Scaron", "\u{0160}"), ("scaron", "\u{0161}"), ("Yuml", "\u{0178}"),
        ("fnof", "\u{0192}"), ("circ", "\u{02C6}"), ("tilde", "\u{02DC}"),
        ("Alpha", "\u{0391}"), ("Beta", "\u{0392}"), ("Gamma", "\u{0393}"),
        ("Delta", "\u{0394}"), ("Epsilon", "\u{0395}"), ("Zeta", "\u{0396}"),
        ("Eta", "\u{0397}"), ("Theta", "\u{0398}"), ("Iota", "\u{0399}"),
        ("Kappa", "\u{039A}"), ("Lambda", "\u{039B}"), ("Mu", "\u{039C}"),
        ("Nu", "\u{039D}"), ("Xi", "\u{039E}"), ("Omicron", "\u{039F}"),
        ("Pi", "\u{03A0}"), ("Rho", "\u{03A1}"), ("Sigma", "\u{03A3}"),
        ("Tau", "\u{03A4}"), ("Upsilon", "\u{03A5}"), ("Phi", "\u{03A6}"),
        ("Chi", "\u{03A7}"), ("Psi", "\u{03A8}"), ("Omega", "\u{03A9}"),
        ("alpha", "\u{03B1}"), ("beta", "\u{03B2}"), ("gamma", "\u{03B3}"),
        ("delta", "\u{03B4}"), ("epsilon", "\u{03B5}"), ("zeta", "\u{03B6}"),
        ("eta", "\u{03B7}"), ("theta", "\u{03B8}"), ("iota", "\u{03B9}"),
        ("kappa", "\u{03BA}"), ("lambda", "\u{03BB}"), ("mu", "\u{03BC}"),
        ("nu", "\u{03BD}"), ("xi", "\u{03BE}"), ("omicron", "\u{03BF}"),
        ("pi", "\u{03C0}"), ("rho", "\u{03C1}"), ("sigmaf", "\u{03C2}"),
        ("sigma", "\u{03C3}"), ("tau", "\u{03C4}"), ("upsilon", "\u{03C5}"),
        ("phi", "\u{03C6}"), ("chi", "\u{03C7}"), ("psi", "\u{03C8}"),
        ("omega", "\u{03C9}"), ("thetasym", "\u{03D1}"), ("upsih", "\u{03D2}"),
        ("piv", "\u{03D6}"), ("ensp", "\u{2002}"), ("emsp", "\u{2003}"),
        ("thinsp", "\u{2009}"), ("zwnj", "\u{200C}"), ("zwj", "\u{200D}"),
        ("lrm", "\u{200E}"), ("rlm", "\u{200F}"), ("ndash", "\u{2013}"),
        ("mdash", "\u{2014}"), ("lsquo", "\u{2018}"), ("rsquo", "\u{2019}"),
        ("sbquo", "\u{201A}"), ("ldquo", "\u{201C}"), ("rdquo", "\u{201D}"),
        ("bdquo", "\u{201E}"), ("dagger", "\u{2020}"), ("Dagger", "\u{2021}"),
        ("bull", "\u{2022}"), ("hellip", "\u{2026}"), ("permil", "\u{2030}"),
        ("prime", "\u{2032}"), ("Prime", "\u{2033}"), ("lsaquo", "\u{2039}"),
        ("rsaquo", "\u{203A}"), ("oline", "\u{203E}"), ("frasl", "\u{2044}"),
        ("euro", "\u{20AC}"), ("image", "\u{2111}"), ("weierp", "\u{2118}"),
        ("real", "\u{211C}"), ("trade", "\u{2122}"), ("alefsym", "\u{2135}"),
        ("larr", "\u{2190}"), ("uarr", "\u{2191}"), ("rarr", "\u{2192}"),
        ("darr", "\u{2193}"), ("harr", "\u{2194}"), ("crarr", "\u{21B5}"),
        ("lArr", "\u{21D0}"), ("uArr", "\u{21D1}"), ("rArr", "\u{21D2}"),
        ("dArr", "\u{21D3}"), ("hArr", "\u{21D4}"), ("forall", "\u{2200}"),
        ("part", "\u{2202}"), ("exist", "\u{2203}"), ("empty", "\u{2205}"),
        ("nabla", "\u{2207}"), ("isin", "\u{2208}"), ("notin", "\u{2209}"),
        ("ni", "\u{220B}"), ("prod", "\u{220F}"), ("sum", "\u{2211}"),
        ("minus", "\u{2212}"), ("lowast", "\u{2217}"), ("radic", "\u{221A}"),
        ("prop", "\u{221D}"), ("infin", "\u{221E}"), ("ang", "\u{2220}"),
        ("and", "\u{2227}"), ("or", "\u{2228}"), ("cap", "\u{2229}"),
        ("cup", "\u{222A}"), ("int", "\u{222B}"), ("there4", "\u{2234}"),
        ("sim", "\u{223C}"), ("cong", "\u{2245}"), ("asymp", "\u{2248}"),
        ("ne", "\u{2260}"), ("equiv", "\u{2261}"), ("le", "\u{2264}"),
        ("ge", "\u{2265}"), ("sub", "\u{2282}"), ("sup", "\u{2283}"),
        ("nsub", "\u{2284}"), ("sube", "\u{2286}"), ("supe", "\u{2287}"),
        ("oplus", "\u{2295}"), ("otimes", "\u{2297}"), ("perp", "\u{22A5}"),
        ("sdot", "\u{22C5}"), ("lceil", "\u{2308}"), ("rceil", "\u{2309}"),
        ("lfloor", "\u{230A}"), ("rfloor", "\u{230B}"), ("lang", "\u{2329}"),
        ("rang", "\u{232A}"), ("loz", "\u{25CA}"), ("spades", "\u{2660}"),
        ("clubs", "\u{2663}"), ("hearts", "\u{2665}"), ("diams", "\u{2666}"),
    ];

    let mut py_dict_obj = py_dict();
    if let PyObject::Dict(ref mut pd) = &mut *py_dict_obj.borrow_mut() {
        for (name, ch) in pairs {
            pd.set(py_str(name), py_str(ch)).ok();
        }
    }

    d.insert("html5".to_string(), py_dict_obj);
    d
}

pub fn create_urllib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("request".to_string(), create_module("urllib.request", create_urllib_request_dict()));
    d.insert("parse".to_string(), create_module("urllib.parse", create_urllib_parse_dict()));
    d
}

use std::rc::Rc;
use std::cell::RefCell;
use std::process::Command;

// ---------------------------------------------------------------------------
// http.client module - HTTPConnection class
// ---------------------------------------------------------------------------

/// Standalone read() method for HTTPResponse instances.
/// `args[0]` is the HTTPResponse instance (auto-bound by BuiltinMethod).
fn http_response_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("read() missing required 'self' argument"));
    }
    let borrowed = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*borrowed {
        if let Some(body) = dict.get("_body") {
            return Ok(body.clone());
        }
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(vec![])))
}

pub fn create_http_client_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // HTTP status code to phrase mapping
    let mut responses = crate::object::py_dict();
    if let crate::object::PyObject::Dict(ref mut resp_dict) = &mut *responses.borrow_mut() {
        let codes = [
            (200, "OK"), (201, "Created"), (202, "Accepted"),
            (204, "No Content"), (301, "Moved Permanently"),
            (302, "Found"), (303, "See Other"),
            (304, "Not Modified"), (307, "Temporary Redirect"),
            (400, "Bad Request"), (401, "Unauthorized"),
            (403, "Forbidden"), (404, "Not Found"),
            (405, "Method Not Allowed"), (408, "Request Timeout"),
            (418, "I'm a Teapot"), (429, "Too Many Requests"),
            (500, "Internal Server Error"), (502, "Bad Gateway"),
            (503, "Service Unavailable"), (504, "Gateway Timeout"),
        ];
        for (code, phrase) in &codes {
            let _ = resp_dict.set(crate::object::py_int(*code), crate::object::py_str(phrase));
        }
    }
    d.insert("responses".to_string(), responses);

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
        dict: resp_dict,
        bases: vec![],
        mro: vec![],
    });
    d.insert("HTTPResponse".to_string(), http_resp_type.clone());

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
                    dict.insert("_host".to_string(), py_str(&host));
                    dict.insert("_port".to_string(), py_int(port as i64));
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
                        let port = dict
                            .get("_port")
                            .and_then(|p| p.as_i64())
                            .unwrap_or(80) as u16;
                        (host, port)
                    } else {
                        return Err(PyError::runtime_error("invalid HTTPConnection instance"));
                    }
                };

                // Connect via TcpStream
                let addr = format!("{}:{}", host, port);
                let stream = match TcpStream::connect(&addr) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(PyError::OsError(format!(
                            "Could not connect to {}: {}",
                            addr, e
                        )));
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

                let mut request = format!(
                    "{} {} HTTP/1.1\r\nHost: {}\r\n",
                    method, path, host
                );
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
                    dict.insert("_stream".to_string(), sock);
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
                        dict.remove("_stream")
                            .ok_or_else(|| {
                                PyError::runtime_error(
                                    "no request made yet - call request() first",
                                )
                            })?
                    } else {
                        return Err(PyError::runtime_error(
                            "invalid HTTPConnection instance",
                        ));
                    }
                };

                // Extract TcpStream from Socket via try_clone
                let mut stream = {
                    let sock_borrowed = sock.borrow();
                    if let PyObject::Socket { inner } = &*sock_borrowed {
                        let inner_borrowed = inner.borrow();
                        match &*inner_borrowed {
                            SocketInner::TcpStream(s) => s
                                .try_clone()
                                .map_err(|e| {
                                    PyError::OsError(format!("Failed to clone stream: {}", e))
                                })?,
                            _ => {
                                return Err(PyError::runtime_error(
                                    "no active HTTP connection",
                                ));
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
                        rd
                    },
                    bases: vec![],
                    mro: vec![],
                });

                // Build HTTPResponse instance
                let mut inst_dict = HashMap::new();
                inst_dict.insert("status".to_string(), py_int(status_code));
                inst_dict.insert("_body".to_string(), PyObjectRef::imm(PyObject::Bytes(body)));

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
        dict: conn_dict,
        bases: vec![],
        mro: vec![],
    });
    d.insert("HTTPConnection".to_string(), http_conn_type);

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
                    dict.insert("_host".to_string(), py_str(&host));
                    dict.insert("_port".to_string(), py_int(port as i64));
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
        dict: smtp_dict,
        bases: vec![],
        mro: vec![],
    });
    d.insert("SMTP".to_string(), smtp_type);

    d
}