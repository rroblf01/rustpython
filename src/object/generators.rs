// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds generator/
// coroutine driving: __next__/send/throw for both.
use super::*;

// ---- generator.__next__() / generator.send() ----

/// Body of `generator.__next__()`/`generator.send(value)`.
///
/// DELIBERATELY uses its own disposable `VirtualMachine::new()` rather than
/// real `&mut VirtualMachine` access, even though that costs rebuilding the
/// entire native module registry on every single resume (measured: 1,441
/// disposable-VM constructions for one real CPython test file,
/// `test_coroutines.py`, contributing a real ~4.5x slowdown when the
/// cycle-GC is enabled vs disabled — not a GC bug, just GC triggered far
/// more often by this avoidable allocation flood). A real-`&mut self`
/// version WAS tried and reverted after it segfaulted: `builtin_next`'s own
/// `PyObject::Generator` arm calls this function's `BuiltinMethod.func`
/// directly (`f(&[args[0].clone()])`), bypassing `vm.rs`'s `call_function`
/// dispatch entirely — so the `fn_addr_eq`-special-case trick used for
/// `generator.throw()` (which IS only ever reached through `call_function`)
/// doesn't cover this path, and it fell through to `with_vm_mut`'s aliased
/// raw-pointer access instead — fatal here specifically because this
/// function, unlike `throw()`'s disposable-VM predecessor, would have
/// pushed/popped `vm.frames` for real, corrupting the REAL, already-
/// executing VM's own frame stack (confirmed via a native SIGSEGV inside
/// `Vec::push`'s reallocation, gdb backtrace showed the alias). Revisit only
/// after auditing (and fixing) EVERY direct, non-`call_function` call site
/// that can reach a generator's `__next__`/`send` (`builtin_next` is the
/// known one; there may be others in itertools-style eager-materialization
/// helpers) to route through real `&mut VirtualMachine` access instead.
pub(crate) fn generator_next_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let frame_ptr: *const std::cell::RefCell<Option<Box<crate::vm::Frame>>> = {
        let gen = args[0].borrow();
        match &*gen {
            PyObject::Generator { frame } => frame as *const _,
            _ => std::ptr::null(),
        }
    };
    if frame_ptr.is_null() {
        return Err(PyError::runtime_error("__next__ on non-generator"));
    }
    let frame_rc = unsafe { &*frame_ptr };
    // A generator that (directly or via some nested callback) tries to
    // resume ITSELF while already mid-execution previously panicked with a
    // raw "RefCell already borrowed" instead of real CPython's own
    // `ValueError: generator already executing` (real trigger: CPython's
    // own `test_coroutines.py`/`test_yield_from.py`). `try_borrow_mut`
    // turns that exact conflict into the correct, catchable exception.
    let mut frame_opt = match frame_rc.try_borrow_mut() {
        Ok(guard) => guard,
        Err(_) => return Err(PyError::value_error("generator already executing")),
    };
    if let Some(f) = frame_opt.as_mut() {
        if args.len() > 1 {
            f.stack.push(args[1].clone());
        } else {
            f.stack.push(crate::object::py_none());
        }
        let mut vm = crate::vm::VirtualMachine::new();
        vm.push_frame((**f).clone());
        match vm.execute() {
            Ok(val) => {
                let modified = vm.frames.pop().unwrap();
                if modified.ip > 0
                    && matches!(
                        &modified.code.instructions[modified.ip - 1].op,
                        crate::bytecode::Opcode::YIELD_VALUE
                    )
                {
                    *f = Box::new(modified);
                    Ok(val)
                } else {
                    *frame_opt = None;
                    Err(crate::object::PyError::Exception(
                        "StopIteration".to_string(),
                        val,
                    ))
                }
            }
            Err(e) => {
                *frame_opt = None;
                Err(wrap_stopiteration_pep479(e))
            }
        }
    } else {
        Err(PyError::StopIteration)
    }
}

