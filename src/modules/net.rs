use crate::object::*;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

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
        Ok(py_tuple(vec![
            py_list(readable),
            py_list(vec![]),
            py_list(vec![]),
        ]))
    });

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

thread_local! {
    static CALLED_PROCESS_ERROR_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// `subprocess.CalledProcessError` — was missing entirely, so
/// `check_output`'s own non-zero-exit-status path raised a generic
/// `RuntimeError` instead of the specific, real exception real code
/// commonly catches by name (`except subprocess.CalledProcessError as e:
/// ... e.returncode`). A real `Exception`-derived `Type` (not just a
/// plain marker) exposing `.returncode`/`.cmd`/`.output`/`.stdout`/
/// `.stderr` as real instance attributes, matching real CPython's shape.
fn get_called_process_error_type() -> PyObjectRef {
    let existing = CALLED_PROCESS_ERROR_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    type_dict.insert_str(
        "__str__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__str__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let rc = dict
                        .get_str("returncode")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(-1);
                    let cmd = dict.get_str("cmd").map(|v| v.str()).unwrap_or_default();
                    Ok(py_str(&format!(
                        "Command '{}' returned non-zero exit status {}.",
                        cmd, rc
                    )))
                } else {
                    Ok(py_str(""))
                }
            },
        }),
    );
    // No native `Exception` PyObject::Type exists to list as a real base
    // here (builtin exceptions are represented as `BuiltinFunction`
    // markers elsewhere, not `Type`s) — `except CalledProcessError:`
    // matches by exact class identity via the normal Instance/Type MRO
    // walk regardless, so this is enough for the common case, just not
    // also catchable via a bare `except Exception:`.
    let typ = PyObjectRef::new(PyObject::Type {
        name: "CalledProcessError".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    // A class's own `mro` must include ITSELF (real Python: `C.__mro__[0]
    // is C`) — `except CalledProcessError as e:`'s matching walks
    // `instance.typ`'s `mro` looking for the `except` clause's referenced
    // class, so an empty `mro` here made an exact-class match fail
    // entirely (confirmed via repro: `check_output` raising this and being
    // caught by its own exact type name still fell through uncaught).
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
    }
    CALLED_PROCESS_ERROR_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn make_called_process_error(
    returncode: i64,
    cmd: &str,
    output: Vec<u8>,
    stderr: Vec<u8>,
) -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    dict.insert_str("returncode", py_int(returncode));
    dict.insert_str("cmd", py_str(cmd));
    dict.insert_str("output", PyObjectRef::imm(PyObject::Bytes(output.clone())));
    dict.insert_str("stdout", PyObjectRef::imm(PyObject::Bytes(output)));
    dict.insert_str("stderr", PyObjectRef::imm(PyObject::Bytes(stderr)));
    PyObjectRef::new(PyObject::Instance {
        typ: get_called_process_error_type(),
        dict,
    })
}

