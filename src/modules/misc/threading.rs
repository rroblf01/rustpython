use crate::object::*;
use std::collections::HashMap;

pub fn create_threading_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! thr_func {
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

    // `threading._dangling` — real CPython's `WeakSet` of still-running
    // `Thread` objects that never got `.join()`ed. Was missing entirely
    // (`AttributeError`), breaking `Lib/test/support/threading_helper.py`'s
    // `threading_setup` (`len(threading._dangling)`, paired with `_thread.
    // _count()` above to snapshot/verify thread cleanup — used by many
    // tests' `setUpModule`, e.g. `test_urllib2_localnet.py`). Since
    // `Thread.start()` here always runs its target synchronously in-place
    // and never leaves anything "dangling", a permanently empty list is
    // behaviorally correct, not just a placeholder.
    d.insert_str("_dangling", py_list(vec![]));
    // `threading._CRLock` — real CPython's `_thread.RLock` class.
    // Missing entirely broke `test_threading.py` which checks
    // `threading._CRLock is not None`.
    d.insert_str("_CRLock", py_none());
    // `threading.local` — thread-local storage.
    // Missing entirely broke `test_threading_local.py`.
    d.insert_str(
        "local",
        PyObjectRef::new(PyObject::Type {
            name: "_thread._local".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        }),
    );

    // `threading.Thread` is a REAL subclassable `PyObject::Type`, not a
    // plain `BuiltinFunction` factory. It used to be a bare
    // `BuiltinFunction` (calling it directly produced a `PyObject::Thread`
    // native value) — that broke the extremely common
    // `class Worker(threading.Thread): ...` pattern (e.g. `test_ftplib.py`'s
    // `DummyFTPServer(asyncore.dispatcher, threading.Thread)`,
    // `test_poplib.py`/`test_smtplib.py`'s analogous dummy servers) two
    // ways: (1) `threading.Thread.__init__(self)`/`threading.Thread.start
    // (self)` — the standard cooperative-multiple-inheritance idiom of
    // calling an unbound base method — raised `AttributeError:
    // 'builtin_function_or_method' object has no attribute 'start'`,
    // because attribute access on a `BuiltinFunction` has no method table
    // at all; (2) even where that idiom wasn't used, a real `type`
    // subclass needs `Thread` to actually BE a class so the normal
    // instance/dict/MRO machinery (identical to any pure-Python base
    // class, e.g. `asyncore.dispatcher`) applies — overridden `run()`
    // resolves via ordinary attribute lookup, `self.active`/etc. live in
    // the same instance dict as every other attribute. State (`_target`,
    // `_args`, `_kwargs`, `_started`, `_finished`, `_result`, `name`,
    // `daemon`) is now stored directly as plain instance attributes rather
    // than a separate native `PyObject::Thread` value.
    let thread_dict: HashMap<String, PyObjectRef> = {
        let mut m: HashMap<String, PyObjectRef> = HashMap::new();
        m.insert(
            "__init__".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: thread_init,
            }),
        );
        m.insert(
            "start".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "start".to_string(),
                func: thread_start,
            }),
        );
        m.insert(
            "run".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "run".to_string(),
                func: thread_run,
            }),
        );
        m.insert(
            "join".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "join".to_string(),
                func: thread_join,
            }),
        );
        for alias in ["is_alive", "isAlive"] {
            m.insert(
                alias.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: alias.to_string(),
                    func: thread_is_alive,
                }),
            );
        }
        m.insert(
            "getName".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getName".to_string(),
                func: thread_get_name,
            }),
        );
        m.insert(
            "setName".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "setName".to_string(),
                func: thread_set_name,
            }),
        );
        m
    };
    let thread_type = PyObjectRef::new(PyObject::Type {
        name: "Thread".to_string(),
        dict: Box::new(str_map_to_typedict(thread_dict)),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *thread_type.borrow_mut() {
        *mro = vec![thread_type.clone()];
    }
    d.insert_str("Thread", thread_type);

    // threading.local() — per-thread storage. This interpreter's object
    // model (Rc<RefCell<PyObject>>) only ever runs Python code on one
    // thread at a time, so a plain instance with its own attribute dict
    // already has exactly the semantics real code depends on (each
    // instance's attributes are independent of any other instance's).
    thr_func!("local", |_| {
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "local".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        }))
    });

    thr_func!("Lock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(LockInner {
            lock: std::sync::atomic::AtomicBool::new(false),
        }));
        Ok(PyObjectRef::new(PyObject::Lock(inner)))
    });

    thr_func!("RLock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(RLockInner {
            owner: None,
            count: 0,
        }));
        Ok(PyObjectRef::new(PyObject::RLock(inner)))
    });

    // _PyRLock is an alias for RLock (used by threading module internals)
    thr_func!("_PyRLock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(RLockInner {
            owner: None,
            count: 0,
        }));
        Ok(PyObjectRef::new(PyObject::RLock(inner)))
    });

    thr_func!("Event", |_| {
        let inner = std::sync::Arc::new(EventInner {
            flag: std::sync::Mutex::new(false),
            condvar: std::sync::Condvar::new(),
        });
        Ok(PyObjectRef::new(PyObject::Event(inner)))
    });

    thr_func!("current_thread", |_| { Ok(py_str("MainThread")) });

    thr_func!("active_count", |_| { Ok(py_int(1)) });

    // Real CPython returns a unique-per-thread integer. This interpreter
    // only ever runs Python code on one thread at a time (see the `local()`
    // comment above), so a stable constant is correct and sufficient — real
    // code (e.g. asgiref's `_CVar`/`Local`) uses this purely to tag/compare
    // "am I still on the thread that stored this", never as a real handle.
    thr_func!("get_ident", |_| { Ok(py_int(1)) });

    thr_func!("get_native_id", |_| { Ok(py_int(1)) });

    // threading.Condition — synchronization primitive built on Lock.
    // Real `Condition(lock=None)`: when no lock is given, one is created
    // (an `RLock()`) rather than leaving `self._lock` unset — a bare
    // `with Condition(): ...` (extremely common) used to store `lock:
    // None`, which `sync::get`'s Condition arms have nothing to
    // acquire/release/delegate to.
    thr_func!("Condition", |args| {
        let lock = match args.first() {
            Some(l) if !matches!(&*l.borrow(), PyObject::Dict(_) | PyObject::None) => l.clone(),
            _ => PyObjectRef::new(PyObject::RLock(std::sync::Arc::new(std::sync::Mutex::new(
                RLockInner {
                    owner: None,
                    count: 0,
                },
            )))),
        };
        Ok(PyObjectRef::new(PyObject::ThreadCondition {
            lock: Some(lock),
            notified: std::sync::Arc::new(std::sync::Mutex::new(false)),
            condvar: std::sync::Arc::new(std::sync::Condvar::new()),
        }))
    });

    // threading.Semaphore — counting semaphore
    thr_func!("Semaphore", |args| {
        let value = args.first().and_then(|a| a.as_i64()).unwrap_or(1) as u32;
        Ok(PyObjectRef::new(PyObject::ThreadSemaphore {
            value: std::sync::Arc::new(std::sync::Mutex::new(value)),
            initial: value,
            bounded: false,
            condvar: std::sync::Arc::new(std::sync::Condvar::new()),
        }))
    });

    // threading.BoundedSemaphore — bounded counting semaphore: `release()`
    // raises `ValueError` if the count would exceed the initial value.
    thr_func!("BoundedSemaphore", |args| {
        let value = args.first().and_then(|a| a.as_i64()).unwrap_or(1) as u32;
        Ok(PyObjectRef::new(PyObject::ThreadSemaphore {
            value: std::sync::Arc::new(std::sync::Mutex::new(value)),
            initial: value,
            bounded: true,
            condvar: std::sync::Arc::new(std::sync::Condvar::new()),
        }))
    });

    // threading.Barrier — synchronization primitive for N threads
    thr_func!("Barrier", |args| {
        let parties = args.first().and_then(|a| a.as_i64()).unwrap_or(2) as u32;
        Ok(PyObjectRef::new(PyObject::ThreadBarrier {
            parties,
            count: std::sync::Arc::new(std::sync::Mutex::new(0u32)),
            broken: std::sync::Arc::new(std::sync::Mutex::new(false)),
            generation: std::sync::Arc::new(std::sync::Mutex::new(0u64)),
            condvar: std::sync::Arc::new(std::sync::Condvar::new()),
        }))
    });

    // threading.Timer — delayed function execution
    thr_func!("Timer", |args| {
        let interval = args.first().and_then(|a| a.as_f64()).unwrap_or(0.0);
        let func = args.get(1).cloned().unwrap_or_else(py_none);
        Ok(PyObjectRef::new(PyObject::ThreadTimer {
            interval,
            func,
            args: args
                .get(2..)
                .map(|a| a.iter().map(|a| a.clone()).collect())
                .unwrap_or_default(),
            cancelled: std::sync::Arc::new(std::sync::Mutex::new(false)),
            started: std::sync::Arc::new(std::sync::Mutex::new(false)),
            finished: std::sync::Arc::new(std::sync::Mutex::new(false)),
        }))
    });

    // `threading.BrokenBarrierError` is patched in by the caller (see
    // `vm/init.rs`, right after this dict is built) — real builtins aren't
    // resolvable from inside this function yet (this runs before
    // `set_builtins_ref`), the same reason `select`'s `error` alias is
    // patched in at the call site rather than here.
    d
}

