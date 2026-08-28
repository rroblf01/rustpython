use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;
use num_bigint::BigInt;

pub(crate) fn asyncio_run_impl(
    vm: &mut crate::vm::VirtualMachine,
    coro: PyObjectRef,
) -> PyResult<PyObjectRef> {
    let coro_borrowed = coro.borrow();
    if let PyObject::Coroutine { ref frame } = &*coro_borrowed {
        let frame_borrowed = frame.borrow();
        if let Some(ref coro_frame) = *frame_borrowed {
            let mut coro_frame_clone = (**coro_frame).clone();
            coro_frame_clone.module_globals = None;
            drop(frame_borrowed);
            drop(coro_borrowed);
            vm.push_frame(coro_frame_clone);
            let result = vm.execute();
            vm.frames.pop();
            return result;
        }
    }
    drop(coro_borrowed);
    // If not a coroutine, try calling it directly
    let coro_clone = coro.clone();
    let send_attr = coro_clone.borrow().get_attribute("send").ok();
    if let Some(send_method) = send_attr {
        let result = crate::object::call_bound_method(
            send_method,
            coro.clone(),
            vec![crate::object::py_none()],
        );
        match result {
            Ok(val) => Ok(val),
            Err(crate::object::PyError::StopIteration) => Ok(crate::object::py_none()),
            Err(e) => Err(e),
        }
    } else {
        crate::object::call_bound_method(coro.clone(), coro.clone(), vec![])
    }
}

pub fn asyncio_run_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "run() missing required argument (coro)",
        ));
    }
    let coro = args[0].clone();
    crate::object::with_vm_mut(|vm| asyncio_run_impl(vm, coro))?
}