thread_local! {
    static COMPLETED_PROCESS_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// `subprocess.CompletedProcess` — `subprocess.run(...)` previously
/// returned a bare `dict` (`{"returncode": ..., "stdout": ..., "stderr":
/// ...}`) instead of a real object with `.returncode`/`.stdout`/`.stderr`
/// ATTRIBUTE access, which is how every real caller uses it
/// (`subprocess.run(...).returncode`) — `result["returncode"]` happened to
/// also work by accident (dict subscript), but `.returncode` raised
/// `AttributeError: 'dict' object has no attribute 'returncode'`. Same
/// shape as `CalledProcessError` just above: a real `Type`/`Instance` pair,
/// not a plain marker.
fn get_completed_process_type() -> PyObjectRef {
    let existing = COMPLETED_PROCESS_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    type_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let rc = dict
                        .get_str("returncode")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let args_repr = dict.get_str("args").map(|v| v.repr()).unwrap_or_default();
                    Ok(py_str(&format!(
                        "CompletedProcess(args={}, returncode={})",
                        args_repr, rc
                    )))
                } else {
                    Ok(py_str("CompletedProcess(...)"))
                }
            },
        }),
    );
    type_dict.insert_str(
        "check_returncode",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "check_returncode".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let rc = dict
                        .get_str("returncode")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if rc != 0 {
                        let cmd = dict.get_str("args").map(|v| v.str()).unwrap_or_default();
                        let stdout = dict
                            .get_str("stdout")
                            .map(|v| v.str().into_bytes())
                            .unwrap_or_default();
                        let stderr = dict
                            .get_str("stderr")
                            .map(|v| v.str().into_bytes())
                            .unwrap_or_default();
                        let err = make_called_process_error(rc, &cmd, stdout, stderr);
                        return Err(PyError::Exception("CalledProcessError".to_string(), err));
                    }
                }
                Ok(py_none())
            },
        }),
    );
    let typ = PyObjectRef::new(PyObject::Type {
        name: "CompletedProcess".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
    }
    COMPLETED_PROCESS_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn make_completed_process(
    cmd_args: PyObjectRef,
    returncode: i64,
    stdout: PyObjectRef,
    stderr: PyObjectRef,
) -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    dict.insert_str("args", cmd_args);
    dict.insert_str("returncode", py_int(returncode));
    dict.insert_str("stdout", stdout);
    dict.insert_str("stderr", stderr);
    PyObjectRef::new(PyObject::Instance {
        typ: get_completed_process_type(),
        dict,
    })
}

