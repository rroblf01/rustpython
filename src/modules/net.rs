use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
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
    // NOTE: real CPython's `select` module only exposes `devpoll` on Solaris
    // (gated behind `HAVE_SYS_DEVPOLL_H` at build time) — `test_devpoll.py`
    // itself checks `if not hasattr(select, 'devpoll'): raise
    // unittest.SkipTest(...)` at import time, expecting to be skipped
    // entirely on every other platform. Previously this module always
    // registered a (nonfunctional, methodless) `select.devpoll` type, which
    // made `hasattr` true on Linux and let the whole test file run against a
    // stub with no `register`/`fileno`/`close` methods. So: no `devpoll`
    // type/factory here at all, matching CPython's own Linux behavior.
    fn poll_reg_of(self_obj: &PyObjectRef) -> Result<PyObjectRef, PyError> {
        if let PyObject::Instance { dict, .. } = &*self_obj.borrow() {
            if let Some(r) = dict.get("_reg") {
                return Ok(r.clone());
            }
        }
        Err(PyError::runtime_error("poll object missing registry"))
    }

    fn poll_is_int_obj(o: &PyObjectRef) -> bool {
        matches!(o, PyObjectRef::SmallInt(_) | PyObjectRef::SmallBool(_))
            || matches!(&*o.borrow(), PyObject::Int(_))
    }

    fn poll_convert_eventmask(o: &PyObjectRef) -> Result<i64, PyError> {
        if !poll_is_int_obj(o) {
            return Err(PyError::type_error("an integer is required"));
        }
        match o.as_i64() {
            Some(v) if v < 0 => Err(PyError::value_error("negative event mask value")),
            Some(v) if v > 0xFFFF => Err(PyError::overflow_error(
                "signed short integer is greater than maximum",
            )),
            Some(v) => Ok(v),
            None => Err(PyError::overflow_error(
                "Python int too large to convert to C unsigned short",
            )),
        }
    }

    fn poll_convert_timeout(o: &PyObjectRef) -> Result<i32, PyError> {
        if matches!(&*o.borrow(), PyObject::None) {
            return Ok(-1);
        }
        if poll_is_int_obj(o) {
            return match o.as_i64() {
                Some(v) => {
                    if v < 0 {
                        Ok(-1)
                    } else if v > i32::MAX as i64 {
                        Err(PyError::overflow_error("timeout is too large"))
                    } else {
                        Ok(v as i32)
                    }
                }
                None => Err(PyError::overflow_error("timeout is too large")),
            };
        }
        if let Some(f) = o.as_f64() {
            if f < 0.0 {
                return Ok(-1);
            }
            if f > i32::MAX as f64 {
                return Err(PyError::overflow_error("timeout is too large"));
            }
            return Ok(f as i32);
        }
        Err(PyError::type_error("timeout must be an integer or None"))
    }

    fn make_poll_type(name: &str) -> PyObjectRef {
        let mut dict = HashMap::new();
        let name_owned = name.to_string();
        // __new__ must raise TypeError: cannot create 'select.poll' instances
        let name_clone = name_owned.clone();
        dict.insert(
            "__new__".to_string(),
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                move |_args: &[PyObjectRef]| {
                    Err(PyError::type_error(format!(
                        "cannot create '{}' instances",
                        name_clone
                    )))
                },
            ))),
        );
        dict.insert(
            "register".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "register".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error(
                            "register expected at least 1 argument, got 0",
                        ));
                    }
                    let fd = get_fd(&args[1])?;
                    let eventmask = if let Some(m) = args.get(2) {
                        poll_convert_eventmask(m)?
                    } else {
                        (POLLIN as i64) | (POLLOUT as i64) | (POLLPRI as i64)
                    };
                    let reg = poll_reg_of(&args[0])?;
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        r.set(py_int(fd as i64), py_int(eventmask))?;
                    }
                    Ok(py_none())
                },
            }),
        );
        dict.insert(
            "unregister".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "unregister".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("unregister expected 1 argument, got 0"));
                    }
                    let fd = get_fd(&args[1])?;
                    let reg = poll_reg_of(&args[0])?;
                    let removed = if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        r.remove(&py_int(fd as i64))
                    } else {
                        return Err(PyError::runtime_error("poll object missing registry"));
                    };
                    removed?;
                    Ok(py_none())
                },
            }),
        );
        dict.insert(
            "modify".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "modify".to_string(),
                func: |args| {
                    if args.len() < 3 {
                        return Err(PyError::type_error("modify expected 2 arguments"));
                    }
                    let fd = get_fd(&args[1])?;
                    let eventmask = poll_convert_eventmask(&args[2])?;
                    let reg = poll_reg_of(&args[0])?;
                    let exists = if let PyObject::Dict(r) = &*reg.borrow() {
                        r.contains(&py_int(fd as i64))?
                    } else {
                        false
                    };
                    if !exists {
                        // Real CPython raises OSError(ENOENT) — "modify"
                        // only touches an already-registered fd, unlike
                        // "register" which adds one.
                        let mut extra = std::collections::HashMap::new();
                        extra.insert("errno".to_string(), py_int(2));
                        extra.insert("strerror".to_string(), py_str("No such file or directory"));
                        return Err(PyError::Exception(
                            "OSError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "OSError".to_string(),
                                args: vec![py_int(2), py_str("No such file or directory")],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: Some(extra),
                            }),
                        ));
                    }
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        r.set(py_int(fd as i64), py_int(eventmask))?;
                    }
                    Ok(py_none())
                },
            }),
        );
        dict.insert(
            "poll".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "poll".to_string(),
                func: |args| {
                    let timeout_ms = if let Some(t) = args.get(1) {
                        poll_convert_timeout(t)?
                    } else {
                        -1
                    };
                    let reg = poll_reg_of(&args[0])?;
                    let entries: Vec<(i32, i16)> = if let PyObject::Dict(r) = &*reg.borrow() {
                        r.items()
                            .into_iter()
                            .map(|(k, v)| {
                                (
                                    k.as_i64().unwrap_or(-1) as i32,
                                    v.as_i64().unwrap_or(0) as i16,
                                )
                            })
                            .collect()
                    } else {
                        vec![]
                    };
                    let mut poll_fds: Vec<PollFd> = entries
                        .iter()
                        .map(|&(fd, events)| PollFd {
                            fd,
                            events,
                            revents: 0,
                        })
                        .collect();
                    // When running inside a cooperative thread drain, a
                    // blocking C poll(-1) would deadlock the entire
                    // interpreter (the drain caller can never return to
                    // write to the pipe or run other coop jobs).  Instead
                    // use a short polling interval so the cooperative
                    // scheduler can make progress.
                    let effective_timeout =
                        if timeout_ms < 0 && crate::modules::coop_blocked_forever() {
                            100
                        } else {
                            timeout_ms
                        };
                    let rc = unsafe {
                        poll(
                            poll_fds.as_mut_ptr(),
                            poll_fds.len() as u32,
                            effective_timeout,
                        )
                    };
                    if rc < 0 {
                        let err = std::io::Error::last_os_error();
                        return Err(PyError::os_error_from_io(&err));
                    }
                    // If we got no events and we're in a cooperative
                    // context with infinite timeout, loop so that other
                    // queued cooperative threads get a chance to run.
                    if rc == 0 && timeout_ms < 0 && crate::modules::coop_blocked_forever() {
                        crate::modules::coop_threads_drain();
                    }
                    let mut result = Vec::new();
                    for pfd in &poll_fds {
                        if pfd.revents != 0 {
                            result.push(py_tuple(vec![
                                py_int(pfd.fd as i64),
                                py_int((pfd.revents as u16) as i64),
                            ]));
                        }
                    }
                    Ok(py_list(result))
                },
            }),
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
    // for closure captures
    let poll_type_for_factory = poll_type.clone();

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
                return Err(PyError::value_error(
                    "filedescriptor out of range in select()",
                ));
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
        // `call_bound_method`'s `BuiltinMethod` arm always prepends BOTH the
        // method's own (placeholder) `self_obj` AND an explicit second one
        // when that placeholder is `PyObject::None` — a convention several
        // dunder methods (e.g. `str.__mod__`) are deliberately written
        // around (`args[0]` is the placeholder, `args[1]` is the real
        // self), NOT something safe to change globally (see
        // `call_bound_method`'s own doc comment). A native `file` object's
        // `fileno` attribute (`src/object/attrs/file.rs`) is built as
        // exactly such a placeholder-`self_obj` `BuiltinMethod`, but its own
        // body expects a single `args[0] = self` (matching LOAD_ATTR's
        // rebind-in-place convention, not call_bound_method's) — so calling
        // it through `call_bound_method` here fed it `[None, file_obj]` and
        // it always saw the placeholder instead of the real file, raising
        // "fileno on non-file" even for a real, open file object registered
        // with `select.select()`/`select.poll().register()`. Rebind by hand
        // (single self) for exactly this placeholder-`BuiltinMethod` shape,
        // matching `call_method_rebound`'s approach but without needing a
        // `&mut VirtualMachine` (a plain native `fn` pointer needs none) —
        // fall back to `call_bound_method` for every other attribute shape
        // (a real user-defined `fileno()` method, e.g. `test_poll.py`'s
        // `Almost`/`Nope` classes, which is unaffected by this since it's a
        // `PyObject::Function`, not a `BuiltinMethod`).
        let is_placeholder_builtin_method = matches!(
            &*fileno_attr.borrow(),
            PyObject::BuiltinMethod { self_obj, .. } if matches!(&*self_obj.borrow(), PyObject::None)
        );
        let result = if is_placeholder_builtin_method {
            let func = if let PyObject::BuiltinMethod { func, .. } = &*fileno_attr.borrow() {
                *func
            } else {
                unreachable!()
            };
            func(&[obj.clone()])?
        } else {
            crate::object::call_bound_method(fileno_attr, obj.clone(), vec![])?
        };
        if let Some(i) = result.as_i64() {
            if i < 0 || i > i32::MAX as i64 {
                return Err(PyError::value_error(
                    "filedescriptor out of range in select()",
                ));
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
                        (&args[..args.len() - 1], Some(d.clone()))
                    } else {
                        (args, None)
                    }
                } else {
                    (args, None)
                }
            }
            _ => (args, None),
        };
        let pos = positional;
        if pos.len() < 3 {
            return Err(PyError::type_error("select() takes at least 3 arguments"));
        }
        if pos.len() > 4 && kwargs.is_none() {
            return Err(PyError::type_error(format!(
                "select() takes at most 4 arguments ({} given)",
                pos.len()
            )));
        }
        let rlist = &pos[0];
        let wlist = &pos[1];
        let xlist = &pos[2];

        let r_vec = collect_fds(rlist)?;
        let w_vec = collect_fds(wlist)?;
        let x_vec = collect_fds(xlist)?;

        let timeout_opt: Option<f64> = if let Some(kw) = kwargs {
            if let Some(v) = kw.get(&py_str("timeout")).ok().flatten() {
                if matches!(&*v.borrow(), PyObject::None) {
                    None
                } else if let Some(f) = v.as_f64() {
                    Some(f)
                } else {
                    return Err(PyError::type_error("timeout must be a float or None"));
                }
            } else if pos.len() >= 4 {
                let t = &pos[3];
                if matches!(&*t.borrow(), PyObject::None) {
                    None
                } else {
                    let is_numeric = matches!(&*t.borrow(), PyObject::Int(_) | PyObject::Float(_))
                        || matches!(
                            t,
                            PyObjectRef::SmallInt(_)
                                | PyObjectRef::SmallBool(_)
                                | PyObjectRef::SmallFloat(_)
                        );
                    if !is_numeric {
                        return Err(PyError::type_error("timeout must be a float or None"));
                    }
                    if let Some(f) = t.as_f64() {
                        Some(f)
                    } else {
                        return Err(PyError::type_error("timeout must be a float or None"));
                    }
                }
            } else {
                None
            }
        } else if pos.len() >= 4 {
            let t = &pos[3];
            if matches!(&*t.borrow(), PyObject::None) {
                None
            } else {
                let is_numeric = matches!(&*t.borrow(), PyObject::Int(_) | PyObject::Float(_))
                    || matches!(
                        t,
                        PyObjectRef::SmallInt(_)
                            | PyObjectRef::SmallBool(_)
                            | PyObjectRef::SmallFloat(_)
                    );
                if !is_numeric {
                    return Err(PyError::type_error("timeout must be a float or None"));
                }
                if let Some(f) = t.as_f64() {
                    Some(f)
                } else {
                    return Err(PyError::type_error("timeout must be a float or None"));
                }
            }
        } else {
            None
        };

        if let Some(f) = timeout_opt {
            if f < 0.0 {
                return Err(PyError::value_error("timeout must be non-negative"));
            }
        }

        let timeout_ms: i32 = match timeout_opt {
            None => -1,
            Some(f) => {
                if f == 0.0 {
                    0
                } else {
                    let ms = (f * 1000.0).trunc() as i64;
                    if ms > i32::MAX as i64 {
                        i32::MAX
                    } else if ms < 0 {
                        0
                    } else {
                        ms as i32
                    }
                }
            }
        };

        if r_vec.is_empty() && w_vec.is_empty() && x_vec.is_empty() {
            if let Some(f) = timeout_opt {
                if f > 0.0 {
                    std::thread::sleep(std::time::Duration::from_secs_f64(f));
                }
            }
            return Ok(py_tuple(vec![
                py_list(vec![]),
                py_list(vec![]),
                py_list(vec![]),
            ]));
        }

        let mut fd_to_events: std::collections::HashMap<i32, i16> =
            std::collections::HashMap::new();
        for (_, fd) in &r_vec {
            *fd_to_events.entry(*fd).or_insert(0) |= POLLIN;
        }
        for (_, fd) in &w_vec {
            *fd_to_events.entry(*fd).or_insert(0) |= POLLOUT;
        }
        for (_, fd) in &x_vec {
            *fd_to_events.entry(*fd).or_insert(0) |= POLLPRI;
        }

        let mut poll_fds: Vec<PollFd> = fd_to_events
            .iter()
            .map(|(&fd, &ev)| PollFd {
                fd,
                events: ev,
                revents: 0,
            })
            .collect();
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
        let mut fd_to_revents: std::collections::HashMap<i32, i16> =
            std::collections::HashMap::new();
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

        Ok(py_tuple(vec![
            py_list(ready_r),
            py_list(ready_w),
            py_list(ready_x),
        ]))
    });

    // ---- poll / devpoll factories and types (Closure so they can capture the Type) ----
    {
        let pt = poll_type_for_factory.clone();
        d.insert(
            "poll".to_string(),
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                move |args: &[PyObjectRef]| {
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
                    let mut inst_dict = AttrMap::new();
                    inst_dict.insert_str("_reg", py_dict());
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: pt.clone(),
                        dict: inst_dict,
                    }))
                },
            ))),
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
    if let PyObject::Type { mro, .. } = &mut *error_type.borrow_mut() {
        *mro = vec![error_type.clone()];
    }
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