/// PEP 479: a `StopIteration` that escapes from a generator's OWN body code
/// (as opposed to the generator's normal exhaustion, which this interpreter
/// signals via a DIFFERENT, internal `PyError::StopIteration`/`Exception
/// ("StopIteration", ...)` produced right here in `generator_next_fallback`
/// once its frame naturally finishes) must be converted into a `RuntimeError`
/// instead of propagating as-is — otherwise a `StopIteration` accidentally
/// raised deep inside a generator (e.g. `yield f()` where `f()` itself raises
/// `StopIteration`) is silently indistinguishable from the generator simply
/// being done, which is exactly the surprising-early-termination bug PEP 479
/// exists to prevent. Confirmed missing via `test_generator_stop.py`'s own
/// `test_stopiteration_wrapping`/`test_stopiteration_wrapping_context`.
fn wrap_stopiteration_pep479(e: PyError) -> PyError {
    let stop_instance = match &e {
        PyError::StopIteration => Some(PyObjectRef::new(PyObject::Exception {
            typ: "StopIteration".to_string(),
            args: vec![],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        })),
        PyError::Exception(name, obj) if name == "StopIteration" => Some(obj.clone()),
        _ => None,
    };
    match stop_instance {
        Some(inst) => {
            // __context__ mirrors __cause__ (both the StopIteration) and
            // __suppress_context__ is True — test_generator_stop's
            // test_stopiteration_wrapping_context asserts all three.
            let ctx = inst.clone();
            PyError::Exception(
                "RuntimeError".to_string(),
                PyObjectRef::new(PyObject::Exception {
                    typ: "RuntimeError".to_string(),
                    args: vec![py_str("generator raised StopIteration")],
                    cause: Some(inst),
                    suppress_context: true,
                    context: Some(ctx),
                    traceback: None,
                    extra: None,
                }),
            )
        }
        None => e,
    }
}

// ---- coroutine.send() / coroutine.throw() ----

/// Body of `coroutine.send(value)` — deliberately uses a disposable VM, same
/// rationale as `generator_next_fallback` above (kept this way for
/// consistency/safety even though no direct, non-`call_function` bypass is
/// currently known for coroutines specifically — `SEND`/`GET_AWAITABLE`
/// opcodes and explicit `.send()` calls all route through `call_function`).
pub(crate) fn coroutine_send_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let gen = args[0].borrow();
    if let PyObject::Coroutine { frame } = &*gen {
        // A coroutine resuming itself re-entrantly must raise real
        // CPython's `ValueError: coroutine already executing`, not panic
        // on a raw RefCell conflict.
        let mut frame_opt = match frame.try_borrow_mut() {
            Ok(guard) => guard,
            Err(_) => return Err(PyError::value_error("coroutine already executing")),
        };
        if let Some(f) = frame_opt.as_mut() {
            if args.len() > 1 {
                f.stack.push(args[1].clone());
            } else {
                f.stack.push(crate::object::py_none());
            }
            let mut vm = crate::vm::VirtualMachine::new();
            vm.push_frame((**f).clone());
            match vm.execute() {
                Ok(val) => {
                    let modified = vm.frames.pop().unwrap();
                    if modified.ip > 0
                        && matches!(
                            &modified.code.instructions[modified.ip - 1].op,
                            crate::bytecode::Opcode::YIELD_VALUE
                        )
                    {
                        *f = Box::new(modified);
                        Ok(val)
                    } else {
                        *frame_opt = None;
                        // Propagate the return value via StopIteration for SEND
                        Err(crate::object::PyError::Exception(
                            "StopIteration".to_string(),
                            val,
                        ))
                    }
                }
                Err(e) => {
                    *frame_opt = None;
                    // Propagate the coroutine's own unhandled
                    // exception as-is; only genuine exhaustion
                    // is signaled as StopIteration.
                    Err(e)
                }
            }
        } else {
            Err(PyError::StopIteration)
        }
    } else {
        Err(PyError::runtime_error("send on non-coroutine"))
    }
}

/// Body of `coroutine.throw(value)` — see `coroutine_send_fallback` above.
pub(crate) fn coroutine_throw_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "throw() missing required argument 'value'",
        ));
    }
    let gen = args[0].borrow();
    if let PyObject::Coroutine { frame } = &*gen {
        let mut frame_opt = match frame.try_borrow_mut() {
            Ok(guard) => guard,
            Err(_) => return Err(PyError::value_error("coroutine already executing")),
        };
        if let Some(f) = frame_opt.as_mut() {
            let mut vm = crate::vm::VirtualMachine::new();
            let raw = args[1].clone();
            let is_callable = !matches!(
                &*raw.borrow(),
                PyObject::Exception { .. }
                    | PyObject::ExceptionGroup { .. }
                    | PyObject::Instance { .. }
            );
            let exc_obj = if is_callable {
                vm.call_function(raw.clone(), vec![], vec![]).map_err(|_| {
                    PyError::type_error(
                        "exceptions must be classes or instances deriving from BaseException",
                    )
                })?
            } else {
                raw
            };
            let typ = match &*exc_obj.borrow() {
                PyObject::Exception { typ, .. } => typ.clone(),
                _ => "Exception".to_string(),
            };
            let err = PyError::Exception(typ, exc_obj);
            vm.push_frame((**f).clone());
            match vm.throw_into_frame(err) {
                Ok(val) => {
                    let modified = vm.frames.pop().unwrap();
                    if modified.ip > 0
                        && matches!(
                            &modified.code.instructions[modified.ip - 1].op,
                            crate::bytecode::Opcode::YIELD_VALUE
                        )
                    {
                        *f = Box::new(modified);
                        Ok(val)
                    } else {
                        *frame_opt = None;
                        Err(crate::object::PyError::Exception(
                            "StopIteration".to_string(),
                            val,
                        ))
                    }
                }
                Err(e) => {
                    *frame_opt = None;
                    Err(e)
                }
            }
        } else {
            Err(PyError::StopIteration)
        }
    } else {
        Err(PyError::runtime_error("throw() on non-coroutine"))
    }
}