pub fn create_subprocess_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sub_func {
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
    d.insert_str("CalledProcessError", get_called_process_error_type());

    // Real `subprocess.run`/`check_call`/`check_output` all have `shell` as
    // a KEYWORD-ONLY parameter (`def run(args, *, shell=False, ...)`) — this
    // used to treat `args[1]` as if it were a POSITIONAL `shell` flag
    // instead. Since keyword arguments arrive packed into a trailing `Dict`
    // (this project's own established calling convention), ANY call using
    // ANY keyword argument at all — `capture_output=True`, `text=True`,
    // `check=True`, `timeout=...`, all extremely common — put that non-
    // empty kwargs dict in `args[1]`, and `args[1].truthy()` on a non-empty
    // dict is always `True`: every such call silently ran in `shell=True`
    // mode instead, feeding the WHOLE debug-stringified command list
    // (`"['/path/to/python', '-c', ...]"`, brackets and all) to `/bin/sh
    // -c` as one command — confirmed via direct repro (`subprocess.run(cmd,
    // capture_output=True, text=True)` failed with `/bin/sh: ... [/path...,:
    // No such file or directory`, the literal Rust-debug-formatted list).
    // Looks up `shell` specifically BY NAME in the trailing kwargs dict
    // instead.
    fn kwarg_bool(args: &[PyObjectRef], name: &str) -> bool {
        match args.last() {
            Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
                if let PyObject::Dict(d) = &*last.borrow() {
                    d.get(&py_str(name))
                        .ok()
                        .flatten()
                        .map(|v| v.truthy())
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    // Extract a keyword dict argument (e.g. `env=`) from the trailing
    // kwargs pack.
    fn kwarg_dict(
        args: &[PyObjectRef],
        name: &str,
    ) -> Option<std::collections::HashMap<String, String>> {
        match args.last() {
            Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
                if let PyObject::Dict(d) = &*last.borrow() {
                    if let Some(v) = d.get(&py_str(name)).ok().flatten() {
                        if let PyObject::Dict(ed) = &*v.borrow() {
                            let mut m = std::collections::HashMap::new();
                            for (k, val) in ed.items() {
                                m.insert(k.str(), val.str());
                            }
                            return Some(m);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    // Apply the `env=` kwargs (replacing the inherited environment
    // entirely, matching CPython) to a command.
    fn apply_env(cmd: &mut std::process::Command, args: &[PyObjectRef]) {
        if let Some(env_map) = kwarg_dict(args, "env") {
            cmd.env_clear();
            for (k, v) in env_map {
                cmd.env(k, v);
            }
        }
    }

    sub_func!("run", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("run() missing required argument"));
        }
        let shell = kwarg_bool(args, "shell");
        let cmd_str = args[0].str();
        if cmd_str.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let output = if shell {
            let mut cmd = std::process::Command::new("/bin/sh");
            cmd.arg("-c").arg(&cmd_str);
            apply_env(&mut cmd, args);
            cmd.output().map_err(|e| PyError::os_error_from_io(&e))?
        } else {
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                vec![cmd_str]
            };
            if cmd_args.is_empty() {
                return Err(PyError::ValueError("empty command".to_string()));
            }
            let mut cmd = std::process::Command::new(&cmd_args[0]);
            cmd.args(&cmd_args[1..]);
            apply_env(&mut cmd, args);
            cmd.output().map_err(|e| PyError::os_error_from_io(&e))?
        };
        let returncode = output.status.code().unwrap_or(-1) as i64;
        let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
        Ok(make_completed_process(
            args[0].clone(),
            returncode,
            py_str(&stdout_str),
            py_str(&stderr_str),
        ))
    });

    sub_func!("check_call", |args| {
        // Run a command and check return code
        if args.is_empty() {
            return Err(PyError::type_error(
                "check_call() missing required argument",
            ));
        }
        let cmd_str = args[0].str();
        let shell = kwarg_bool(args, "shell");
        let output = if shell {
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c").arg(&cmd_str);
            apply_env(&mut cmd, args);
            cmd.output().map_err(|e| PyError::os_error_from_io(&e))?
        } else {
            // Was: `cmd_str.split_whitespace()` — the LIST form (the common
            // case, e.g. `[sys.executable, '-E', '-c', code]`) was stringified
            // via `.str()` first (producing the Rust-debug-ish
            // `"['/path/to/exe', '-E', ...]"`) and THEN split on whitespace,
            // shredding the executable path into garbage tokens like
            // `"['/path/to/exe',"` — every list-form call failed with a
            // spurious `OSError: No such file or directory`. Mirrors the
            // already-correct `run`/`check_output` handling just above/below.
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                cmd_str.split_whitespace().map(|s| s.to_string()).collect()
            };
            if cmd_args.is_empty() {
                return Err(PyError::type_error(
                    "check_call() requires a non-empty command",
                ));
            }
            let mut cmd = std::process::Command::new(&cmd_args[0]);
            cmd.args(&cmd_args[1..]);
            apply_env(&mut cmd, args);
            cmd.output().map_err(|e| PyError::os_error_from_io(&e))?
        };
        let returncode = output.status.code().unwrap_or(-1);
        if returncode != 0 {
            return Err(PyError::runtime_error(format!(
                "Command returned non-zero exit status {}",
                returncode
            )));
        }
        Ok(py_none())
    });

    sub_func!("check_output", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "check_output() missing required argument",
            ));
        }
        let shell = kwarg_bool(args, "shell");
        let cmd_str = args[0].str();
        if cmd_str.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let output = if shell {
            let mut cmd = std::process::Command::new("/bin/sh");
            cmd.arg("-c").arg(&cmd_str);
            apply_env(&mut cmd, args);
            cmd.output().map_err(|e| PyError::os_error_from_io(&e))?
        } else {
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                vec![cmd_str]
            };
            if cmd_args.is_empty() {
                return Err(PyError::ValueError("empty command".to_string()));
            }
            let mut cmd = std::process::Command::new(&cmd_args[0]);
            cmd.args(&cmd_args[1..]);
            apply_env(&mut cmd, args);
            cmd.output().map_err(|e| PyError::os_error_from_io(&e))?
        };
        if !output.status.success() {
            let rc = output.status.code().unwrap_or(-1) as i64;
            let instance =
                make_called_process_error(rc, &args[0].str(), output.stdout, output.stderr);
            return Err(PyError::Exception(
                "CalledProcessError".to_string(),
                instance,
            ));
        }
        // Return stdout as bytes
        Ok(PyObjectRef::imm(PyObject::Bytes(output.stdout)))
    });

    // Real CPython's internal `subprocess._cleanup()` reaps any finished
    // background children it's still tracking. `Lib/test/support/script_
    // helper.py`'s `run_python_until_end` calls it unconditionally in a
    // `finally` block after every spawned test subprocess — it was missing
    // entirely (`AttributeError`), so EVERY test using `assert_python_ok`/
    // `assert_python_failure` failed there even after the process itself
    // ran and exited correctly. This project's `Popen`/`run`/`check_call`
    // all block synchronously with no background-process registry, so
    // there is nothing to reap — a no-op is behavior-equivalent.
    sub_func!("_cleanup", |_args| Ok(py_none()));

    // Constants
    d.insert_str("PIPE", py_int(-1));
    d.insert_str("STDOUT", py_int(-2));
    d.insert_str("DEVNULL", py_int(-3));

    // `subprocess.Popen` — was missing ENTIRELY (only the higher-level
    // `run`/`check_output` convenience wrappers existed, both of which
    // block synchronously via `Command::output()`). Real CPython's own
    // `run`/`check_output` are themselves implemented ON TOP OF `Popen` —
    // it's the foundational class real code reaches for whenever it needs
    // non-blocking spawn, streaming I/O, or manual process lifecycle
    // control (`.poll()`, `.wait()`, `.communicate()`, `.terminate()`).
    // Spawns eagerly (matching real `Popen` semantics — construction
    // itself starts the child process) via `Command::spawn()` (non-
    // blocking, unlike `.output()`). `stdin`/`stdout`/`stderr` accept
    // `subprocess.PIPE`/`DEVNULL`/`STDOUT` (int sentinels) or default to
    // inherited (matching real Python's own `None` default).
    sub_func!("Popen", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "Popen() missing required argument 'args'",
            ));
        }
        let kwargs = args.last().and_then(|a| {
            if let PyObject::Dict(d) = &*a.borrow() {
                Some(d.clone())
            } else {
                None
            }
        });
        let get_kw = |name: &str| {
            kwargs
                .as_ref()
                .and_then(|d| d.get(&py_str(name)).ok().flatten())
        };
        let shell = get_kw("shell").map(|v| v.truthy()).unwrap_or(false);
        let cmd_arg = &args[0];

        let mut command = if shell {
            let cmd_str = cmd_arg.str();
            let mut c = std::process::Command::new("/bin/sh");
            c.arg("-c").arg(&cmd_str);
            c
        } else {
            let cmd_args: Vec<String> = match &*cmd_arg.borrow() {
                PyObject::List(items) | PyObject::Tuple(items) => {
                    items.iter().map(|a| a.str()).collect()
                }
                _ => vec![cmd_arg.str()],
            };
            if cmd_args.is_empty() {
                return Err(PyError::value_error("empty command"));
            }
            let mut c = std::process::Command::new(&cmd_args[0]);
            c.args(&cmd_args[1..]);
            c
        };
        if let Some(cwd) = get_kw("cwd") {
            if !matches!(&*cwd.borrow(), PyObject::None) {
                command.current_dir(crate::object::path_arg_to_string(&cwd));
            }
        }
        if let Some(env_kw) = get_kw("env") {
            // An explicit env REPLACES the inherited environment entirely
            // (CPython semantics — the test suite relies on it for
            // PYTHONHASHSEED).
            if let PyObject::Dict(ed) = &*env_kw.borrow() {
                command.env_clear();
                for (k, v) in ed.items() {
                    command.env(k.str(), v.str());
                }
            }
        }
        // Sentinel ints (matching this module's own PIPE/STDOUT/DEVNULL
        // constants just above) map to the corresponding `Stdio` mode;
        // anything else (including the real default, `None`) inherits the
        // parent's stream, matching real Python's own default.
        let stdio_for = |v: &PyObjectRef| -> std::process::Stdio {
            match v.as_i64() {
                Some(-1) => std::process::Stdio::piped(),
                Some(-3) => std::process::Stdio::null(),
                _ => std::process::Stdio::inherit(),
            }
        };
        // `stderr=STDOUT` (-2): the child's stdout and stderr must flow into
        // the SAME pipe (CPython merges them; test_cmd_line_script's
        // interactive_python drains the combined stream looking for the
        // REPL prompt, which is written to stderr). Rust's `Command` has no
        // "stderr = stdout" `Stdio`, so build a socketpair, hand BOTH the
        // child's stdout and stderr the same write end, and expose the read
        // end as both `.stdout` and `.stderr`.
        let stderr_is_stdout = get_kw("stderr")
            .map(|v| v.as_i64() == Some(-2))
            .unwrap_or(false);
        let mut merged_read_end: Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> = None;
        if stderr_is_stdout {
            use std::os::fd::OwnedFd;
            use std::os::unix::io::IntoRawFd;
            if let Ok((read_sock, write_sock)) = std::os::unix::net::UnixStream::pair() {
                let write2 = write_sock.try_clone();
                let read_file = unsafe { std::fs::File::from_raw_fd(read_sock.into_raw_fd()) };
                merged_read_end = Some(std::rc::Rc::new(std::cell::RefCell::new(read_file)));
                command.stdout(std::process::Stdio::from(unsafe {
                    OwnedFd::from_raw_fd(write_sock.into_raw_fd())
                }));
                if let Ok(w2) = write2 {
                    command.stderr(std::process::Stdio::from(unsafe {
                        OwnedFd::from_raw_fd(w2.into_raw_fd())
                    }));
                } else {
                    command.stderr(std::process::Stdio::null());
                }
            }
        }
        if let Some(v) = get_kw("stdin") {
            command.stdin(stdio_for(&v));
        }
        if !stderr_is_stdout {
            if let Some(v) = get_kw("stdout") {
                command.stdout(stdio_for(&v));
            }
            if let Some(v) = get_kw("stderr") {
                match v.as_i64() {
                    Some(-2) => {
                        command.stderr(std::process::Stdio::piped());
                    }
                    _ => {
                        command.stderr(stdio_for(&v));
                    }
                }
            }
        }

        let mut child = command.spawn().map_err(|e| PyError::os_error_from_io(&e))?;
        let pid = child.id() as i64;
        // Take the piped ends into the Process so `.stdin`/`.stdout`/
        // `.stderr` expose real, readable/writable file objects (not dummy
        // /dev/null — the interactive REPL tests write a statement then
        // read the prompt back, which needs the actual pipe).
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        let wrap_pipe = |p: Option<std::process::ChildStdout>| -> Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> {
            p.map(|s| std::rc::Rc::new(std::cell::RefCell::new(unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) })))
        };
        let wrap_pipe_in = |p: Option<std::process::ChildStdin>| -> Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> {
            p.map(|s| std::rc::Rc::new(std::cell::RefCell::new(unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) })))
        };
        let wrap_pipe_err = |p: Option<std::process::ChildStderr>| -> Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> {
            p.map(|s| std::rc::Rc::new(std::cell::RefCell::new(unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) })))
        };
        let (stdout_pipe, stderr_pipe) = if let Some(m) = merged_read_end {
            (Some(m.clone()), Some(m))
        } else {
            (
                wrap_pipe(child.stdout.take()),
                wrap_pipe_err(child.stderr.take()),
            )
        };
        let stdin_pipe = wrap_pipe_in(child.stdin.take());
        Ok(PyObjectRef::new(PyObject::Process {
            child: std::rc::Rc::new(std::cell::RefCell::new(Some(child))),
            returncode: std::rc::Rc::new(std::cell::RefCell::new(None)),
            pid,
            stdin_pipe,
            stdout_pipe,
            stderr_pipe,
        }))
    });

    d
}

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