pub fn create_asyncio_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! asyncio_func {
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

    // Future class
    let mut future_type_dict = HashMap::new();
    macro_rules! future_method {
        ($name:expr, $func:expr) => {
            future_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    future_method!("__init__", |args| {
        let self_obj = args[0].clone();
        let _obj = self_obj.borrow_mut();
        // Future state stored in __dict__
        Ok(crate::object::py_none())
    });
    future_method!("__await__", |args| {
        // Returns a generator that yields self once then returns result
        let self_obj = args[0].clone();
        Ok(self_obj)
    });
    future_method!("set_result", |args| {
        let self_obj = args[0].clone();
        let result = args[1].clone();
        self_obj.borrow_mut().set_attribute("_result", result).ok();
        self_obj
            .borrow_mut()
            .set_attribute("_done", crate::object::py_bool(true))
            .ok();
        Ok(crate::object::py_none())
    });
    future_method!("done", |args| {
        let self_obj = args[0].clone();
        if let Ok(val) = self_obj.borrow().get_attribute("_done") {
            return Ok(val);
        }
        Ok(crate::object::py_bool(false))
    });
    future_method!("result", |args| {
        let self_obj = args[0].clone();
        if let Ok(val) = self_obj.borrow().get_attribute("_result") {
            return Ok(val);
        }
        Err(crate::object::PyError::runtime_error(
            "Future has no result",
        ))
    });

    let future_type = PyObjectRef::new(PyObject::Type {
        name: "Future".to_string(),
        dict: Box::new(str_map_to_typedict(future_type_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("Future", future_type);

    // Task class
    let mut task_type_dict = HashMap::new();
    macro_rules! task_method {
        ($name:expr, $func:expr) => {
            task_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    task_method!("__init__", |args| {
        let self_obj = args[0].clone();
        let coro = args[1].clone();
        self_obj.borrow_mut().set_attribute("_coro", coro).ok();
        self_obj
            .borrow_mut()
            .set_attribute("_done", crate::object::py_bool(false))
            .ok();
        Ok(crate::object::py_none())
    });
    task_method!("step", |args| {
        let self_obj = args[0].clone();
        let coro = self_obj.borrow().get_attribute("_coro")?;
        // Try to advance the coroutine via __next__ or send
        let next_func = coro.borrow().get_attribute("__next__")?;
        match crate::object::call_bound_method(next_func, coro.clone(), vec![]) {
            Ok(val) => {
                // If the coroutine yielded a Future, set up wakeup
                let type_name = val.borrow().type_name();
                if type_name == "Future" {
                    // Register a callback to resume this task
                    let self_clone = self_obj.clone();
                    let callback = PyObjectRef::new(PyObject::Closure(Rc::new(move |_args| {
                        // Step the task again
                        let _next_func2 = self_clone
                            .borrow()
                            .get_attribute("_coro")
                            .ok()
                            .and_then(|c| c.borrow().get_attribute("send").ok());
                        Ok(crate::object::py_none())
                    })));
                    val.borrow_mut()
                        .set_attribute("_callbacks", crate::object::py_list(vec![callback]))
                        .ok();
                }
                Ok(val)
            }
            Err(crate::object::PyError::StopIteration) => {
                self_obj
                    .borrow_mut()
                    .set_attribute("_done", crate::object::py_bool(true))
                    .ok();
                Ok(crate::object::py_none())
            }
            Err(e) => Err(e),
        }
    });

    let task_type = PyObjectRef::new(PyObject::Type {
        name: "Task".to_string(),
        dict: Box::new(str_map_to_typedict(task_type_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("Task", task_type);

    // asyncio.run(coro): Minimal event loop
    // get_running_loop()/get_event_loop() — this native asyncio module has
    // no real running-loop/scheduler state to consult (no coroutine
    // scheduler here at all — `run` above just directly executes the
    // coroutine's frame synchronously), so the only correct answer for
    // `get_running_loop()` in EVERY case this module can actually represent
    // is "no loop is running". Missing this entirely (get_running_loop
    // didn't exist under this name at all) broke the extremely common
    // defensive idiom `try: asyncio.get_running_loop() except
    // RuntimeError: ...` — those callers catch RuntimeError specifically,
    // not AttributeError, so real code (e.g. Django's own internals) that
    // uses this idiom crashed instead of falling through cleanly.
    asyncio_func!("get_running_loop", |_args| {
        Err(crate::object::PyError::runtime_error(
            "no running event loop",
        ))
    });

    asyncio_func!("run", asyncio_run_builtin);

    // asyncio.sleep(delay) -> Future
    // Returns a Future that resolves after the delay
    asyncio_func!("sleep", |args| {
        let delay = args[0].clone();
        // Create a Future by calling builtins.dict or using construct
        let future = crate::object::PyObjectRef::new(crate::object::PyObject::Instance {
            typ: crate::object::py_none(), // placeholder
            dict: AttrMap::new(),
        });
        // Set Future-specific attributes
        future
            .borrow_mut()
            .set_attribute("_done", crate::object::py_bool(false))
            .ok();
        future
            .borrow_mut()
            .set_attribute("_result", crate::object::py_none())
            .ok();
        // For now, immediately resolve sleep(0) and create pending for others
        if let crate::object::PyObject::Int(n) = &*delay.borrow() {
            if n == &num_bigint::BigInt::from(0) {
                future
                    .borrow_mut()
                    .set_attribute("_done", crate::object::py_bool(true))
                    .ok();
                future
                    .borrow_mut()
                    .set_attribute("_result", crate::object::py_none())
                    .ok();
            }
        }
        Ok(future)
    });

    // asyncio.gather(*coros, return_exceptions=False)
    asyncio_func!("gather", |args| {
        let futures: Vec<PyObjectRef> = args.to_vec();
        // For now, return a simple list of results (blocking gather)
        let mut results = Vec::new();
        for f in &futures {
            // Try to run directly if it's a coroutine
            let f_type = f.borrow().type_name();
            if f_type == "coroutine" || f_type == "generator" {
                if let Ok(send) = f.borrow().get_attribute("send") {
                    match crate::object::call_bound_method(
                        send,
                        f.clone(),
                        vec![crate::object::py_none()],
                    ) {
                        Ok(val) => results.push(val),
                        Err(crate::object::PyError::StopIteration) => {
                            results.push(crate::object::py_none())
                        }
                        Err(e) => return Err(e),
                    }
                }
            } else {
                results.push(f.clone());
            }
        }
        Ok(crate::object::py_list(results))
    });

    // asyncio.iscoroutinefunction(func): Check if func is a coroutine function
    asyncio_func!("iscoroutinefunction", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "iscoroutinefunction() missing required argument",
            ));
        }
        let func = &args[0];
        let borrowed = func.borrow();
        // Check for __code__ with CO_COROUTINE flag (0x80)
        if let Ok(code) = borrowed.get_attribute("__code__") {
            if let Ok(flags) = code.borrow().get_attribute("co_flags") {
                if let PyObject::Int(n) = &*flags.borrow() {
                    if n & BigInt::from(0x80) != BigInt::from(0) {
                        return Ok(py_bool(true));
                    }
                }
            }
        }
        // Check if it's a coroutine type
        let type_name = borrowed.type_name();
        if type_name == "coroutine" || type_name == "coroutine_function" {
            return Ok(py_bool(true));
        }
        Ok(py_bool(false))
    });

    d
}
