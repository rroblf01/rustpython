use crate::object::*;
use std::collections::HashMap;

pub fn create_selectors_dict() -> HashMap<String, PyObjectRef> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut d: HashMap<String, PyObjectRef> = HashMap::new();
    d.insert_str("EVENT_READ", py_int(1));
    d.insert_str("EVENT_WRITE", py_int(2));

    thread_local! {
        static KEY_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
        static SELECTOR_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    }

    fn inst_get(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
        if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            return dict.get(name).cloned();
        }
        None
    }
    fn sock_fd(sock: &PyObjectRef) -> i64 {
        use std::os::fd::AsRawFd;
        if let PyObject::Socket { inner } = &*sock.borrow() {
            match &*inner.borrow() {
                SocketInner::TcpListener(l) => l.as_raw_fd() as i64,
                SocketInner::TcpStream(s) => s.as_raw_fd() as i64,
                _ => -1,
            }
        } else {
            -1
        }
    }
    fn stream_readable(stream: &std::net::TcpStream) -> bool {
        let _ = stream.set_nonblocking(true);
        let mut buf = [0u8; 1];
        matches!((&stream).peek(&mut buf), Ok(_))
    }
    fn obj_readable(obj: &PyObjectRef) -> bool {
        if let PyObject::Socket { inner } = &*obj.borrow() {
            return match &*inner.borrow() {
                SocketInner::TcpStream(s) => stream_readable(s),
                SocketInner::TcpListener(l) => {
                    let _ = l.set_nonblocking(true);
                    matches!(l.accept(), Ok(_))
                }
                _ => false,
            };
        }
        true // non-socket objects: always ready
    }

    fn make_key(fileobj: PyObjectRef, fd: i64, events: i64, data: PyObjectRef) -> PyObjectRef {
        let typ = KEY_TYPE.with(|c| {
            if let Some(t) = &*c.borrow() {
                return t.clone();
            }
            let mut td: HashMap<String, PyObjectRef> = HashMap::new();
            td.insert("__repr__".into(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".into(),
                func: |args| {
                    let g = |n: &str| inst_get(&args[0], n).unwrap_or_else(py_none);
                    Ok(py_str(&format!(
                        "SelectorKey(fileobj={}, fd={}, events={}, data={})",
                        g("fileobj").repr(),
                        g("fd").repr(),
                        g("events").repr(),
                        g("data").repr()
                    )))
                },
            }));
            let t = PyObjectRef::new(PyObject::Type {
                name: "SelectorKey".into(),
                dict: Box::new(crate::object::str_map_to_typedict(td)),
                bases: vec![],
                mro: vec![],
            });
            *c.borrow_mut() = Some(t.clone());
            t
        });
        let mut dict = AttrMap::new();
        dict.insert_str("fileobj", fileobj);
        dict.insert_str("fd", py_int(fd));
        dict.insert_str("events", py_int(events));
        dict.insert_str("data", data);
        PyObjectRef::new(PyObject::Instance { typ, dict })
    }

    fn reg_of(self_obj: &PyObjectRef) -> PyObjectRef {
        inst_get(self_obj, "_reg").expect("selector registry missing")
    }
    fn ensure_open(self_obj: &PyObjectRef) -> PyResult<()> {
        match inst_get(self_obj, "_closed") {
            Some(c) if c.truthy() => Err(PyError::RuntimeError(
                "Selector is closed".into(),
            )),
            _ => Ok(()),
        }
    }
    fn set_closed(self_obj: &PyObjectRef) {
        if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
            dict.insert_str("_closed", py_bool(true));
        }
    }

    for alias in ["SelectSelector", "PollSelector", "EpollSelector",
                  "KqueueSelector", "DevpollSelector"] {
        let _ = alias;
    }
    // (aliases wired after DefaultSelector is built below)
    /// Extract a keyword argument from the trailing packed-kwargs Dict that
    /// our call machinery appends (returns first positional at `pos` if it's
    /// not a dict).
    fn sel_kw(args: &[PyObjectRef], pos: usize, name: &str) -> Option<PyObjectRef> {
        if let Some(a) = args.get(pos) {
            if let PyObject::Dict(dd) = &*a.borrow() {
                if let Ok(Some(v)) = dd.get(&py_str(name)) {
                    return Some(v);
                }
            }
        }
        None
    }

    d.insert_str("DefaultSelector", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "DefaultSelector".into(),
        func: move |_args| {
            let typ = SELECTOR_TYPE.with(|c| {
                if let Some(t) = &*c.borrow() {
                    return t.clone();
                }
                let bf = |name: &'static str, f: crate::object::BuiltinFunc| {
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: name.to_string(),
                        func: f,
                    })
                };
                let mut td: HashMap<String, PyObjectRef> = HashMap::new();

                td.insert("register".into(), bf("register", |args| {
                    ensure_open(&args[0])?;
                    if args.len() < 3 {
                        return Err(PyError::type_error(
                            "register expected at least 2 arguments, got {}",
                        ));
                    }
                    let fileobj = args[1].clone();
                    let events = args[2].as_i64().unwrap_or(0);
                    let data = args.get(3).cloned().unwrap_or_else(py_none);
                    if events == 0 {
                        return Err(PyError::ValueError(
                            "Invalid event mask".into(),
                        ));
                    }
                    let fd = sock_fd(&fileobj);
                    let reg = reg_of(&args[0]);
                    if let PyObject::Dict(r) = &*reg.borrow() {
                        if r.contains(&py_int(fd))? {
                            return Err(PyError::KeyError(fd.to_string()));
                        }
                    }
                    let key = make_key(fileobj.clone(), fd, events, data);
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        r.set(py_int(fd), key.clone())?;
                    }
                    Ok(key)
                }));

                td.insert("unregister".into(), bf("unregister", |args| {
                    ensure_open(&args[0])?;
                    let fd = sock_fd(&args[1]);
                    let reg = reg_of(&args[0]);
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        match r.remove(&py_int(fd))? {
                            k => return Ok(k),
                        }
                    }
                    Err(PyError::KeyError(fd.to_string()))
                }));

                td.insert("modify".into(), bf("modify", |args| {
                    ensure_open(&args[0])?;
                    if args.len() < 3 {
                        return Err(PyError::type_error("modify expected 3 arguments"));
                    }
                    let fd = sock_fd(&args[1]);
                    let reg = reg_of(&args[0]);
                    let old = if let PyObject::Dict(r) = &*reg.borrow() {
                        r.get(&py_int(fd))?
                    } else { None };
                    let old = old.ok_or_else(|| PyError::KeyError(fd.to_string()))?;
                    let fileobj = inst_get(&old, "fileobj").unwrap_or_else(py_none);
                    let events = args[2].as_i64().unwrap_or(0);
                    let data = args.get(3).cloned().unwrap_or_else(py_none);
                    let key = make_key(fileobj, fd, events, data);
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        r.set(py_int(fd), key.clone())?;
                    }
                    Ok(key)
                }));

                td.insert("select".into(), bf("select", |args| {
                    ensure_open(&args[0])?;
                    // positional OR kwargs form: select(timeout=t) packs into
                    // a trailing Dict.
                    let timeout = args
                        .get(1)
                        .and_then(|a| a.as_f64())
                        .or_else(|| sel_kw(args, 1, "timeout")
                            .and_then(|v| v.as_f64()));
                    let start = std::time::Instant::now();
                    loop {
                        let mut ready: Vec<PyObjectRef> = Vec::new();
                        let reg = reg_of(&args[0]);
                        let entries: Vec<(PyObjectRef, i64)> =
                            if let PyObject::Dict(r) = &*reg.borrow() {
                                r.items()
                                    .into_iter()
                                    .map(|(_k, key)| {
                                        let ev = inst_get(&key, "events")
                                            .and_then(|e| e.as_i64())
                                            .unwrap_or(0);
                                        (key, ev)
                                    })
                                    .collect()
                            } else { vec![] };
                        for (key, events) in entries {
                            let fileobj =
                                inst_get(&key, "fileobj").unwrap_or_else(py_none);
                            let mut ev = 0i64;
                            if events & 1 != 0 && obj_readable(&fileobj) { ev |= 1; }
                            if events & 2 != 0 { ev |= 2; }
                            if ev != 0 {
                                ready.push(py_tuple(vec![key, py_int(ev)]));
                            }
                        }
                        if !ready.is_empty() {
                            return Ok(py_list(ready));
                        }
                        if let Some(t) = timeout {
                            if t <= 0.0
                                || std::time::Instant::now().duration_since(start)
                                    .as_secs_f64() >= t
                            {
                                return Ok(py_list(vec![]));
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(4));
                        // Cooperative SIGALRM delivery point.
                        if let Some(err) = misc_alarm_check() {
                            return Err(err);
                        }
                    }
                }));

                td.insert("get_key".into(), bf("get_key", |args| {
                    ensure_open(&args[0])?;
                    let fd = sock_fd(&args[1]);
                    let reg = reg_of(&args[0]);
                    if let PyObject::Dict(r) = &*reg.borrow() {
                        if let Some(k) = r.get(&py_int(fd))? {
                            return Ok(k);
                        }
                    }
                    Err(PyError::KeyError(fd.to_string()))
                }));
                td.insert("close".into(), bf("close", |args| {
                    set_closed(&args[0]);
                    Ok(py_none())
                }));
                td.insert("__enter__".into(), bf("__enter__", |args| Ok(args[0].clone())));
                td.insert("__exit__".into(), bf("__exit__", |_a| Ok(py_bool(false))));
                td.insert("get_map".into(), bf("get_map", |args| Ok(reg_of(&args[0]))));

                let t = PyObjectRef::new(PyObject::Type {
                    name: "DefaultSelector".into(),
                    dict: Box::new(crate::object::str_map_to_typedict(td)),
                    bases: vec![],
                    mro: vec![],
                });
                *c.borrow_mut() = Some(t.clone());
                t
            });
            let mut attrs = AttrMap::new();
            attrs.insert_str("_reg", py_dict());
            Ok(PyObjectRef::new(PyObject::Instance { typ, dict: attrs }))
        },
    }));
    // Expose the SelectorKey TYPE itself (tests reference selectors.SelectorKey).
    let dummy_key = make_key(py_none(), 0, 0, py_none());
    if let PyObject::Instance { typ, .. } = &*dummy_key.borrow() {
        d.insert_str("SelectorKey", typ.clone());
    }

    // CPython aliases: every platform selector is the same implementation here.
    if let Some(default_sel) = d.get("DefaultSelector").cloned() {
        for alias in [
            "SelectSelector", "PollSelector", "EpollSelector",
            "KqueueSelector", "DevpollSelector",
        ] {
            d.insert_str(alias, default_sel.clone());
        }
    }
    d
}