pub fn create_html_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! html_func {
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
                    let entity: String = chars[i + 1..i + end].iter().collect();
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
                                let codepoint: Option<u32> =
                                    if entity.starts_with("#x") || entity.starts_with("#X") {
                                        u32::from_str_radix(&entity[2..], 16).ok()
                                    } else {
                                        entity[1..].parse().ok()
                                    };
                                codepoint
                                    .and_then(|cp| char::from_u32(cp))
                                    .map(|c| c.to_string())
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
        ("amp", "&"),
        ("lt", "<"),
        ("gt", ">"),
        ("quot", "\""),
        ("apos", "'"),
        ("nbsp", "\u{00A0}"),
        ("iexcl", "\u{00A1}"),
        ("cent", "\u{00A2}"),
        ("pound", "\u{00A3}"),
        ("curren", "\u{00A4}"),
        ("yen", "\u{00A5}"),
        ("brvbar", "\u{00A6}"),
        ("sect", "\u{00A7}"),
        ("uml", "\u{00A8}"),
        ("copy", "\u{00A9}"),
        ("ordf", "\u{00AA}"),
        ("laquo", "\u{00AB}"),
        ("not", "\u{00AC}"),
        ("shy", "\u{00AD}"),
        ("reg", "\u{00AE}"),
        ("macr", "\u{00AF}"),
        ("deg", "\u{00B0}"),
        ("plusmn", "\u{00B1}"),
        ("sup2", "\u{00B2}"),
        ("sup3", "\u{00B3}"),
        ("acute", "\u{00B4}"),
        ("micro", "\u{00B5}"),
        ("para", "\u{00B6}"),
        ("middot", "\u{00B7}"),
        ("cedil", "\u{00B8}"),
        ("sup1", "\u{00B9}"),
        ("ordm", "\u{00BA}"),
        ("raquo", "\u{00BB}"),
        ("frac14", "\u{00BC}"),
        ("frac12", "\u{00BD}"),
        ("frac34", "\u{00BE}"),
        ("iquest", "\u{00BF}"),
        ("times", "\u{00D7}"),
        ("divide", "\u{00F7}"),
        ("OElig", "\u{0152}"),
        ("oelig", "\u{0153}"),
        ("Scaron", "\u{0160}"),
        ("scaron", "\u{0161}"),
        ("Yuml", "\u{0178}"),
        ("fnof", "\u{0192}"),
        ("circ", "\u{02C6}"),
        ("tilde", "\u{02DC}"),
        ("Alpha", "\u{0391}"),
        ("Beta", "\u{0392}"),
        ("Gamma", "\u{0393}"),
        ("Delta", "\u{0394}"),
        ("Epsilon", "\u{0395}"),
        ("Zeta", "\u{0396}"),
        ("Eta", "\u{0397}"),
        ("Theta", "\u{0398}"),
        ("Iota", "\u{0399}"),
        ("Kappa", "\u{039A}"),
        ("Lambda", "\u{039B}"),
        ("Mu", "\u{039C}"),
        ("Nu", "\u{039D}"),
        ("Xi", "\u{039E}"),
        ("Omicron", "\u{039F}"),
        ("Pi", "\u{03A0}"),
        ("Rho", "\u{03A1}"),
        ("Sigma", "\u{03A3}"),
        ("Tau", "\u{03A4}"),
        ("Upsilon", "\u{03A5}"),
        ("Phi", "\u{03A6}"),
        ("Chi", "\u{03A7}"),
        ("Psi", "\u{03A8}"),
        ("Omega", "\u{03A9}"),
        ("alpha", "\u{03B1}"),
        ("beta", "\u{03B2}"),
        ("gamma", "\u{03B3}"),
        ("delta", "\u{03B4}"),
        ("epsilon", "\u{03B5}"),
        ("zeta", "\u{03B6}"),
        ("eta", "\u{03B7}"),
        ("theta", "\u{03B8}"),
        ("iota", "\u{03B9}"),
        ("kappa", "\u{03BA}"),
        ("lambda", "\u{03BB}"),
        ("mu", "\u{03BC}"),
        ("nu", "\u{03BD}"),
        ("xi", "\u{03BE}"),
        ("omicron", "\u{03BF}"),
        ("pi", "\u{03C0}"),
        ("rho", "\u{03C1}"),
        ("sigmaf", "\u{03C2}"),
        ("sigma", "\u{03C3}"),
        ("tau", "\u{03C4}"),
        ("upsilon", "\u{03C5}"),
        ("phi", "\u{03C6}"),
        ("chi", "\u{03C7}"),
        ("psi", "\u{03C8}"),
        ("omega", "\u{03C9}"),
        ("thetasym", "\u{03D1}"),
        ("upsih", "\u{03D2}"),
        ("piv", "\u{03D6}"),
        ("ensp", "\u{2002}"),
        ("emsp", "\u{2003}"),
        ("thinsp", "\u{2009}"),
        ("zwnj", "\u{200C}"),
        ("zwj", "\u{200D}"),
        ("lrm", "\u{200E}"),
        ("rlm", "\u{200F}"),
        ("ndash", "\u{2013}"),
        ("mdash", "\u{2014}"),
        ("lsquo", "\u{2018}"),
        ("rsquo", "\u{2019}"),
        ("sbquo", "\u{201A}"),
        ("ldquo", "\u{201C}"),
        ("rdquo", "\u{201D}"),
        ("bdquo", "\u{201E}"),
        ("dagger", "\u{2020}"),
        ("Dagger", "\u{2021}"),
        ("bull", "\u{2022}"),
        ("hellip", "\u{2026}"),
        ("permil", "\u{2030}"),
        ("prime", "\u{2032}"),
        ("Prime", "\u{2033}"),
        ("lsaquo", "\u{2039}"),
        ("rsaquo", "\u{203A}"),
        ("oline", "\u{203E}"),
        ("frasl", "\u{2044}"),
        ("euro", "\u{20AC}"),
        ("image", "\u{2111}"),
        ("weierp", "\u{2118}"),
        ("real", "\u{211C}"),
        ("trade", "\u{2122}"),
        ("alefsym", "\u{2135}"),
        ("larr", "\u{2190}"),
        ("uarr", "\u{2191}"),
        ("rarr", "\u{2192}"),
        ("darr", "\u{2193}"),
        ("harr", "\u{2194}"),
        ("crarr", "\u{21B5}"),
        ("lArr", "\u{21D0}"),
        ("uArr", "\u{21D1}"),
        ("rArr", "\u{21D2}"),
        ("dArr", "\u{21D3}"),
        ("hArr", "\u{21D4}"),
        ("forall", "\u{2200}"),
        ("part", "\u{2202}"),
        ("exist", "\u{2203}"),
        ("empty", "\u{2205}"),
        ("nabla", "\u{2207}"),
        ("isin", "\u{2208}"),
        ("notin", "\u{2209}"),
        ("ni", "\u{220B}"),
        ("prod", "\u{220F}"),
        ("sum", "\u{2211}"),
        ("minus", "\u{2212}"),
        ("lowast", "\u{2217}"),
        ("radic", "\u{221A}"),
        ("prop", "\u{221D}"),
        ("infin", "\u{221E}"),
        ("ang", "\u{2220}"),
        ("and", "\u{2227}"),
        ("or", "\u{2228}"),
        ("cap", "\u{2229}"),
        ("cup", "\u{222A}"),
        ("int", "\u{222B}"),
        ("there4", "\u{2234}"),
        ("sim", "\u{223C}"),
        ("cong", "\u{2245}"),
        ("asymp", "\u{2248}"),
        ("ne", "\u{2260}"),
        ("equiv", "\u{2261}"),
        ("le", "\u{2264}"),
        ("ge", "\u{2265}"),
        ("sub", "\u{2282}"),
        ("sup", "\u{2283}"),
        ("nsub", "\u{2284}"),
        ("sube", "\u{2286}"),
        ("supe", "\u{2287}"),
        ("oplus", "\u{2295}"),
        ("otimes", "\u{2297}"),
        ("perp", "\u{22A5}"),
        ("sdot", "\u{22C5}"),
        ("lceil", "\u{2308}"),
        ("rceil", "\u{2309}"),
        ("lfloor", "\u{230A}"),
        ("rfloor", "\u{230B}"),
        ("lang", "\u{2329}"),
        ("rang", "\u{232A}"),
        ("loz", "\u{25CA}"),
        ("spades", "\u{2660}"),
        ("clubs", "\u{2663}"),
        ("hearts", "\u{2665}"),
        ("diams", "\u{2666}"),
    ];

    let py_dict_obj = py_dict();
    if let PyObject::Dict(ref mut pd) = &mut *py_dict_obj.borrow_mut() {
        for (name, ch) in pairs {
            pd.set(py_str(name), py_str(ch)).ok();
        }
    }

    d.insert_str("html5", py_dict_obj);
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

use std::cell::RefCell;
use std::rc::Rc;

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