/// Shared by `gethostbyname`/`gethostbyname_ex`: the literal for
/// `localhost`, a real A lookup otherwise, and the original name when
/// neither yields an address (matching this module's other best-effort
/// resolver behavior).
fn gethostbyname_impl(hostname: &str) -> PyResult<String> {
    if hostname == "localhost" || hostname == "127.0.0.1" {
        return Ok("127.0.0.1".to_string());
    }
    match std::net::ToSocketAddrs::to_socket_addrs(&(hostname, 0)) {
        Ok(mut addrs) => Ok(addrs
            .find(|a| a.is_ipv4())
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|| hostname.to_string())),
        Err(_) => Ok(hostname.to_string()),
    }
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
        let sock_type = if args.len() > 1 {
            args[1].as_i64().unwrap_or(1)
        } else {
            1
        };
        let proto = if args.len() > 2 {
            args[2].as_i64().unwrap_or(0)
        } else {
            0
        };
        if family == 1 {
            // AF_UNIX: the OS endpoint is created at bind()/connect() time,
            // exactly like CPython, so an unbound socket starts empty.
            return Ok(PyObjectRef::new(PyObject::Socket {
                inner: Rc::new(RefCell::new(SocketInner::Uninitialized)),
                family,
                type_: sock_type,
                proto,
            }));
        }
        if family != 2 {
            // Real CPython raises OSError (errno EAFNOSUPPORT) when a
            // platform/build lacks a given address family — e.g.
            // `test_socket.py`'s `requireSocket()` helper creates a probe
            // socket at class-decoration time and expects `except OSError`
            // to catch an unsupported family, skipping the test instead of
            // crashing the whole module. A bare RuntimeError isn't an
            // OSError subclass, so it propagated uncaught here.
            let io_err = std::io::Error::from_raw_os_error(97); // EAFNOSUPPORT (Linux)
            return Err(PyError::os_error_from_io(&io_err));
        }
        Ok(PyObjectRef::new(PyObject::Socket {
            inner: std::rc::Rc::new(std::cell::RefCell::new(SocketInner::Uninitialized)),
            family,
            type_: sock_type,
            proto,
        }))
    });

    /// Wrap a raw TcpStream as our Socket object.
    fn wrap_tcp_stream(stream: std::net::TcpStream) -> PyObjectRef {
        PyObjectRef::new(PyObject::Socket {
            inner: Rc::new(RefCell::new(SocketInner::TcpStream(stream))),
            family: 2, // AF_INET
            type_: 1,  // SOCK_STREAM
            proto: 0,
        })
    }

    // socketpair(): std has no AF_UNIX pair in this codebase's socket model
    // (TcpListener/TcpStream only), so emulate with a loopback TCP pair --
    // semantically identical for the select()/send/recv patterns tests use.
    sock_func!("socketpair", |args| {
        let family = args.get(0).and_then(|a| a.as_i64()).unwrap_or(2);
        let stype = args.get(1).and_then(|a| a.as_i64()).unwrap_or(1);
        if family == 1 {
            // The real thing: a connected AF_UNIX pair, which is what the
            // whole stdlib (and nearly every test) means by socketpair().
            let (a_end, b_end) = std::os::unix::net::UnixStream::pair()
                .map_err(|e| PyError::os_error_from_io(&e))?;
            return Ok(py_list(vec![
                PyObjectRef::new(PyObject::Socket {
                    inner: Rc::new(RefCell::new(SocketInner::UnixStream(a_end))),
                    family: 1,
                    type_: stype,
                    proto: 0,
                }),
                PyObjectRef::new(PyObject::Socket {
                    inner: Rc::new(RefCell::new(SocketInner::UnixStream(b_end))),
                    family: 1,
                    type_: stype,
                    proto: 0,
                }),
            ]));
        }
        let l = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| PyError::OsError(format!("socketpair bind: {}", e)))?;
        let addr = l
            .local_addr()
            .map_err(|e| PyError::OsError(format!("{}", e)))?;
        let c = std::net::TcpStream::connect(addr)
            .map_err(|e| PyError::OsError(format!("socketpair connect: {}", e)))?;
        let (a_end, _l) = l
            .accept()
            .map_err(|e| PyError::OsError(format!("socketpair accept: {}", e)))?;
        Ok(py_list(vec![wrap_tcp_stream(a_end), wrap_tcp_stream(c)]))
    });

    // `socket.create_connection(address, timeout=None, source_address=None)`
    // — connects a new TCP socket to `address` and returns it. Missing
    // entirely broke any real client code using the standard high-level
    // connect helper (e.g. `smtplib`/`poplib`/`ftplib`-style dummy test
    // servers connecting a plain client socket by hand), which fell back to
    // raw `socket()`+`connect()` only when this raised `AttributeError`.
    sock_func!("create_connection", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "create_connection() missing required argument: 'address'",
            ));
        }
        let addr = socket_addr_to_string(&args[0])?;
        let stream = TcpStream::connect(&addr).map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(wrap_tcp_stream(stream))
    });

    // `socket.getaddrinfo(host, port, family=0, type=0, proto=0, flags=0)`
    // — real CPython returns a list of `(family, type, proto, canonname,
    // sockaddr)` 5-tuples. This interpreter only ever resolves/creates
    // AF_INET/SOCK_STREAM sockets, so a single synthesized IPv4 entry per
    // resolved address is enough for the common "connect to what
    // getaddrinfo tells you" pattern (real trigger: `smtplib.py`'s
    // `SMTP._get_socket`, `ftplib`/`poplib`-adjacent test helpers) without
    // pretending to support IPv6/UDP resolution it doesn't otherwise have.
    sock_func!("getaddrinfo", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "getaddrinfo() missing required arguments: 'host' and 'port'",
            ));
        }
        let host = if matches!(&*args[0].borrow(), PyObject::None) {
            "0.0.0.0".to_string()
        } else {
            args[0].str()
        };
        let port: u16 = match &*args[1].borrow() {
            PyObject::None => 0,
            _ => args[1]
                .as_i64()
                .or_else(|| args[1].str().parse::<i64>().ok())
                .unwrap_or(0) as u16,
        };
        let addr_str = format!("{}:{}", host, port);
        let resolved = addr_str
            .to_socket_addrs()
            .map_err(|e| PyError::os_error_from_io(&e))?;
        let mut results = vec![];
        for sockaddr in resolved {
            if !sockaddr.is_ipv4() {
                continue;
            }
            let sockaddr_tuple = py_tuple(vec![
                py_str(&sockaddr.ip().to_string()),
                py_int(sockaddr.port() as i64),
            ]);
            results.push(py_tuple(vec![
                py_int(2),  // AF_INET
                py_int(1),  // SOCK_STREAM
                py_int(6),  // IPPROTO_TCP
                py_str(""), // canonname
                sockaddr_tuple,
            ]));
        }
        if results.is_empty() {
            let io_err = std::io::Error::from_raw_os_error(-2);
            return Err(PyError::os_error_from_io(&io_err));
        }
        Ok(py_list(results))
    });

    // Honest: this interpreter's `socket()` only supports AF_INET (see
    // below) — reporting `has_ipv6 = True` would make `test.support.
    // socket_helper`'s own `_is_ipv6_enabled()` try `socket.socket(AF_INET6,
    // ...)`, which raises `RuntimeError` (not `OSError`, the only thing it
    // catches), crashing instead of cleanly falling back to "no IPv6".
    d.insert_str("_GLOBAL_DEFAULT_TIMEOUT", py_none());
    d.insert_str("has_ipv6", py_bool(false));
    // Real CPython's `socket.has_dualstack_ipv6()` is a FUNCTION (not a
    // plain attribute) that probes whether the platform can create a
    // single SOCK_STREAM socket handling both AF_INET and AF_INET6 (real
    // CPython's own implementation opens an AF_INET6 socket and tries
    // clearing `IPV6_V6ONLY`) — since this interpreter's `socket()` only
    // ever supports AF_INET at all (see `has_ipv6 = False` right above),
    // dual-stack is unconditionally unavailable, so this always answers
    // `False` rather than attempting a real IPv6 probe.
    sock_func!("has_dualstack_ipv6", |_args| Ok(py_bool(false)));
    d.insert_str("AF_INET", py_int(2));
    d.insert_str("AF_INET6", py_int(10));
    d.insert_str("AF_UNIX", py_int(1));
    d.insert_str("SOCK_STREAM", py_int(1));
    d.insert_str("SOCK_DGRAM", py_int(2));
    d.insert_str("SOCK_RAW", py_int(3));
    d.insert_str("SOCK_SEQPACKET", py_int(5));
    d.insert_str("SOCK_CLOEXEC", py_int(0o2000000));
    d.insert_str("SOCK_NONBLOCK", py_int(0o4000));
    d.insert_str("SOL_SOCKET", py_int(1));
    d.insert_str("SO_REUSEADDR", py_int(2));
    d.insert_str("SO_TYPE", py_int(3));
    d.insert_str("SO_ERROR", py_int(4));
    d.insert_str("SO_BROADCAST", py_int(6));
    d.insert_str("SO_SNDBUF", py_int(7));
    d.insert_str("SO_RCVBUF", py_int(8));
    d.insert_str("SO_KEEPALIVE", py_int(9));
    d.insert_str("SO_OOBINLINE", py_int(10));
    d.insert_str("SO_LINGER", py_int(13));
    d.insert_str("SO_RCVLOWAT", py_int(18));
    d.insert_str("SO_SNDLOWAT", py_int(19));
    d.insert_str("SO_RCVTIMEO", py_int(20));
    d.insert_str("SO_SNDTIMEO", py_int(21));
    d.insert_str("SO_DOMAIN", py_int(39));
    d.insert_str("SO_PROTOCOL", py_int(38));
    d.insert_str("SO_KEEPIDLE", py_int(4));
    d.insert_str("SO_MARK", py_int(36));
    d.insert_str("SO_PRIORITY", py_int(35));
    d.insert_str("SO_BINDTODEVICE", py_int(25));
    d.insert_str("SO_PASSCRED", py_int(16));
    d.insert_str("SO_PEERCRED", py_int(17));
    d.insert_str("IPPROTO_IP", py_int(0));
    d.insert_str("IPPROTO_ICMP", py_int(1));
    d.insert_str("IPPROTO_TCP", py_int(6));
    d.insert_str("IPPROTO_UDP", py_int(17));
    d.insert_str("IPPROTO_IPV6", py_int(41));
    d.insert_str("IPPROTO_RAW", py_int(255));
    d.insert_str("TCP_NODELAY", py_int(1));
    d.insert_str("TCP_MAXSEG", py_int(2));
    d.insert_str("TCP_KEEPIDLE", py_int(4));
    d.insert_str("TCP_KEEPINTVL", py_int(5));
    d.insert_str("TCP_KEEPCNT", py_int(6));
    d.insert_str("TCP_SYNCNT", py_int(7));
    d.insert_str("TCP_LINGER2", py_int(8));
    d.insert_str("TCP_DEFER_ACCEPT", py_int(9));
    d.insert_str("TCP_USER_TIMEOUT", py_int(18));
    d.insert_str("TCP_NOTSENT_LOWAT", py_int(17));
    d.insert_str("TCP_QUICKACK", py_int(12));
    d.insert_str("TCP_CONGESTION", py_int(13));
    d.insert_str("SOMAXCONN", py_int(128));
    d.insert_str("MSG_OOB", py_int(1));
    d.insert_str("MSG_PEEK", py_int(2));
    d.insert_str("MSG_DONTROUTE", py_int(4));
    d.insert_str("MSG_CTRUNC", py_int(8));
    d.insert_str("MSG_TRUNC", py_int(16));
    d.insert_str("MSG_DONTWAIT", py_int(64));
    d.insert_str("MSG_EOR", py_int(128));
    d.insert_str("MSG_WAITALL", py_int(256));
    d.insert_str("MSG_NOSIGNAL", py_int(16384));
    d.insert_str("MSG_MORE", py_int(16384));
    d.insert_str("MSG_NOSIGNAL", py_int(16384));
    d.insert_str("MSG_CMSG_CLOEXEC", py_int(0o40000000));
    d.insert_str("SHUT_RD", py_int(0));
    d.insert_str("SHUT_WR", py_int(1));
    d.insert_str("SHUT_RDWR", py_int(2));
    d.insert_str("ZMQ_RADIO", py_int(0));
    d.insert_str("ZMQ_DGRAM", py_int(1));
    d.insert_str("ZMQ_STREAM", py_int(2));
    d.insert_str("ZMQ_SEQPACKET", py_int(3));
    d.insert_str("CANONICAL", py_int(2));
    d.insert_str("NI_NUMERICHOST", py_int(1));
    d.insert_str("NI_NUMERICSERV", py_int(2));
    d.insert_str("NI_NAMEREQD", py_int(4));
    d.insert_str("NI_NUMERIC", py_int(1 | 2));
    d.insert_str("AI_PASSIVE", py_int(1));
    d.insert_str("AI_CANONNAME", py_int(2));
    d.insert_str("AI_NUMERICHOST", py_int(4));
    d.insert_str("AI_NUMERICSERV", py_int(1024));
    d.insert_str("AI_V4MAPPED", py_int(8));
    d.insert_str("AI_ALL", py_int(16));
    d.insert_str("AI_ADDRCONFIG", py_int(32));
    d.insert_str("CMSG_LEN", py_int(16));
    d.insert_str("CMSG_SPACE", py_int(16));

    // `CMSG_LEN`/`CMSG_SPACE` are functions in CPython (they take a
    // control-message length and return the aligned buffer size), and
    // `socket.cmsg_len`/`cmsg_space` are their aliases.
    sock_func!("CMSG_LEN", |args| {
        let len = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
        Ok(py_int(((len + 15) & !15) as i64))
    });
    sock_func!("CMSG_SPACE", |args| {
        let len = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
        Ok(py_int(((len + 15) & !15) as i64))
    });
    d.insert_str(
        "cmsg_len",
        d.get_str("CMSG_LEN").cloned().unwrap_or_else(py_none),
    );
    d.insert_str(
        "cmsg_space",
        d.get_str("CMSG_SPACE").cloned().unwrap_or_else(py_none),
    );

    // Default-timeout state for `getdefaulttimeout`/`setdefaulttimeout` and
    // the `socket(...)` constructor's `timeout` keyword.
    thread_local! {
        static DEFAULT_TIMEOUT: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
    }
    sock_func!("getdefaulttimeout", |_| {
        Ok(match DEFAULT_TIMEOUT.with(|t| t.get()) {
            Some(seconds) => py_float(seconds),
            None => py_none(),
        })
    });
    sock_func!("setdefaulttimeout", |args| {
        let value = match args.first() {
            None => None,
            Some(v) if matches!(&*v.borrow(), PyObject::None) => None,
            Some(v) => {
                if !(v.as_f64().is_some() && v.as_f64().unwrap() >= 0.0) {
                    return Err(PyError::value_error("Timeout value out of range"));
                }
                v.as_f64()
            }
        };
        DEFAULT_TIMEOUT.with(|t| t.set(value));
        Ok(py_none())
    });

    // Byte-order helpers: these really do need to swap (protocol code
    // writes `struct.pack("!H", port)`-style values, and `inet_aton`/
    // `inet_ntoa` are how tests build/compare addresses).
    sock_func!("htons", |args| {
        let v = args.first().and_then(|a| a.as_i64()).unwrap_or(0) as u16;
        Ok(py_int(v.swap_bytes() as i64))
    });
    sock_func!("htonl", |args| {
        let v = args.first().and_then(|a| a.as_i64()).unwrap_or(0) as u32;
        Ok(py_int(v.swap_bytes() as i64))
    });
    sock_func!("ntohs", |args| {
        let v = args.first().and_then(|a| a.as_i64()).unwrap_or(0) as u16;
        Ok(py_int(u16::from_be(v) as i64))
    });
    sock_func!("ntohl", |args| {
        let v = args.first().and_then(|a| a.as_i64()).unwrap_or(0) as u32;
        Ok(py_int(u32::from_be(v) as i64))
    });
    sock_func!("inet_aton", |args| {
        let text = args
            .first()
            .ok_or_else(|| PyError::type_error("inet_aton() missing argument"))?
            .str();
        let parsed: std::net::Ipv4Addr = text
            .parse()
            .map_err(|_| PyError::os_error_from_io(&std::io::Error::from_raw_os_error(99)))?;
        Ok(PyObjectRef::imm(PyObject::Bytes(parsed.octets().to_vec())))
    });
    sock_func!("inet_ntoa", |args| {
        let raw = args
            .first()
            .and_then(|a| match &*a.borrow() {
                PyObject::Bytes(b) => Some(b.clone()),
                PyObject::ByteArray(b) => Some(b.clone()),
                _ => None,
            })
            .unwrap_or_default();
        if raw.len() != 4 {
            return Err(PyError::os_error_from_io(
                &std::io::Error::from_raw_os_error(22),
            ));
        }
        let addr = std::net::Ipv4Addr::new(raw[0], raw[1], raw[2], raw[3]);
        Ok(py_str(&addr.to_string()))
    });
    sock_func!("inet_pton", |args| {
        let family = args.first().and_then(|a| a.as_i64()).unwrap_or(2);
        let text = args.get(1).map(|a| a.str()).unwrap_or_default();
        if family == 2 {
            let parsed: std::net::Ipv4Addr = text
                .parse()
                .map_err(|_| PyError::os_error_from_io(&std::io::Error::from_raw_os_error(99)))?;
            Ok(PyObjectRef::imm(PyObject::Bytes(parsed.octets().to_vec())))
        } else {
            let parsed: std::net::Ipv6Addr = text
                .parse()
                .map_err(|_| PyError::os_error_from_io(&std::io::Error::from_raw_os_error(99)))?;
            Ok(PyObjectRef::imm(PyObject::Bytes(parsed.octets().to_vec())))
        }
    });
    sock_func!("inet_ntop", |args| {
        let family = args.first().and_then(|a| a.as_i64()).unwrap_or(2);
        let raw = args
            .get(1)
            .and_then(|a| match &*a.borrow() {
                PyObject::Bytes(b) => Some(b.clone()),
                PyObject::ByteArray(b) => Some(b.clone()),
                _ => None,
            })
            .unwrap_or_default();
        if family == 2 && raw.len() == 4 {
            let addr = std::net::Ipv4Addr::new(raw[0], raw[1], raw[2], raw[3]);
            return Ok(py_str(&addr.to_string()));
        }
        if raw.len() == 16 {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&raw);
            let addr = std::net::Ipv6Addr::from(octets);
            return Ok(py_str(&addr.to_string()));
        }
        Err(PyError::os_error_from_io(
            &std::io::Error::from_raw_os_error(22),
        ))
    });
    sock_func!("gethostbyaddr", |args| {
        let address = args.first().map(|a| a.str()).unwrap_or_default();
        let hostname = args
            .get(1)
            .filter(|a| !matches!(&*a.borrow(), PyObject::None))
            .map(|a| a.str())
            .unwrap_or_else(|| address.clone());
        let aliases: Vec<PyObjectRef> = Vec::new();
        let addrtype = py_int(2);
        let addr = py_str(&address);
        Ok(py_tuple(vec![
            py_str(&hostname),
            py_list(aliases),
            addrtype,
            addr,
        ]))
    });
    sock_func!("gethostbyname_ex", |args| {
        let hostname = args
            .first()
            .ok_or_else(|| PyError::type_error("gethostbyname_ex() missing argument"))?
            .str();
        let address = match gethostbyname_impl(&hostname) {
            Ok(ip) => ip,
            Err(e) => return Err(e),
        };
        Ok(py_tuple(vec![
            py_str(&hostname),
            py_list(vec![]),
            py_list(vec![py_str(&address)]),
        ]))
    });
    sock_func!("getprotobyname", |args| {
        let name = args
            .first()
            .ok_or_else(|| PyError::type_error("getprotobyname() missing argument"))?
            .str();
        let value = match name.to_ascii_lowercase().as_str() {
            "tcp" => 6,
            "udp" => 17,
            "ip" | "ipproto" => 0,
            "icmp" => 1,
            "ipv6" | "ipv6-protocol" => 41,
            _ => {
                return Err(PyError::os_error_from_io(
                    &std::io::Error::from_raw_os_error(93),
                ))
            }
        };
        Ok(py_int(value))
    });
    sock_func!("getservbyname", |args| {
        let name = args
            .first()
            .ok_or_else(|| PyError::type_error("getservbyname() missing argument"))?
            .str();
        let port = match name.as_str() {
            "http" => 80,
            "https" => 443,
            "ftp" => 21,
            "ftp-data" => 20,
            "pop3" => 110,
            "smtp" => 25,
            "telnet" => 23,
            "nntp" => 119,
            "imap" => 143,
            _ => {
                return Err(PyError::os_error_from_io(
                    &std::io::Error::from_raw_os_error(0),
                ))
            }
        };
        Ok(py_int(port))
    });
    sock_func!("getservbyport", |args| {
        let port = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
        let name = match port {
            80 => "http",
            443 => "https",
            21 => "ftp",
            20 => "ftp-data",
            110 => "pop3",
            25 => "smtp",
            23 => "telnet",
            119 => "nntp",
            143 => "imap",
            _ => {
                return Err(PyError::os_error_from_io(
                    &std::io::Error::from_raw_os_error(0),
                ))
            }
        };
        Ok(py_tuple(vec![py_str(name), py_int(port), py_str("tcp")]))
    });
    sock_func!("if_nameindex", |_| {
        Ok(py_list(vec![py_tuple(vec![py_int(1), py_str("lo")])]))
    });
    sock_func!("if_nametoindex", |args| {
        let name = args.first().map(|a| a.str()).unwrap_or_default();
        if name == "lo" {
            Ok(py_int(1))
        } else {
            Err(PyError::os_error_from_io(
                &std::io::Error::from_raw_os_error(19),
            ))
        }
    });
    sock_func!("if_indextoname", |args| {
        let idx = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
        if idx == 1 {
            Ok(py_str("lo"))
        } else {
            Err(PyError::os_error_from_io(
                &std::io::Error::from_raw_os_error(19),
            ))
        }
    });
    sock_func!("getnameinfo", |args| {
        let address = match args.first() {
            Some(v) => v.str(),
            None => return Err(PyError::type_error("getnameinfo() missing argument")),
        };
        let sockaddr = match &*args[0].borrow() {
            PyObject::Tuple(items) => items.clone(),
            _ => Vec::new(),
        };
        let port = sockaddr.get(1).and_then(|p| p.as_i64()).unwrap_or(0);
        Ok(py_tuple(vec![py_str(&address), py_int(port)]))
    });
    // `create_server(address, *, family=AF_INET, backlog=None,
    // reuse_port=False, dualstack_ipv6=False)`: a bound, listening socket
    // usable as a context manager, which is how every stdlib test server
    // (`test_httplib`, `test_urllib2_localnet`, `test_socketserver`) is
    // written.
    sock_func!("create_server", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "create_server() missing required argument: 'address'",
            ));
        }
        let addr = socket_addr_to_string(&args[0])?;
        let listener = std::net::TcpListener::bind(addr.as_str())
            .map_err(|e| PyError::os_error_from_io(&e))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(PyObjectRef::new(PyObject::Socket {
            inner: Rc::new(RefCell::new(SocketInner::TcpListener(listener))),
            family: 2,
            type_: 1,
            proto: 0,
        }))
    });
    sock_func!("fromfd", |args| {
        let _fd = args
            .get(1)
            .and_then(|a| a.as_i64())
            .ok_or_else(|| PyError::type_error("fromfd() missing required argument: 'fd'"))?;
        Err(PyError::os_error_from_io(
            &std::io::Error::from_raw_os_error(38),
        ))
    });

    sock_func!("gethostname", |_| {
        match std::process::Command::new("hostname").output() {
            Ok(output) => {
                let hostname = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(py_str(&hostname))
            }
            Err(_) => Ok(py_str("localhost")),
        }
    });

    sock_func!("getfqdn", |args| {
        let name = if let Some(first) = args.first() {
            first.str().trim().to_string()
        } else {
            String::new()
        };
        if name.is_empty() || name == "0.0.0.0" || name == "::" {
            match std::process::Command::new("hostname").output() {
                Ok(output) => {
                    let hostname = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !hostname.is_empty() {
                        Ok(py_str(&hostname))
                    } else {
                        Ok(py_str("localhost"))
                    }
                }
                Err(_) => Ok(py_str("localhost")),
            }
        } else {
            Ok(py_str(&name))
        }
    });

    // `socket.getfqdn([name])` was entirely missing (`AttributeError`) —
    // real CPython implements it in pure Python (`Lib/socket.py`): with no
    // usable `name`, fall back to `gethostname()`; otherwise a real reverse
    // lookup is attempted via `gethostbyaddr()` and, on failure, the
    // original `name` is returned unchanged. This project has no DNS
    // reverse-lookup support to build the full algorithm on, so this is the
    // honest best-effort subset: the empty-name fallback (the common case —
    // building a local test server URL, `test_urllib2_localnet.py`) plus
    // the plain passthrough CPython's own `except socket.error: pass` path
    // falls back to when a reverse lookup isn't available.
    sock_func!("getfqdn", |args| {
        let name = args.first().map(|a| a.str()).unwrap_or_default();
        let name = name.trim();
        if name.is_empty() || name == "0.0.0.0" {
            match std::process::Command::new("hostname").output() {
                Ok(output) => Ok(py_str(String::from_utf8_lossy(&output.stdout).trim())),
                Err(_) => Ok(py_str("localhost")),
            }
        } else {
            Ok(py_str(name))
        }
    });

    sock_func!("gethostbyname", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "gethostbyname() missing required argument",
            ));
        }
        let hostname = args[0].str();
        Ok(py_str(&gethostbyname_impl(&hostname)?))
    });

    d
}

