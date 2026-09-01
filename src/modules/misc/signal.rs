use crate::object::*;
use std::collections::HashMap;

// Real, shared registered-signal-handler storage — `signal.signal()` writes
// here, `signal.getsignal()`/`raise_signal()`/`os.kill()` (killing our own
// pid, the only pid that means anything in this single-process interpreter)
// all read/invoke from the SAME map. A thread-local (not a global `static`)
// since every other piece of shared mutable module state in this codebase
// uses the same convention (see `WARN_FILTERS_LIST` above).
thread_local! {
    static SIGNAL_HANDLERS: std::cell::RefCell<std::collections::HashMap<i64, PyObjectRef>> = std::cell::RefCell::new(std::collections::HashMap::new());
}
fn signal_handlers(
) -> &'static std::thread::LocalKey<std::cell::RefCell<std::collections::HashMap<i64, PyObjectRef>>>
{
    &SIGNAL_HANDLERS
}

/// Actually invoke a registered `signal.signal(signum, handler)` callback,
/// matching real Python's `handler(signum, frame)` call shape (`frame` is
/// simply `None` here — this interpreter has no cross-call frame object to
/// hand back meaningfully at an arbitrary interrupt point). Silently does
/// nothing if no handler is registered, or if the stored value is one of
/// the `SIG_DFL`/`SIG_IGN` int sentinels rather than a real callable —
/// matches `signal.signal()`'s own default/ignore semantics.
pub(crate) fn invoke_signal_handler_impl(
    vm: &mut crate::vm::VirtualMachine,
    signum: i64,
) -> PyResult<PyObjectRef> {
    let handler = SIGNAL_HANDLERS.with(|h| h.borrow().get(&signum).cloned());
    match handler {
        Some(h) if !matches!(&*h.borrow(), PyObject::Int(_)) => {
            vm.call_function(h, vec![py_int(signum), py_none()], vec![])
        }
        _ => Ok(py_none()),
    }
}

pub(crate) fn signal_raise_signal_impl(
    vm: &mut crate::vm::VirtualMachine,
    signum: i64,
) -> PyResult<PyObjectRef> {
    invoke_signal_handler_impl(vm, signum)?;
    Ok(py_none())
}

pub fn signal_raise_signal_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "raise_signal() missing required argument (signalnum)",
        ));
    }
    let signum = args[0]
        .as_i64()
        .ok_or_else(|| PyError::type_error("signalnum must be an int"))?;
    crate::object::with_vm_mut(|vm| signal_raise_signal_impl(vm, signum))?
}

pub fn create_signal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Signal constants
    d.insert_str("SIGINT", py_int(2));
    d.insert_str("SIGTERM", py_int(15));
    d.insert_str("SIGHUP", py_int(1));
    d.insert_str("SIGILL", py_int(4));
    d.insert_str("SIGFPE", py_int(8));
    d.insert_str("SIGKILL", py_int(9));
    d.insert_str("SIGSEGV", py_int(11));
    d.insert_str("SIGPIPE", py_int(13));
    d.insert_str("SIGALRM", py_int(14));
    d.insert_str("SIGUSR1", py_int(10));
    d.insert_str("SIGUSR2", py_int(12));
    d.insert_str("SIG_DFL", py_int(0));
    d.insert_str("SIG_IGN", py_int(1));

    macro_rules! sig_func {
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

    // `signal.signal(signum, handler)` — was a total no-op (never stored
    // `handler` anywhere), so `raise_signal`/`os.kill(os.getpid(), sig)`
    // had no way to actually invoke a registered Python-level handler even
    // once real handler-invocation support was added below. Real handler
    // storage, shared across `signal`/`getsignal`/`raise_signal`/`os.kill`
    // (see `signal_handlers` and its own doc comment).
    sig_func!("signal", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "signal() requires 2 arguments (signalnum, handler)",
            ));
        }
        let signum = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("signalnum must be an int"))?;
        let old = signal_handlers()
            .with(|h| h.borrow().get(&signum).cloned())
            .unwrap_or_else(py_none);
        signal_handlers().with(|h| h.borrow_mut().insert(signum, args[1].clone()));
        Ok(old)
    });

    sig_func!("getsignal", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "getsignal() missing required argument (signalnum)",
            ));
        }
        let signum = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("signalnum must be an int"))?;
        Ok(signal_handlers()
            .with(|h| h.borrow().get(&signum).cloned())
            .unwrap_or_else(|| py_int(0)))
    });

    // `signal.alarm(sec)` — real deadline stored thread-locally; delivery
    // happens at cooperative checkpoints (selectors' select loop, time.sleep)
    // which invoke the registered SIGALRM handler via with_vm_mut.
    d.insert_str("alarm", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "alarm".into(),
        func: |args| {
            let sec = args.first().and_then(|a| a.as_f64()).unwrap_or(0.0);
            let prev = crate::modules::misc_alarm_set(sec);
            Ok(py_int(prev as i64))
        },
    }));
    // `signal.raise_signal(signum)` — was a no-op even after real handler
    // STORAGE was added just above, because actually CALLING a registered
    // Python-level handler needs a live `&mut VirtualMachine` (same
    // `with_vm_mut`-is-UB class of bug as `asyncio.run`/`exec` elsewhere in
    // this file) — real invocation happens via `vm.rs`'s own special case
    // for this exact function pointer (see `signal_raise_signal_impl`);
    // this is the `with_vm_mut`-based fallback for any path that reaches
    // it without going through that special case.
    sig_func!("raise_signal", signal_raise_signal_builtin);

    d
}