// ---- generator.throw() ----

/// Real body of `generator.throw(value)`, given genuine `&mut
/// VirtualMachine` access — called directly from `vm.rs`'s `call_function`
/// special-case (matching `func` by `fn_addr_eq` against
/// `generator_throw_fallback` below) instead of that fallback's own
/// disposable `VirtualMachine::new()`. That disposable VM was memory-safe
/// (unlike the `with_vm_mut` bugs elsewhere this session) but still
/// SEMANTICALLY wrong: `exc_type`/`exc_value` (what `sys.exc_info()` reads)
/// live on the `VirtualMachine` instance itself, so resuming the generator
/// on a fresh, blank VM instance meant any `sys.exc_info()` call inside the
/// generator's own `except:` block (real trigger: `contextlib.
/// contextmanager`'s `__exit__` calling `gen.throw(...)`, exercised by
/// `unittest`'s own `_Outcome.testPartExecutor`) saw `(None, None, None)`
/// instead of the exception actually being thrown in — `unittest.main()`
/// then crashed on `issubclass(None, ...)` the moment any real test
/// failure/error needed to be classified and reported.
pub(crate) fn generator_throw_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "throw() missing required argument 'value'",
        ));
    }
    let gen = args[0].borrow();
    if let PyObject::Generator { frame } = &*gen {
        let mut frame_opt = match frame.try_borrow_mut() {
            Ok(guard) => guard,
            Err(_) => return Err(PyError::value_error("generator already executing")),
        };
        if let Some(f) = frame_opt.as_mut() {
            let raw = args[1].clone();
            let is_callable = !matches!(
                &*raw.borrow(),
                PyObject::Exception { .. }
                    | PyObject::ExceptionGroup { .. }
                    | PyObject::Instance { .. }
            );
            let exc_obj = if is_callable {
                vm.call_function(raw.clone(), vec![], vec![]).map_err(|_| {
                    PyError::type_error(
                        "exceptions must be classes or instances deriving from BaseException",
                    )
                })?
            } else {
                raw
            };
            let typ = match &*exc_obj.borrow() {
                PyObject::Exception { typ, .. } => typ.clone(),
                _ => "Exception".to_string(),
            };
            let err = PyError::Exception(typ, exc_obj);
            vm.push_frame((**f).clone());
            match vm.throw_into_frame(err) {
                Ok(val) => {
                    let modified = vm.frames.pop().unwrap();
                    if modified.ip > 0
                        && matches!(
                            &modified.code.instructions[modified.ip - 1].op,
                            crate::bytecode::Opcode::YIELD_VALUE
                        )
                    {
                        *f = Box::new(modified);
                        Ok(val)
                    } else {
                        *frame_opt = None;
                        Err(crate::object::PyError::Exception(
                            "StopIteration".to_string(),
                            val,
                        ))
                    }
                }
                Err(e) => {
                    vm.frames.pop();
                    *frame_opt = None;
                    Err(wrap_stopiteration_pep479(e))
                }
            }
        } else {
            Err(PyError::StopIteration)
        }
    } else {
        Err(PyError::runtime_error("throw() on non-generator"))
    }
}

/// Fallback for `generator.throw()` when reached some other way than a live
/// bytecode `CALL` (believed unreachable in practice — every real call goes
/// through `vm.rs`'s `call_function` special-case above). Kept only because
/// `PyObject::BuiltinMethod.func` must be a plain `fn` pointer with no
/// captured VM access, matching the same fallback convention used
/// throughout this codebase (see `import_module_builtin`, `find_spec_builtin`).
pub(crate) fn generator_throw_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| generator_throw_with_vm(vm, args))?
}
