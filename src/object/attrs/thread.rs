// Auto-extracted from src/object/attrs/mod.rs lines 5419-5525
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Thread(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "start" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "start".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Thread(inner_arc) = &*obj {
                                let mut locked = inner_arc.lock().unwrap();
                                if locked.started {
                                    return Err(PyError::runtime_error(
                                        "threads can only be started once",
                                    ));
                                }
                                locked.started = true;
                                let target = locked.target.clone();
                                let thread_args = locked.args.clone();
                                // Cooperative scheduling: DEFER the target into the
                                // global pending-queue. It runs when someone joins
                                // this thread or when a potentially-blocking op
                                // (Queue.get on empty, Lock.acquire contention,
                                // Event.wait) drains the queue — giving deferred
                                // bodies their happens-before with the main flow.
                                let result = locked.result.clone();
                                drop(locked);
                                crate::modules::coop_threads_enqueue(Box::new(move || {
                                    let call_result =
                                        crate::object::builtin_call(&target, &thread_args);
                                    match call_result {
                                        Ok(val) => {
                                            *result.lock().unwrap() = Some(val);
                                        }
                                        Err(e) => {
                                            // Cooperative-scheduler unwind
                                            // (blocked-forever) is internal.
                                            if !crate::object::is_stop_iteration_error(&e) {
                                                eprintln!("Thread raised: {}", e);
                                            }
                                        }
                                    }
                                }));
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Thread(inner_arc) = &*obj {
                                // Drain cooperative queue so this join's target
                                // (and everything queued before it) runs now.
                                crate::modules::coop_threads_drain();
                                let mut locked = inner_arc.lock().unwrap();
                                if let Some(handle) = locked.handle.take() {
                                    handle
                                        .join()
                                        .map_err(|_| PyError::runtime_error("thread panicked"))?;
                                    return Ok(locked
                                        .result
                                        .lock()
                                        .unwrap()
                                        .clone()
                                        .unwrap_or_else(|| py_none()));
                                }
                                // No real `handle` (the common case — see
                                // `ThreadInner::started`'s own doc comment):
                                // `start()` already ran the target
                                // synchronously to completion by the time it
                                // returned, so `join()` on a `started`
                                // thread just returns its already-available
                                // result immediately instead of incorrectly
                                // erroring.
                                if locked.started {
                                    return Ok(locked
                                        .result
                                        .lock()
                                        .unwrap()
                                        .clone()
                                        .unwrap_or_else(|| py_none()));
                                }
                            }
                            Err(PyError::runtime_error(
                                "cannot join thread before it is started",
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "is_alive" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "is_alive".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Thread(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                return Ok(py_bool(locked.handle.is_some()));
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'Thread' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