fn thread_dict_get(self_obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    if let PyObject::Instance { dict, .. } = &*self_obj.borrow() {
        dict.get_str(name).cloned()
    } else {
        None
    }
}

fn thread_dict_set(self_obj: &PyObjectRef, name: &str, val: PyObjectRef) {
    if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
        dict.insert_str(name, val);
    }
}

/// `Thread.__init__(self, group=None, target=None, name=None, args=(),
/// kwargs=None, *, daemon=None)`. See the `thread_type` construction site's
/// doc comment for why this whole type moved off a bare `PyObject::Thread`
/// native value.
fn thread_init(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "__init__() missing 1 required positional argument: 'self'",
        ));
    }
    let self_obj = args[0].clone();
    let (positional, kwargs) = match args.last() {
        Some(last) if args.len() > 1 && matches!(&*last.borrow(), PyObject::Dict(_)) => {
            (&args[1..args.len() - 1], Some(last.clone()))
        }
        _ => (&args[1..], None),
    };
    let kwarg = |name: &str| -> Option<PyObjectRef> {
        kwargs.as_ref().and_then(|d| {
            if let PyObject::Dict(d) = &*d.borrow() {
                d.get(&py_str(name)).ok().flatten()
            } else {
                None
            }
        })
    };
    // positional layout (after self): group, target, name, args, kwargs
    let target = kwarg("target")
        .or_else(|| positional.get(1).cloned())
        .unwrap_or_else(py_none);
    let name = kwarg("name").or_else(|| positional.get(2).cloned());
    let args_tuple = kwarg("args").or_else(|| positional.get(3).cloned());
    let kwargs_dict = kwarg("kwargs").or_else(|| positional.get(4).cloned());
    let daemon = kwarg("daemon");

    thread_dict_set(&self_obj, "_target", target);
    thread_dict_set(
        &self_obj,
        "_args",
        args_tuple.unwrap_or_else(|| py_tuple(vec![])),
    );
    thread_dict_set(
        &self_obj,
        "_kwargs",
        kwargs_dict.unwrap_or_else(|| PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new())))),
    );
    thread_dict_set(&self_obj, "name", name.unwrap_or_else(|| py_str("Thread")));
    thread_dict_set(
        &self_obj,
        "daemon",
        daemon.unwrap_or_else(|| py_bool(false)),
    );
    thread_dict_set(&self_obj, "_started", py_bool(false));
    thread_dict_set(&self_obj, "_finished", py_bool(false));
    thread_dict_set(&self_obj, "_result", py_none());
    Ok(py_none())
}

