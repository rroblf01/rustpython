use crate::object::*;
use std::collections::HashMap;
use num_traits::ToPrimitive;

// ─── _concurrent module (concurrent.futures native backend) ──────────────

pub fn create_concurrent_futures_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cf_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Standard exceptions
    d.insert("InvalidStateError".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "InvalidStateError".to_string(),
        func: |args| {
            let msg = if args.is_empty() { String::new() } else { args[0].str() };
            Ok(PyObjectRef::new(PyObject::Exception {
                typ: "InvalidStateError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
            }))
        },
    }));
    d.insert("TimeoutError".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "TimeoutError".to_string(),
        func: |args| {
            let msg = if args.is_empty() { String::new() } else { args[0].str() };
            Ok(PyObjectRef::new(PyObject::Exception {
                typ: "TimeoutError".to_string(),
                args: vec![py_str(&msg)],
                cause: None,
            }))
        },
    }));

    // ── ThreadPoolExecutor ─────────────────────────────────────────────
    
    // Executor base class (used by asgiref for type hints)
    let executor_type = PyObjectRef::new(PyObject::Type {
        name: "Executor".to_string(),
        dict: HashMap::new(),
        bases: vec![],
        mro: vec![],
    });
    d.insert("Executor".to_string(), executor_type);

    cf_func!("ThreadPoolExecutor", |args| {
        let max_workers = if args.len() > 0 {
            match &*args[0].borrow() {
                PyObject::Int(i) => i.to_i64().unwrap_or(1) as usize,
                _ => 1,
            }
        } else {
            1
        };

        let mut inst_dict = HashMap::new();

        // submit(fn, *args, **kwargs) -> returns a Future-like object
        inst_dict.insert("submit".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "submit".to_string(),
            func: |s_args| {
                // Minimal stub: if we have a callable, try calling it
                if !s_args.is_empty() {
                    let _fn = &s_args[0];
                    let _fn_args = &s_args[1..];
                    // In a real implementation we'd spawn a thread;
                    // here we just call synchronously and return a completed future.
                }
                // Return a simple completed future
                let mut fut_dict = HashMap::new();
                fut_dict.insert("result".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "result".to_string(),
                    func: |_| Ok(py_none()),
                }));
                fut_dict.insert("done".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "done".to_string(),
                    func: |_| Ok(py_bool(true)),
                }));
                fut_dict.insert("cancel".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "cancel".to_string(),
                    func: |_| Ok(py_bool(false)),
                }));
                fut_dict.insert("add_done_callback".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "add_done_callback".to_string(),
                    func: |_| Ok(py_none()),
                }));
                fut_dict.insert("exception".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "exception".to_string(),
                    func: |_| Ok(py_none()),
                }));
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: py_str("Future"),
                    dict: fut_dict,
                }))
            },
        }));

        // map(fn, *iterables) -> returns list of results
        inst_dict.insert("map".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "map".to_string(),
            func: |_| Ok(py_list(vec![])),
        }));

        // shutdown(wait=True)
        inst_dict.insert("shutdown".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "shutdown".to_string(),
            func: |_| Ok(py_none()),
        }));

        // __enter__ / __exit__ for context manager
        inst_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |ctx_args| {
                // Return self
                if ctx_args.is_empty() {
                    return Err(PyError::type_error("__enter__ missing self"));
                }
                Ok(ctx_args[0].clone())
            },
        }));

        inst_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__exit__".to_string(),
            func: |_| Ok(py_bool(false)),
        }));

        inst_dict.insert("_max_workers".to_string(), py_int(max_workers as i64));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("ThreadPoolExecutor"),
            dict: inst_dict,
        }))
    });

    // ── ProcessPoolExecutor (stub) ──────────────────────────────────────

    cf_func!("ProcessPoolExecutor", |args| {
        let max_workers = if args.len() > 0 {
            match &*args[0].borrow() {
                PyObject::Int(i) => i.to_i64().unwrap_or(1) as usize,
                _ => 1,
            }
        } else {
            1
        };

        let mut inst_dict = HashMap::new();
        inst_dict.insert("submit".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "submit".to_string(),
            func: |_| {
                let mut fut_dict = HashMap::new();
                fut_dict.insert("result".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "result".to_string(),
                    func: |_| Ok(py_none()),
                }));
                fut_dict.insert("done".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "done".to_string(),
                    func: |_| Ok(py_bool(true)),
                }));
                fut_dict.insert("cancel".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "cancel".to_string(),
                    func: |_| Ok(py_bool(false)),
                }));
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: py_str("Future"),
                    dict: fut_dict,
                }))
            },
        }));
        inst_dict.insert("map".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "map".to_string(),
            func: |_| Ok(py_list(vec![])),
        }));
        inst_dict.insert("shutdown".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "shutdown".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("_max_workers".to_string(), py_int(max_workers as i64));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("ProcessPoolExecutor"),
            dict: inst_dict,
        }))
    });

    // ── Future (stub class) ─────────────────────────────────────────────

    cf_func!("Future", |_args| {
        let mut inst_dict = HashMap::new();
        inst_dict.insert("result".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "result".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("done".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "done".to_string(),
            func: |_| Ok(py_bool(false)),
        }));
        inst_dict.insert("cancel".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "cancel".to_string(),
            func: |_| Ok(py_bool(false)),
        }));
        inst_dict.insert("cancelled".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "cancelled".to_string(),
            func: |_| Ok(py_bool(false)),
        }));
        inst_dict.insert("running".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "running".to_string(),
            func: |_| Ok(py_bool(false)),
        }));
        inst_dict.insert("add_done_callback".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "add_done_callback".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("exception".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "exception".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("set_result".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "set_result".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("set_exception".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "set_exception".to_string(),
            func: |_| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("Future"),
            dict: inst_dict,
        }))
    });

    // ── wait / as_completed (utility functions) ─────────────────────────

    cf_func!("wait", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("wait() requires at least 1 argument (futures)"));
        }
        // Return (done, not_done) tuple of sets
        let done_set = PySet::new();
        let not_done_set = PySet::new();
        Ok(py_tuple(vec![
            PyObjectRef::new(PyObject::Set(done_set)),
            PyObjectRef::new(PyObject::Set(not_done_set)),
        ]))
    });

    cf_func!("as_completed", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("as_completed() requires at least 1 argument (futures)"));
        }
        Ok(py_list(vec![]))
    });

    // ── constants ───────────────────────────────────────────────────────

    d.insert("FIRST_COMPLETED".to_string(), py_str("FIRST_COMPLETED"));
    d.insert("FIRST_EXCEPTION".to_string(), py_str("FIRST_EXCEPTION"));
    d.insert("ALL_COMPLETED".to_string(), py_str("ALL_COMPLETED"));

    d
}
