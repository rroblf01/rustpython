// Extracted from src/object/attrs/mod.rs — Globals + Lock/RLock/Event/Queue
use crate::object::*;
use super::*;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Globals(_) => {
                fn globals_key<'a>(args: &'a [PyObjectRef], i: usize) -> Result<crate::interner::StrId, PyError> {
                    match &*args[i].borrow() {
                        PyObject::Str(s) => Ok(crate::interner::intern(s.as_str())),
                        _ => Err(PyError::key_error(args[i].str())),
                    }
                }
                match name {
                    "keys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "keys".to_string(),
                        func: |args| {
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let keys: Vec<PyObjectRef> = g
                                    .borrow()
                                    .keys()
                                    .map(|k| py_str(crate::interner::lookup_str(*k)))
                                    .collect();
                                Ok(py_list(keys))
                            } else if let PyObject::Dict(_d) = &*args[0].borrow() {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_keys",
                                    args[0].clone(),
                                ));
                            } else {
                                Err(PyError::runtime_error("keys on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "values" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "values".to_string(),
                        func: |args| {
                            if let PyObject::Dict(_d) = &*args[0].borrow() {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_values",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*args[0].borrow() {
                                let values: Vec<PyObjectRef> =
                                    g.borrow().values().cloned().collect();
                                Ok(py_list(values))
                            } else {
                                Err(PyError::runtime_error("values on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "items" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "items".to_string(),
                        func: |args| {
                            if let PyObject::Dict(_d) = &*args[0].borrow() {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_items",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*args[0].borrow() {
                                let items: Vec<PyObjectRef> = g
                                    .borrow()
                                    .iter()
                                    .map(|(k, v)| {
                                        py_tuple(vec![
                                            py_str(crate::interner::lookup_str(*k)),
                                            v.clone(),
                                        ])
                                    })
                                    .collect();
                                Ok(py_list(items))
                            } else {
                                Err(PyError::runtime_error("items on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("get() takes at least 1 argument"));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let key = globals_key(args, 1)?;
                                Ok(g.borrow().get(&key).cloned().unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(d.get(&args[1])?.unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else {
                                Err(PyError::runtime_error("get on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "setdefault" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setdefault".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "setdefault() takes at least 1 argument",
                                ));
                            }
                            let default = if args.len() > 2 {
                                args[2].clone()
                            } else {
                                py_none()
                            };
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let key = globals_key(args, 1)?;
                                let mut map = g.borrow_mut();
                                if let Some(v) = map.get(&key) {
                                    return Ok(v.clone());
                                }
                                map.insert(key, default.clone());
                                Ok(default)
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    match d.get(&args[1])? {
                                        Some(v) => Ok(v),
                                        None => {
                                            d.set(args[1].clone(), default.clone())?;
                                            Ok(default)
                                        }
                                    }
                                } else {
                                    unreachable!()
                                }
                            } else {
                                Err(PyError::runtime_error("setdefault on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("pop() takes at least 1 argument"));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let key = globals_key(args, 1)?;
                                match g.borrow_mut().remove(&key) {
                                    Some(v) => Ok(v),
                                    None => {
                                        if args.len() > 2 {
                                            Ok(args[2].clone())
                                        } else {
                                            Err(PyError::key_error(args[1].str()))
                                        }
                                    }
                                }
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    match d.remove(&args[1]) {
                                        Ok(val) => Ok(val),
                                        Err(_) if args.len() > 2 => Ok(args[2].clone()),
                                        Err(e) => Err(e),
                                    }
                                } else {
                                    unreachable!()
                                }
                            } else {
                                Err(PyError::runtime_error("pop on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "popitem" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "popitem".to_string(),
                        func: |args| {
                            if args.len() > 1 {
                                return Err(PyError::type_error(format!(
                                    "dict.popitem() takes no arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let mut map = g.borrow_mut();
                                let first = map.iter().next().map(|(k, v)| {
                                    (
                                        *k,
                                        py_str(crate::interner::lookup_str(*k)),
                                        v.clone(),
                                    )
                                });
                                if let Some((key, kobj, v)) = first {
                                    map.remove(&key);
                                    Ok(py_tuple(vec![kobj, v]))
                                } else {
                                    Err(PyError::key_error(
                                        "popitem(): dictionary is empty".to_string(),
                                    ))
                                }
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    let items = d.items();
                                    if items.is_empty() {
                                        return Err(PyError::key_error(
                                            "popitem(): dictionary is empty".to_string(),
                                        ));
                                    }
                                    let (k, v) = items.into_iter().last().unwrap();
                                    d.remove(&k)?;
                                    Ok(py_tuple(vec![k, v]))
                                } else {
                                    unreachable!()
                                }
                            } else {
                                Err(PyError::runtime_error("popitem on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                g.borrow_mut().clear();
                                Ok(py_none())
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    d.clear();
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("clear on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let mut d = crate::object::PyDict::new();
                                for (k, v) in g.borrow().iter() {
                                    d.set(py_str(crate::interner::lookup_str(*k)), v.clone())?;
                                }
                                Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
                            } else if let PyObject::Dict(src) = &*args[0].borrow() {
                                Ok(PyObjectRef::new(PyObject::Dict(Box::new((**src).clone()))))
                            } else {
                                Err(PyError::runtime_error("copy on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "update".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "update() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let src = args[1].borrow();
                                match &*src {
                                    PyObject::Dict(d) => {
                                        let mut map = g.borrow_mut();
                                        for (k, v) in d.items() {
                                            if let PyObject::Str(s) = &*k.borrow() {
                                                map.insert(
                                                    crate::interner::intern(s.as_str()),
                                                    v,
                                                );
                                            }
                                        }
                                        Ok(py_none())
                                    }
                                    PyObject::Globals(other) => {
                                        let pairs: Vec<(String, PyObjectRef)> = other
                                            .borrow()
                                            .iter()
                                            .map(|(k, v)| {
                                                (
                                                    crate::interner::lookup_str(*k).to_string(),
                                                    v.clone(),
                                                )
                                            })
                                            .collect();
                                        drop(src);
                                        let mut map = g.borrow_mut();
                                        for (k, v) in pairs {
                                            map.insert(crate::interner::intern(&k), v);
                                        }
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::type_error(
                                        "update() argument must be a dict".to_string(),
                                    )),
                                }
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut db = args[0].borrow_mut();
                                let dst = match &mut *db {
                                    PyObject::Dict(d) => d,
                                    _ => unreachable!(),
                                };
                                let src = args[1].borrow();
                                match &*src {
                                    PyObject::Dict(d) => {
                                        for (k, v) in d.items() {
                                            dst.set(k.clone(), v)?;
                                        }
                                        Ok(py_none())
                                    }
                                    PyObject::Globals(other) => {
                                        let pairs: Vec<(PyObjectRef, PyObjectRef)> = other
                                            .borrow()
                                            .iter()
                                            .map(|(k, v)| {
                                                (
                                                    py_str(crate::interner::lookup_str(*k)),
                                                    v.clone(),
                                                )
                                            })
                                            .collect();
                                        drop(src);
                                        for (k, v) in pairs {
                                            dst.set(k, v)?;
                                        }
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::type_error(
                                        "update() argument must be a dict".to_string(),
                                    )),
                                }
                            } else {
                                Err(PyError::runtime_error("update on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'dict' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Lock(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                if locked
                                    .lock
                                    .compare_exchange(
                                        false,
                                        true,
                                        std::sync::atomic::Ordering::SeqCst,
                                        std::sync::atomic::Ordering::SeqCst,
                                    )
                                    .is_err()
                                {
                                    // Contended: run deferred threads once (they may
                                    // release), then retry; otherwise report deadlock
                                    // instead of spinning forever.
                                    drop(locked);
                                    crate::modules::coop_threads_drain();
                                    let locked = inner_arc.lock().unwrap();
                                    if locked
                                        .lock
                                        .compare_exchange(
                                            false,
                                            true,
                                            std::sync::atomic::Ordering::SeqCst,
                                            std::sync::atomic::Ordering::SeqCst,
                                        )
                                        .is_err()
                                    {
                                        return Err(PyError::runtime_error(
                                            "lock acquire deadlock in single-threaded interpreter",
                                        ));
                                    }
                                } else {
                                    locked.lock.store(true, std::sync::atomic::Ordering::SeqCst);
                                }
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                locked
                                    .lock
                                    .store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `acquire(blocking=True, timeout=-1)` — the old body
                    // ignored BOTH kwargs entirely and always spun on the
                    // atomic flag forever (`while locked.load() { yield_now() }`
                    // with no exit condition beyond the flag itself
                    // clearing). Since this interpreter runs everything
                    // SYNCHRONOUSLY (no real OS threads backing Python-level
                    // threads), nothing else can ever run concurrently to
                    // release an already-held lock — so re-acquiring a lock
                    // already held by "this" logical flow is a hard,
                    // permanent deadlock unless `blocking=False` or a
                    // `timeout` bounds the wait. Confirmed hanging via
                    // `Lib/test/lock_tests.py`'s `test_state_after_timeout`
                    // (`lock.acquire(); lock.acquire(timeout=0.01)`).
                    "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "acquire".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let kwargs = args.last().and_then(|a| {
                                    if let PyObject::Dict(d) = &*a.borrow() {
                                        Some((**d).clone())
                                    } else {
                                        None
                                    }
                                });
                                let get_kw = |name: &str| -> Option<PyObjectRef> {
                                    kwargs
                                        .as_ref()
                                        .and_then(|d| d.get(&py_str(name)).ok().flatten())
                                };
                                let is_kwargs_dict =
                                    |v: &PyObjectRef| matches!(&*v.borrow(), PyObject::Dict(_));
                                let blocking = get_kw("blocking")
                                    .or_else(|| args.get(1).filter(|a| !is_kwargs_dict(a)).cloned())
                                    .map(|v| v.truthy())
                                    .unwrap_or(true);
                                let timeout = get_kw("timeout")
                                    .or_else(|| args.get(2).filter(|a| !is_kwargs_dict(a)).cloned())
                                    .and_then(|v| v.as_f64());
                                let locked = inner_arc.lock().unwrap();
                                let try_take = || -> bool {
                                    if locked.lock.load(std::sync::atomic::Ordering::SeqCst) {
                                        false
                                    } else {
                                        locked
                                            .lock
                                            .store(true, std::sync::atomic::Ordering::SeqCst);
                                        true
                                    }
                                };
                                if !blocking {
                                    return Ok(py_bool(try_take()));
                                }
                                if let Some(t) = timeout.filter(|t| *t >= 0.0) {
                                    let deadline = std::time::Instant::now()
                                        + std::time::Duration::from_secs_f64(t);
                                    loop {
                                        if try_take() {
                                            return Ok(py_bool(true));
                                        }
                                        if std::time::Instant::now() >= deadline {
                                            return Ok(py_bool(false));
                                        }
                                        std::thread::yield_now();
                                    }
                                }
                                while !try_take() {
                                    std::thread::yield_now();
                                }
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "release".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                locked
                                    .lock
                                    .store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "locked" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "locked".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                return Ok(py_bool(
                                    locked.lock.load(std::sync::atomic::Ordering::SeqCst),
                                ));
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'lock' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::RLock(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "acquire".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if let Some(owner) = inner.owner {
                                    if owner == current_id {
                                        inner.count += 1;
                                        return Ok(py_bool(true));
                                    }
                                }
                                // Spin waiting for lock
                                while inner.owner.is_some() {
                                    drop(inner);
                                    std::thread::yield_now();
                                    inner = inner_arc.lock().unwrap();
                                }
                                inner.owner = Some(current_id);
                                inner.count = 1;
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "release".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if inner.owner != Some(current_id) {
                                    return Err(PyError::runtime_error(
                                        "cannot release un-acquired lock",
                                    ));
                                }
                                inner.count -= 1;
                                if inner.count == 0 {
                                    inner.owner = None;
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if let Some(owner) = inner.owner {
                                    if owner == current_id {
                                        inner.count += 1;
                                        return Ok(args[0].clone());
                                    }
                                }
                                while inner.owner.is_some() {
                                    drop(inner);
                                    std::thread::yield_now();
                                    inner = inner_arc.lock().unwrap();
                                }
                                inner.owner = Some(current_id);
                                inner.count = 1;
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if inner.owner != Some(current_id) {
                                    return Err(PyError::runtime_error(
                                        "cannot release un-acquired lock",
                                    ));
                                }
                                inner.count -= 1;
                                if inner.count == 0 {
                                    inner.owner = None;
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'RLock' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Event(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "is_set" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "is_set".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let flag = inner_arc.flag.lock().unwrap();
                                return Ok(py_bool(*flag));
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "set" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "set".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let mut flag = inner_arc.flag.lock().unwrap();
                                *flag = true;
                                inner_arc.condvar.notify_all();
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let mut flag = inner_arc.flag.lock().unwrap();
                                *flag = false;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "wait" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "wait".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                // Cooperative scheduling: first run any deferred
                                // thread bodies (they may set() the event), then
                                // report the flag. If the pending queue is empty
                                // and the event is still unset, NOTHING left in
                                // this single-threaded interpreter can ever set
                                // it -- spinning here would deadlock against the
                                // very continuation that would call set()
                                // (bpo-17141-style), so return the current flag.
                                crate::modules::coop_threads_drain();
                                let flag = inner_arc.flag.lock().unwrap();
                                if !*flag && crate::modules::coop_blocked_forever() {
                                    // Deferred body blocked on an event that
                                    // nothing left can set: unwind this body.
                                    drop(flag);
                                    return Err(PyError::StopIteration);
                                }
                                return Ok(py_bool(*flag));
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'Event' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Queue(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "put" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "put".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let item = args
                                    .get(1)
                                    .cloned()
                                    .ok_or_else(|| PyError::type_error("put() missing argument"))?;
                                let mut q = inner_arc.lock().unwrap();
                                q.queue.push_back(item);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let mut q = inner_arc.lock().unwrap();
                                // Cooperative scheduling: an empty queue may be
                                // waiting on a deferred producer thread.
                                if q.queue.is_empty() {
                                    drop(q);
                                    crate::modules::coop_threads_drain();
                                    q = inner_arc.lock().unwrap();
                                }
                                return q
                                    .queue
                                    .pop_front()
                                    .ok_or_else(|| PyError::runtime_error("empty queue"));
                            }
                            Err(PyError::runtime_error("not a Queue"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "qsize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "qsize".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let q = inner_arc.lock().unwrap();
                                return Ok(py_int(q.queue.len() as i64));
                            }
                            Ok(py_int(0))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'Queue' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