/// Default `Thread.run()`: calls `self._target(*self._args, **self._kwargs)`
/// if a target was given (matches real CPython). Subclasses commonly
/// override `run()` entirely (e.g. `test_ftplib.py`'s `DummyFTPServer.run`)
/// — ordinary attribute lookup on `self` already honors that override, this
/// is only reached for the un-overridden base case.
fn thread_run(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = &args[0];
    let target = thread_dict_get(self_obj, "_target").unwrap_or_else(py_none);
    if matches!(&*target.borrow(), PyObject::None) {
        return Ok(py_none());
    }
    let targs = thread_dict_get(self_obj, "_args").unwrap_or_else(|| py_tuple(vec![]));
    let mut call_args: Vec<PyObjectRef> = match &*targs.borrow() {
        PyObject::Tuple(items) => items.clone(),
        _ => vec![],
    };
    if let Some(kw) = thread_dict_get(self_obj, "_kwargs") {
        if let PyObject::Dict(d) = &*kw.borrow() {
            if !d.is_empty() {
                call_args.push(kw.clone());
            }
        }
    }
    crate::object::builtin_call(&target, &call_args)
}

fn thread_start(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("start() missing 'self'"));
    }
    let self_obj = args[0].clone();
    let already_started = thread_dict_get(&self_obj, "_started")
        .map(|v| v.truthy())
        .unwrap_or(false);
    if already_started {
        return Err(PyError::runtime_error("threads can only be started once"));
    }
    thread_dict_set(&self_obj, "_started", py_bool(true));
    let self_clone = self_obj.clone();
    crate::modules::coop_threads_enqueue(Box::new(move || {
        let run_method = self_clone.borrow().get_attribute("run");
        let call_result = match run_method {
            Ok(m) => crate::object::call_bound_method(m, self_clone.clone(), vec![]),
            Err(e) => Err(e),
        };
        match call_result {
            Ok(v) => thread_dict_set(&self_clone, "_result", v),
            Err(e) => {
                if !crate::object::is_stop_iteration_error(&e) {
                    eprintln!("Thread raised: {}", e);
                }
            }
        }
        thread_dict_set(&self_clone, "_finished", py_bool(true));
    }));
    Ok(py_none())
}

