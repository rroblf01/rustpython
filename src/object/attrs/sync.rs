// Extracted from src/object/attrs/mod.rs — Globals + Lock/RLock/Event/Queue
use super::*;
use crate::object::*;

/// Mirrors `_thread.TIMEOUT_MAX` (`src/modules/misc/thread.rs`) — the
/// largest timeout CPython's lock/semaphore `acquire()` accepts before
/// raising `OverflowError`. Kept as a separate constant here (rather than
/// importing the module's value) since this file has no dependency on
/// `modules::misc::thread`.
const THREAD_TIMEOUT_MAX: f64 = 4294967.0;

// ── Shared helpers for Condition/Semaphore/Barrier/Timer ────────────────
//
// `BuiltinMethod.func` is a plain `fn(&[PyObjectRef]) -> PyResult<...>`
// pointer (see `object/core/small.rs`'s `BuiltinFunc`), not a capturing
// closure — every closure below re-derives whatever it needs from `args`
// itself rather than capturing from the enclosing `match` arm.

/// Split a builtin-method call's trailing args into `(positional, kwargs)`,
/// mirroring `Lock::acquire`'s own extraction: this project's calling
/// convention appends a trailing `Dict` of keyword arguments rather than
/// passing them separately.
fn split_kwargs(args: &[PyObjectRef]) -> (&[PyObjectRef], Option<PyObjectRef>) {
    match args.last() {
        Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
            (&args[..args.len() - 1], Some(last.clone()))
        }
        _ => (args, None),
    }
}

fn get_kwarg(kwargs: &Option<PyObjectRef>, name: &str) -> Option<PyObjectRef> {
    kwargs.as_ref().and_then(|d| {
        if let PyObject::Dict(d) = &*d.borrow() {
            d.get(&py_str(name)).ok().flatten()
        } else {
            None
        }
    })
}

/// Try to take a `Lock`/`RLock` once, without blocking. Used to let
/// `Condition` delegate to whatever lock object it wraps (a real `Lock` or
/// `RLock`, per real `threading.Condition`'s own "export the lock's
/// acquire/release" convention) without needing a live `&mut
/// VirtualMachine` to make a generic method call.
fn generic_lock_try_acquire(lock_ref: &PyObjectRef) -> bool {
    match &*lock_ref.borrow() {
        PyObject::Lock(inner_arc) => {
            let locked = inner_arc.lock().unwrap();
            if locked.lock.load(std::sync::atomic::Ordering::SeqCst) {
                false
            } else {
                locked.lock.store(true, std::sync::atomic::Ordering::SeqCst);
                true
            }
        }
        PyObject::RLock(inner_arc) => {
            let mut inner = inner_arc.lock().unwrap();
            let current_id = std::thread::current().id();
            if let Some(owner) = inner.owner {
                if owner == current_id {
                    inner.count += 1;
                    return true;
                }
                false
            } else {
                inner.owner = Some(current_id);
                inner.count = 1;
                true
            }
        }
        _ => true,
    }
}

