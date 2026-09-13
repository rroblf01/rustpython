use crate::object::*;
use std::collections::HashMap;

pub fn create_queue_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! q_func {
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

    q_func!("Queue", |_args| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(QueueInner {
            queue: std::collections::VecDeque::new(),
        }));
        Ok(PyObjectRef::new(PyObject::Queue(inner)))
    });

    // `queue.SimpleQueue` — a plain unbounded FIFO with no task-tracking
    // (no `task_done`/`join`) and no `maxsize`. Backed by the same
    // `PyObject::Queue` the regular `Queue` uses (its `get`/`put`/`qsize`
    // methods already match SimpleQueue's own semantics closely enough;
    // real CPython implements SimpleQueue as a distinct C type mainly for
    // reentrancy safety around signal handlers, not for a different
    // Python-visible API surface).
    q_func!("SimpleQueue", |_args| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(QueueInner {
            queue: std::collections::VecDeque::new(),
        }));
        Ok(PyObjectRef::new(PyObject::Queue(inner)))
    });

    // `queue.SimpleQueue` was entirely missing — a real, general gap (any
    // code doing `from queue import SimpleQueue`, not just this project's
    // own test suite, failed outright with an ImportError). Real CPython's
    // `SimpleQueue` is a simpler, unbounded, reentrancy-safe queue with no
    // `task_done()`/`join()` bookkeeping; this project's existing `Queue`
    // is already backed by an unbounded `Arc<Mutex<VecDeque>>` with no
    // maxsize enforcement, so it's a faithful-enough implementation to
    // reuse directly rather than adding a second, near-identical native
    // queue type.
    q_func!("SimpleQueue", |_args| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(QueueInner {
            queue: std::collections::VecDeque::new(),
        }));
        Ok(PyObjectRef::new(PyObject::Queue(inner)))
    });

    d
}