// ── signal.alarm cooperative timer ─────────────────────────────────────
thread_local! {
    pub static ALARM_DEADLINE: std::cell::RefCell<Option<std::time::Instant>> =
        const { std::cell::RefCell::new(None) };
}

/// Set (sec>0), clear (sec==0) the alarm deadline; returns previous seconds
/// remaining (0 when none was armed).
pub fn misc_alarm_set(sec: f64) -> f64 {
    ALARM_DEADLINE.with(|d| {
        let mut d = d.borrow_mut();
        let prev_remaining = match *d {
            Some(deadline) => {
                let now = std::time::Instant::now();
                if deadline > now {
                    deadline.duration_since(now).as_secs_f64()
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        *d = if sec > 0.0 {
            Some(std::time::Instant::now() + std::time::Duration::from_secs_f64(sec))
        } else {
            None
        };
        prev_remaining
    })
}

/// Fire the SIGALRM handler if the alarm deadline has passed. Returns
/// Some(err) when the handler raised (caller should propagate).
pub fn misc_alarm_check() -> Option<crate::object::PyError> {
    let due = ALARM_DEADLINE.with(|d| match *d.borrow() {
        Some(dl) => std::time::Instant::now() >= dl,
        None => false,
    });
    if !due {
        return None;
    }
    ALARM_DEADLINE.with(|d| *d.borrow_mut() = None);
    let out = crate::object::with_vm_mut(|vm| crate::modules::invoke_signal_handler_impl(vm, 14).err());
    match out {
        Ok(inner) => inner,
        Err(e) => Some(e),
    }
}
