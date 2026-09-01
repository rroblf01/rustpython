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

    thr_func!("Thread", |args| {
        // Real `threading.Thread.__init__(self, group=None, target=None,
        // name=None, args=(), kwargs=None, *, daemon=None)` is overwhelmingly
        // called with `target`/`args` as KEYWORD arguments in real code
        // (`Thread(target=f, args=(1, 2))`) — this used to treat `args[0]`/
        // `args[1]` as ALWAYS being the positional `target`/`args`, so any
        // keyword-argument call packed its kwargs into a trailing `Dict`
        // (this project's own established calling convention) that got
        // mistaken for the target itself, then failing to CALL it with
        // `TypeError: 'dict' object is not callable` the moment the thread
        // actually ran — i.e. `threading.Thread` was completely broken for
        // the single most common way real code constructs one. Now checks
        // for a trailing kwargs dict first and pulls `target`/`args` out of
        // it if present, falling back to positional args only for whichever
        // of the two a kwarg didn't already supply.
        let (positional, kwargs) = match args.last() {
            Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
                (&args[..args.len() - 1], Some(last.clone()))
            }
            _ => (args, None),
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
        let target = kwarg("target")
            .or_else(|| positional.get(1).cloned())
            .unwrap_or_else(py_none);
        let args_tuple = kwarg("args").or_else(|| positional.get(3).cloned());
        let thread_args = match args_tuple {
            Some(t) => match &*t.borrow() {
                PyObject::Tuple(items) => items.clone(),
                _ => vec![],
            },
            None => vec![],
        };
        let inner = std::sync::Arc::new(std::sync::Mutex::new(ThreadInner {
            handle: None,
            result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            target,
            args: thread_args,
            started: false,
        }));
        Ok(PyObjectRef::new(PyObject::Thread(inner)))
    });

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

    d
}