fn generic_lock_release(lock_ref: &PyObjectRef) {
    match &*lock_ref.borrow() {
        PyObject::Lock(inner_arc) => {
            let locked = inner_arc.lock().unwrap();
            locked
                .lock
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        PyObject::RLock(inner_arc) => {
            let mut inner = inner_arc.lock().unwrap();
            if inner.count > 0 {
                inner.count -= 1;
                if inner.count == 0 {
                    inner.owner = None;
                }
            }
        }
        _ => {}
    }
}

fn generic_lock_locked(lock_ref: &PyObjectRef) -> bool {
    match &*lock_ref.borrow() {
        PyObject::Lock(inner_arc) => inner_arc
            .lock()
            .unwrap()
            .lock
            .load(std::sync::atomic::Ordering::SeqCst),
        PyObject::RLock(inner_arc) => inner_arc.lock().unwrap().owner.is_some(),
        _ => false,
    }
}

/// Blocking acquire on an arbitrary wrapped `Lock`/`RLock`, following the
/// same cooperative-scheduling policy as `Lock::acquire`/`Event::wait`
/// elsewhere in this file: try, drain any deferred thread bodies once
/// (they may release it), retry, and only then give up — a deadlock error
/// for a top-level caller, an internal unwind for a deferred body with
/// nothing left pending (mirrors `coop_blocked_forever`'s existing use in
/// `Event::wait`).
fn generic_lock_acquire(
    lock_ref: &PyObjectRef,
    blocking: bool,
    timeout: Option<f64>,
) -> PyResult<bool> {
    // CPython: a non-blocking acquire cannot also carry a real timeout —
    // `lock.acquire(False, 1)` raises `ValueError: can't specify a timeout
    // for a non-blocking call` (the sentinel "no timeout given" is -1, so
    // that value alone is still allowed through). Was previously accepted
    // silently and just fell through to the `!blocking` early return below.
    if !blocking {
        if let Some(t) = timeout {
            if t != -1.0 {
                return Err(PyError::value_error(
                    "can't specify a timeout for a non-blocking call",
                ));
            }
        }
    } else if let Some(t) = timeout {
        // CPython: any timeout other than the "none given" sentinel -1
        // must be non-negative — `lock.acquire(timeout=-100)` raises
        // ValueError rather than being treated as "no timeout".
        if t < 0.0 && t != -1.0 {
            return Err(PyError::value_error(
                "timeout value must be a non-negative number",
            ));
        }
        // CPython caps timeouts at `_thread.TIMEOUT_MAX` and raises
        // OverflowError above it (`lock.acquire(timeout=1e100)`) — without
        // this check the value flowed straight into
        // `Duration::from_secs_f64`, which PANICS (aborting the whole
        // process, not a catchable Python exception) once the value is too
        // big to represent as a `Duration`.
        if t > THREAD_TIMEOUT_MAX {
            return Err(PyError::overflow_error("timeout value is too large"));
        }
    }
    if generic_lock_try_acquire(lock_ref) {
        return Ok(true);
    }
    if !blocking {
        return Ok(false);
    }
    if let Some(t) = timeout.filter(|t| *t >= 0.0) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(t);
        loop {
            crate::modules::coop_threads_drain();
            if generic_lock_try_acquire(lock_ref) {
                return Ok(true);
            }
            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::yield_now();
        }
    }
    crate::modules::coop_threads_drain();
    if generic_lock_try_acquire(lock_ref) {
        return Ok(true);
    }
    if crate::modules::coop_blocked_forever() {
        return Err(PyError::StopIteration);
    }
    Err(PyError::runtime_error(
        "lock acquire deadlock in single-threaded interpreter",
    ))
}

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
        PyObject::Globals(_) => {
            fn globals_key<'a>(
                args: &'a [PyObjectRef],
                i: usize,
            ) -> Result<crate::interner::StrId, PyError> {
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
                            let values: Vec<PyObjectRef> = g.borrow().values().cloned().collect();
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
                                (*k, py_str(crate::interner::lookup_str(*k)), v.clone())
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
                        if args.len() != 1 {
                            return Err(PyError::type_error(format!(
                                "clear() takes no arguments ({} given)",
                                args.len() - 1
                            )));
                        }
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
                        if args.len() != 1 {
                            return Err(PyError::type_error(format!(
                                "copy() takes no arguments ({} given)",
                                args.len() - 1
                            )));
                        }
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
                            return Err(PyError::type_error("update() takes at least 1 argument"));
                        }
                        if let PyObject::Globals(g) = &*args[0].borrow() {
                            let src = args[1].borrow();
                            match &*src {
                                PyObject::Dict(d) => {
                                    let mut map = g.borrow_mut();
                                    for (k, v) in d.items() {
                                        if let PyObject::Str(s) = &*k.borrow() {
                                            map.insert(crate::interner::intern(s.as_str()), v);
                                        }
                                    }
                                    Ok(py_none())
                                }
                                PyObject::Globals(other) => {
                                    let pairs: Vec<(String, PyObjectRef)> = other
                                        .borrow()
                                        .iter()
                                        .map(|(k, v)| {
                                            (crate::interner::lookup_str(*k).to_string(), v.clone())
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
                                            (py_str(crate::interner::lookup_str(*k)), v.clone())
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
                            // CPython: `acquire(False, timeout)` with a
                            // REAL timeout (anything but the "none given"
                            // sentinel -1) is a ValueError — a
                            // non-blocking call can't also wait.
                            if !blocking {
                                if let Some(t) = timeout {
                                    if t != -1.0 {
                                        return Err(PyError::value_error(
                                            "can't specify a timeout for a non-blocking call",
                                        ));
                                    }
                                }
                            } else if let Some(t) = timeout {
                                if t < 0.0 && t != -1.0 {
                                    return Err(PyError::value_error(
                                        "timeout value must be a non-negative number",
                                    ));
                                }
                                if t > THREAD_TIMEOUT_MAX {
                                    return Err(PyError::overflow_error(
                                        "timeout value is too large",
                                    ));
                                }
                            }
                            let locked = inner_arc.lock().unwrap();
                            let try_take = || -> bool {
                                if locked.lock.load(std::sync::atomic::Ordering::SeqCst) {
                                    false
                                } else {
                                    locked.lock.store(true, std::sync::atomic::Ordering::SeqCst);
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
                // Real `RLock._is_owned()` — used directly by
                // `test_contextlib.py::LockContextTestCase.
                // testWithRLock` and by `Condition.__init__`'s own
                // `hasattr(lock, '_is_owned')` delegation. Since this
                // interpreter only ever runs Python on one logical
                // flow at a time, "is the CURRENT thread the owner" and
                // "is ANY thread the owner" coincide in practice.
                "_is_owned" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "_is_owned".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::RLock(inner_arc) = &*obj {
                            return Ok(py_bool(inner_arc.lock().unwrap().owner.is_some()));
                        }
                        Ok(py_bool(false))
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
                // `Queue.empty()` / `Queue.full()` — same shape as `qsize`
                // (real trigger: test_sched.py's concurrent scheduler
                // tests assert `q.empty()` after each drain step). `full`
                // is always False here: this unbounded implementation has
                // no maxsize (matching SimpleQueue reuse below).
                "empty" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "empty".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::Queue(inner_arc) = &*obj {
                            let q = inner_arc.lock().unwrap();
                            return Ok(py_bool(q.queue.is_empty()));
                        }
                        Ok(py_bool(true))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "full" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "full".to_string(),
                    func: |_args| Ok(py_bool(false)),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'Queue' object has no attribute '{}'",
                    name
                ))),
            }
        }
        PyObject::ThreadCondition { lock, .. } => {
            let _lock = lock.clone();
            match name {
                "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "acquire".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadCondition {
                            lock: Some(lock), ..
                        } = &*obj
                        {
                            let lock = lock.clone();
                            let (positional, kwargs) = split_kwargs(&args[1..]);
                            let is_kwargs_dict =
                                |v: &PyObjectRef| matches!(&*v.borrow(), PyObject::Dict(_));
                            let blocking = get_kwarg(&kwargs, "blocking")
                                .or_else(|| {
                                    positional.first().filter(|a| !is_kwargs_dict(a)).cloned()
                                })
                                .map(|v| v.truthy())
                                .unwrap_or(true);
                            let timeout = get_kwarg(&kwargs, "timeout")
                                .or_else(|| {
                                    positional.get(1).filter(|a| !is_kwargs_dict(a)).cloned()
                                })
                                .and_then(|v| v.as_f64());
                            return Ok(py_bool(generic_lock_acquire(&lock, blocking, timeout)?));
                        }
                        Ok(py_bool(true))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "release".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadCondition {
                            lock: Some(lock), ..
                        } = &*obj
                        {
                            generic_lock_release(lock);
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "locked" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "locked".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadCondition {
                            lock: Some(lock), ..
                        } = &*obj
                        {
                            return Ok(py_bool(generic_lock_locked(lock)));
                        }
                        Ok(py_bool(false))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // Real `Condition.__init__` exports `self._is_owned =
                // lock._is_owned` when the wrapped lock defines one
                // (`RLock._is_owned`), else falls back to its OWN
                // try-acquire-then-release check (`Condition._is_owned`,
                // used for a plain `Lock`, which has no ownership
                // concept). `generic_lock_locked` already implements
                // exactly that fallback for both `Lock`/`RLock`, so one
                // arm covers both cases here. Needed so much as `with
                // Condition(): ...`-adjacent code checking `lock.
                // _is_owned()` directly (real trigger: CPython's own
                // `test_contextlib.py::LockContextTestCase.
                // testWithCondition`) works rather than AttributeError.
                "_is_owned" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "_is_owned".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadCondition {
                            lock: Some(lock), ..
                        } = &*obj
                        {
                            return Ok(py_bool(generic_lock_locked(lock)));
                        }
                        Ok(py_bool(false))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__enter__".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadCondition {
                            lock: Some(lock), ..
                        } = &*obj
                        {
                            generic_lock_acquire(lock, true, None)?;
                        }
                        Ok(args[0].clone())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__exit__".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadCondition {
                            lock: Some(lock), ..
                        } = &*obj
                        {
                            generic_lock_release(lock);
                        }
                        Ok(py_bool(false))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // Cooperative `wait()`: release the wrapped lock so any
                // deferred thread body waiting to acquire it can run,
                // drain the queue once (mirrors `Event::wait`'s own
                // policy), then reacquire before returning. Consumes
                // one pending `notify()` if present; otherwise, if we
                // are ourselves a deferred body with nothing left
                // pending, unwind (internal StopIteration) instead of
                // reporting a spurious wake.
                "wait" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "wait".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadCondition {
                            lock: Some(lock),
                            notified,
                            ..
                        } = &*obj
                        {
                            let lock = lock.clone();
                            let notified = notified.clone();
                            generic_lock_release(&lock);
                            crate::modules::coop_threads_drain();
                            let _ = generic_lock_acquire(&lock, true, None);
                            let mut flag = notified.lock().unwrap();
                            let was_notified = *flag;
                            if was_notified {
                                *flag = false;
                            }
                            drop(flag);
                            if !was_notified && crate::modules::coop_blocked_forever() {
                                return Err(PyError::StopIteration);
                            }
                            return Ok(py_bool(true));
                        }
                        Ok(py_bool(true))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // `wait_for(predicate, timeout=None)`: this interpreter
                // cannot really block, so it gives progress exactly one
                // chance (check, drain deferred bodies once, check
                // again) rather than looping — an unbounded loop here
                // would spin forever for a top-level caller whose
                // predicate never becomes true and nothing is pending.
                "wait_for" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "wait_for".to_string(),
                    func: |args| {
                        let predicate = args.get(1).cloned().ok_or_else(|| {
                            PyError::type_error("wait_for() missing required argument: 'predicate'")
                        })?;
                        let call_pred = |p: &PyObjectRef| crate::object::builtin_call(p, &[]);
                        let result = call_pred(&predicate)?;
                        if result.truthy() {
                            return Ok(result);
                        }
                        let lock = {
                            let obj = args[0].borrow();
                            if let PyObject::ThreadCondition {
                                lock: Some(lock), ..
                            } = &*obj
                            {
                                Some(lock.clone())
                            } else {
                                None
                            }
                        };
                        if let Some(lock) = &lock {
                            generic_lock_release(lock);
                            crate::modules::coop_threads_drain();
                            let _ = generic_lock_acquire(lock, true, None);
                        }
                        call_pred(&predicate)
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "notify" | "notify_all" | "notifyAll" => {
                    Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::ThreadCondition {
                                lock: Some(lock),
                                notified,
                                ..
                            } = &*obj
                            {
                                if !generic_lock_locked(lock) {
                                    return Err(PyError::runtime_error(
                                        "cannot notify on un-acquired lock",
                                    ));
                                }
                                *notified.lock().unwrap() = true;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    }))
                }
                _ => Err(PyError::attribute_error(format!(
                    "'Condition' object has no attribute '{}'",
                    name
                ))),
            }
        }
        PyObject::ThreadSemaphore { bounded, .. } => {
            let bounded = *bounded;
            match name {
                "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "acquire".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadSemaphore { value, .. } = &*obj {
                            let value = value.clone();
                            let (positional, kwargs) = split_kwargs(&args[1..]);
                            let is_kwargs_dict =
                                |v: &PyObjectRef| matches!(&*v.borrow(), PyObject::Dict(_));
                            let blocking = get_kwarg(&kwargs, "blocking")
                                .or_else(|| {
                                    positional.first().filter(|a| !is_kwargs_dict(a)).cloned()
                                })
                                .map(|v| v.truthy())
                                .unwrap_or(true);
                            let timeout = get_kwarg(&kwargs, "timeout")
                                .or_else(|| {
                                    positional.get(1).filter(|a| !is_kwargs_dict(a)).cloned()
                                })
                                .and_then(|v| v.as_f64());
                            let try_take = |v: &std::sync::Arc<std::sync::Mutex<u32>>| -> bool {
                                let mut n = v.lock().unwrap();
                                if *n > 0 {
                                    *n -= 1;
                                    true
                                } else {
                                    false
                                }
                            };
                            if try_take(&value) {
                                return Ok(py_bool(true));
                            }
                            if !blocking {
                                return Ok(py_bool(false));
                            }
                            if let Some(t) = timeout.filter(|t| *t >= 0.0) {
                                let deadline = std::time::Instant::now()
                                    + std::time::Duration::from_secs_f64(t);
                                loop {
                                    crate::modules::coop_threads_drain();
                                    if try_take(&value) {
                                        return Ok(py_bool(true));
                                    }
                                    if std::time::Instant::now() >= deadline {
                                        return Ok(py_bool(false));
                                    }
                                    std::thread::yield_now();
                                }
                            }
                            crate::modules::coop_threads_drain();
                            if try_take(&value) {
                                return Ok(py_bool(true));
                            }
                            if crate::modules::coop_blocked_forever() {
                                return Err(PyError::StopIteration);
                            }
                            return Ok(py_bool(false));
                        }
                        Ok(py_bool(true))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                // `release(n=1)` — `BoundedSemaphore` additionally
                // raises `ValueError` if the new count would exceed
                // the value it was constructed with (real CPython
                // semantics; plain `Semaphore` has no ceiling).
                "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "release".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadSemaphore {
                            value,
                            initial,
                            bounded,
                            ..
                        } = &*obj
                        {
                            let (positional, kwargs) = split_kwargs(&args[1..]);
                            let n = get_kwarg(&kwargs, "n")
                                .or_else(|| positional.first().cloned())
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1);
                            if n < 1 {
                                return Err(PyError::value_error("n must be one or more"));
                            }
                            let mut v = value.lock().unwrap();
                            let new_val = *v as i64 + n;
                            if *bounded && new_val > *initial as i64 {
                                return Err(PyError::value_error(
                                    "Semaphore released too many times",
                                ));
                            }
                            *v = new_val as u32;
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__enter__".to_string(),
                    func: |args| {
                        let obj = args[0].borrow();
                        if let PyObject::ThreadSemaphore { value, .. } = &*obj {
                            let try_take = || -> bool {
                                let mut n = value.lock().unwrap();
                                if *n > 0 {
                                    *n -= 1;
                                    true
                                } else {
                                    false
                                }
                            };
                            if !try_take() {
                                crate::modules::coop_threads_drain();
                                let _ = try_take();
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
                        if let PyObject::ThreadSemaphore {
                            value,
                            initial,
                            bounded,
                            ..
                        } = &*obj
                        {
                            let mut v = value.lock().unwrap();
                            let new_val = *v + 1;
                            if !(*bounded && new_val > *initial) {
                                *v = new_val;
                            }
                        }
                        Ok(py_bool(false))
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    if bounded {
                        "BoundedSemaphore"
                    } else {
                        "Semaphore"
                    },
                    name
                ))),
            }
        }
        PyObject::ThreadBarrier {
            parties,
            count,
            broken,
            ..
        } => match name {
            "parties" => Ok(py_int(*parties as i64)),
            "n_waiting" => Ok(py_int(*count.lock().unwrap() as i64)),
            "broken" => Ok(py_bool(*broken.lock().unwrap())),
            // Cooperative `wait()`: increment the arrival counter; the
            // call that completes the round (count reaches `parties`)
            // releases everyone by resetting the counter and returns
            // its arrival index immediately. Any earlier arrival drains
            // deferred thread bodies once to give the rest of the
            // party a chance to arrive (mirrors `Event::wait`'s single
            // drain-then-decide policy); if nothing is left pending to
            // ever complete the round, the barrier breaks.
            // NOTE on `generation`: after `coop_threads_drain()`, an
            // empty pending queue does NOT necessarily mean "nothing
            // can ever complete my round" — it may mean a LATER
            // arrival ran during that very drain and already completed
            // it (bumping `generation`). Only treat "queue empty" as
            // "genuinely stuck" when `generation` hasn't moved past
            // what we observed on arrival (verified via a 3-party
            // repro: `Barrier(3)` + two deferred `Thread`s + the main
            // flow all calling `wait()` — without this check, the
            // 2nd-of-3 arrival's own drain transitively runs the
            // 3rd-of-3 arrival to completion, then sees an empty queue
            // and incorrectly broke a barrier that had, in fact, just
            // released everyone).
            "wait" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "wait".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    if let PyObject::ThreadBarrier {
                        parties,
                        count,
                        broken,
                        generation,
                        ..
                    } = &*obj
                    {
                        if *broken.lock().unwrap() {
                            return Err(PyError::runtime_error("Barrier is broken"));
                        }
                        let my_gen = *generation.lock().unwrap();
                        let idx = {
                            let mut c = count.lock().unwrap();
                            let idx = *c;
                            *c += 1;
                            idx
                        };
                        if idx + 1 == *parties {
                            *count.lock().unwrap() = 0;
                            *generation.lock().unwrap() += 1;
                            return Ok(py_int(idx as i64));
                        }
                        let broken = broken.clone();
                        let generation = generation.clone();
                        crate::modules::coop_threads_drain();
                        if *broken.lock().unwrap() {
                            return Err(PyError::runtime_error("Barrier is broken"));
                        }
                        if *generation.lock().unwrap() != my_gen {
                            // A later arrival completed (and released)
                            // this same round while we drained.
                            return Ok(py_int(idx as i64));
                        }
                        if crate::modules::coop_blocked_forever() {
                            *broken.lock().unwrap() = true;
                            return Err(PyError::StopIteration);
                        }
                        return Ok(py_int(idx as i64));
                    }
                    Ok(py_int(0))
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "reset" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "reset".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    if let PyObject::ThreadBarrier {
                        count,
                        broken,
                        generation,
                        ..
                    } = &*obj
                    {
                        *count.lock().unwrap() = 0;
                        *broken.lock().unwrap() = false;
                        *generation.lock().unwrap() += 1;
                    }
                    Ok(py_none())
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "abort" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "abort".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    if let PyObject::ThreadBarrier { broken, .. } = &*obj {
                        *broken.lock().unwrap() = true;
                    }
                    Ok(py_none())
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            _ => Err(PyError::attribute_error(format!(
                "'Barrier' object has no attribute '{}'",
                name
            ))),
        },
        PyObject::ThreadTimer { .. } => match name {
            "start" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "start".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    if let PyObject::ThreadTimer {
                        func,
                        args: targs,
                        cancelled,
                        started,
                        finished,
                        ..
                    } = &*obj
                    {
                        {
                            let mut s = started.lock().unwrap();
                            if *s {
                                return Err(PyError::runtime_error(
                                    "threads can only be started once",
                                ));
                            }
                            *s = true;
                        }
                        let func = func.clone();
                        let targs = targs.clone();
                        let cancelled = cancelled.clone();
                        let finished = finished.clone();
                        crate::modules::coop_threads_enqueue(Box::new(move || {
                            if !*cancelled.lock().unwrap() {
                                if let Err(e) = crate::object::builtin_call(&func, &targs) {
                                    if !crate::object::is_stop_iteration_error(&e) {
                                        eprintln!("Thread raised: {}", e);
                                    }
                                }
                            }
                            *finished.lock().unwrap() = true;
                        }));
                    }
                    Ok(py_none())
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            // `cancel()` — "Stop the timer if it hasn't finished yet."
            // Real CPython's own `Timer.cancel()` sets the same
            // internal event its target checks before running, so
            // this also marks `finished` (not just `cancelled`) —
            // `is_alive()`/`join()` should see a cancelled-before-run
            // Timer as done, not perpetually pending.
            "cancel" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "cancel".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    if let PyObject::ThreadTimer {
                        cancelled,
                        finished,
                        ..
                    } = &*obj
                    {
                        *cancelled.lock().unwrap() = true;
                        *finished.lock().unwrap() = true;
                    }
                    Ok(py_none())
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "join".to_string(),
                func: |_args| {
                    crate::modules::coop_threads_drain();
                    Ok(py_none())
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "is_alive" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "is_alive".to_string(),
                func: |args| {
                    let obj = args[0].borrow();
                    if let PyObject::ThreadTimer {
                        started, finished, ..
                    } = &*obj
                    {
                        return Ok(py_bool(
                            *started.lock().unwrap() && !*finished.lock().unwrap(),
                        ));
                    }
                    Ok(py_bool(false))
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            _ => Err(PyError::attribute_error(format!(
                "'Timer' object has no attribute '{}'",
                name
            ))),
        },
        _ => unreachable!(
            "{} called with non-{} object",
            stringify!(get),
            o.type_name()
        ),
    }
}