fn thread_join(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Extract timeout from keyword args (trailing PyDict) or positional args.
    // CPython's Thread.join(timeout=None) — timeout defaults to None.
    let mut timeout_val: Option<f64> = None;
    // Check for kwargs dict as last arg
    if let Some(last) = args.last() {
        if let PyObject::Dict(d) = &*last.borrow() {
            if let Ok(Some(val)) = d.get(&py_str("timeout")) {
                if !matches!(&*val.borrow(), PyObject::None) {
                    timeout_val = val.as_f64();
                }
            }
        }
    }
    // Check positional: args[1] if no kwargs dict
    if timeout_val.is_none() && args.len() > 1 {
        let second = &args[1];
        if !matches!(&*second.borrow(), PyObject::None | PyObject::Dict(_)) {
            timeout_val = second.as_f64();
        }
    }
    // Cooperative scheduling: a thread body only runs when the queue is
    // drained, so `join(timeout)` must drain too. Returning early on a
    // positive timeout (as this used to) meant the body NEVER ran and
    // `is_alive()` stayed True forever — which is what made
    // test_enum's two `@requires_working_threading` tests spin until the
    // runner's 15-minute thread-join watchdog fired. There is no real
    // wall-clock wait here: draining runs the body to completion, after
    // which the (real) timeout is irrelevant.
    let _ = timeout_val;
    crate::modules::coop_threads_drain();
    Ok(py_none())
}

fn thread_is_alive(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_bool(false));
    }
    let self_obj = &args[0];
    let started = thread_dict_get(self_obj, "_started")
        .map(|v| v.truthy())
        .unwrap_or(false);
    let finished = thread_dict_get(self_obj, "_finished")
        .map(|v| v.truthy())
        .unwrap_or(false);
    Ok(py_bool(started && !finished))
}

fn thread_get_name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_str("Thread"));
    }
    Ok(thread_dict_get(&args[0], "name").unwrap_or_else(|| py_str("Thread")))
}

fn thread_set_name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("setName() missing 'name' argument"));
    }
    thread_dict_set(&args[0], "name", args[1].clone());
    Ok(py_none())
}