/// Patches `socket.error`/`socket.timeout`/`socket.gaierror`/
/// `socket.herror` onto an already-built socket module dict, using the
/// REAL builtin OSError/TimeoutError objects — `create_socket_dict` itself
/// has no access to `builtins` (called with no arguments, before
/// `set_builtins_ref`'s thread-local is even populated), so this must run
/// from `register_native_modules`, which receives `builtins` directly as a
/// parameter. `socket.error`/`socket.timeout` are straight aliases (PEP
/// 3151 predates them; kept for compatibility, `socket.error is OSError`);
/// `gaierror`/`herror` are REAL OSError subclasses (`.__base__ is
/// OSError`), with `bases` holding the actual builtin object directly —
/// that also makes `find_exception_base_name`'s existing base-list scan
/// recognize their exception ancestry for free, no separate name-based
/// registration needed.
pub(crate) fn patch_socket_exception_aliases(
    socket_dict: &mut HashMap<String, PyObjectRef>,
    builtins: &HashMap<crate::interner::StrId, PyObjectRef>,
) {
    let real_oserror = builtins.get(&crate::interner::intern("OSError")).cloned();
    if let Some(oserror) = real_oserror.clone() {
        socket_dict.insert("error".to_string(), oserror);
    }
    if let Some(real_timeout) = builtins
        .get(&crate::interner::intern("TimeoutError"))
        .cloned()
    {
        socket_dict.insert("timeout".to_string(), real_timeout);
    }
    for name in ["gaierror", "herror"] {
        let bases: Vec<PyObjectRef> = real_oserror.clone().into_iter().collect();
        let typ = PyObjectRef::new(PyObject::Type {
            name: name.to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases,
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
            *mro = vec![typ.clone()];
        }
        socket_dict.insert(name.to_string(), typ);
    }
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
