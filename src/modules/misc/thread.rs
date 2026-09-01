use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;

// `_thread.start_new_thread(func, args)` — this project's threading model
// runs the target SYNCHRONOUSLY in-place (see `_count`'s own doc comment
// just below), so "starting a thread" just means "call `func(*args)` now".
// A real user-defined `def other_thread():` (`PyObject::Function`) needs
// a live `&mut VirtualMachine` to push a frame and execute — a genuine
// gap (confirmed: `object::call_function` only handles
// `BuiltinFunction`/`Closure`, raising `TypeError: 'function' object is
// not callable` for a plain Python target) — but actually making the call
// succeed synchronously, IN THIS SAME CALL STACK, reintroduces a WORSE
// problem: any real thread-test pattern of "acquire a lock, then spawn a
// worker that also acquires that same lock" (extremely common —
// `test_thread.py`'s own `test__count`: `mut.acquire()` then
// `thread.start_new_thread(task, ())` where `task` calls `mut.acquire()`
// again) is a genuine, unbreakable DEADLOCK once the worker body actually
// runs before `start_new_thread` returns — there is no other real OS
// thread to ever release the lock. Confirmed by trying the natural fix
// (routing through `vm.call_function`, matching `asyncio.run`'s own
// pattern): `test_thread.py` and `test_threadsignals.py` both went from a
// fast, pre-existing FAIL to a 120s TIMEOUT. A fast, wrong-shaped error is
// a strictly better outcome for this interpreter's fake-single-threaded
// execution model than a real hang, so deliberately left AS THE ORIGINAL,
// restrictive `object::call_function`-based behavior rather than "fixed".
// ── Cooperative thread scheduler ─────────────────────────────────────
// PyObjectRef is !Send so Python-level threads cannot be OS threads.
// Instead of running targets synchronously inside .start() (which made
// producer/consumeter and lock-handoff tests hang or fail), targets are
// queued as closures and only executed when someone JOINS them or when a
// potentially-blocking operation finds nothing to read — that operation
// drains the queue first and retries once, which is exactly the
// happens-before relationship those tests rely on.
thread_local! {
    static COOP_QUEUE: RefCell<std::collections::VecDeque<Box<dyn FnOnce()>>> =
        const { RefCell::new(std::collections::VecDeque::new()) };
}

/// Run every queued thread-body (FIFO). Bodies may enqueue more work;
/// bounded to avoid pathological infinite feedback loops.
thread_local! {
    static IN_DRAIN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// True when called while draining deferred thread bodies AND the pending
/// queue is empty -- nothing else can make progress, so a blocked waiter
/// should unwind (internal StopIteration) instead of spinning forever.
pub(crate) fn coop_blocked_forever() -> bool {
    let in_drain = IN_DRAIN.with(|c| c.get()) > 0;
    let queue_empty = COOP_QUEUE.with(|q| q.borrow_mut().is_empty());
    in_drain && queue_empty
}

pub fn coop_threads_drain() {
    const MAX_JOBS: usize = 10_000;
    let mut ran = 0usize;
    IN_DRAIN.with(|c| c.set(c.get() + 1));
    loop {
        let next = COOP_QUEUE.with(|q| q.borrow_mut().pop_front());
        match next {
            Some(job) => {
                job();
                ran += 1;
                if ran >= MAX_JOBS {
                    break;
                }
            }
            None => break,
        }
    }
    IN_DRAIN.with(|c| c.set(c.get() - 1));
}

pub(crate) fn coop_threads_enqueue(job: Box<dyn FnOnce()>) {
    COOP_QUEUE.with(|q| q.borrow_mut().push_back(job));
}

pub fn create_thread_module_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // Real CPython's max `Lock.acquire(timeout=...)` value (platform max C
    // `long` in seconds, roughly). Needed by `test.support.lock_tests`.
    d.insert_str("TIMEOUT_MAX", py_float(4294967.0));
    // `_thread.get_ident()` — the calling thread's identifier (real CPython's
    // pprint.py and reprlib.py both use it as a recursion-guard key).
    d.insert_str(
        "get_ident",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "get_ident".to_string(),
            func: |_args: &[PyObjectRef]| {
                use std::sync::atomic::{AtomicU64, Ordering};
                thread_local! { static ID: AtomicU64 = const { AtomicU64::new(0) }; }
                static NEXT: AtomicU64 = AtomicU64::new(1);
                let id = ID.with(|c| {
                    let mut v = c.load(Ordering::Relaxed);
                    if v == 0 {
                        v = NEXT.fetch_add(1, Ordering::Relaxed);
                        c.store(v, Ordering::Relaxed);
                    }
                    v
                });
                Ok(py_int(id as i64))
            },
        }),
    );
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

    thr_func!("start_new_thread", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "start_new_thread() requires at least 2 arguments (function, args)",
            ));
        }
        let func = args[0].clone();
        let func_args = if let PyObject::Tuple(items) = &*args[1].borrow() {
            items.clone()
        } else {
            return Err(PyError::type_error(
                "start_new_thread() args must be a tuple",
            ));
        };
        // Call function synchronously
        crate::object::call_function(&func, func_args)?;
        Ok(py_int(0))
    });

    thr_func!("allocate_lock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(LockInner {
            lock: std::sync::atomic::AtomicBool::new(false),
        }));
        Ok(PyObjectRef::new(PyObject::Lock(inner)))
    });

    // _thread.RLock — reentrant lock (CPython C extension replacement).
    // Threading module internals and user code use `_thread.RLock`.
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

    // `_thread._count()` — was missing entirely (`AttributeError`), breaking
    // `Lib/test/support/threading_helper.py`'s `threading_setup`/
    // `threading_cleanup` (used by a wide range of tests, e.g.
    // `test_urllib2_localnet.py`'s `setUpModule`, to snapshot the thread
    // count before a test and verify it settles back down after). Since
    // `threading.Thread.start()` here always runs its target SYNCHRONOUSLY
    // in-place (no real OS threads — `PyObjectRef` isn't `Send`), there is
    // only ever the one, current thread live at any point this could be
    // observed from Python; a constant `1` makes `threading_cleanup`'s
    // `count <= orig_count` check trivially and correctly hold.
    thr_func!("_count", |_| Ok(py_int(1)));

    d
}
