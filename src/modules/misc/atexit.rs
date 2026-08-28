use crate::object::*;
use std::collections::HashMap;

thread_local! {
    // Each callback stores the callable plus the extra positional args
    // (and a trailing keyword dict, if any) it was registered with — real
    // `atexit.register(func, *args, **kwargs)` passes those on invocation.
    static EXIT_CALLBACKS: std::cell::RefCell<Vec<(PyObjectRef, Vec<PyObjectRef>, Vec<(String, PyObjectRef)>)>> = std::cell::RefCell::new(Vec::new());
}

thread_local! {
    // The real `sys` module (registered once at VM init) — native code like
    // atexit's `_run_exitfuncs` reads the CURRENT `sys.unraisablehook` from
    // it to report raising callbacks. A disposable VM's own sys module would
    // hold the DEFAULT hook, losing any reassignment made by
    // `catch_unraisable_exception`-style contexts.
    static CURRENT_SYS_MODULE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

pub(crate) fn set_sys_module(mod_ref: Option<PyObjectRef>) {
    CURRENT_SYS_MODULE.with(|m| *m.borrow_mut() = mod_ref);
}

thread_local! {
    // The real builtins map (see `set_builtins_ref`) — lets native code
    // resolve a builtin exception CLASS object by name.
    static CURRENT_BUILTINS: std::cell::RefCell<Option<std::rc::Rc<std::collections::HashMap<crate::interner::StrId, PyObjectRef>>>> = std::cell::RefCell::new(None);
}

pub(crate) fn set_builtins_ref(
    builtins: std::rc::Rc<std::collections::HashMap<crate::interner::StrId, PyObjectRef>>,
) {
    CURRENT_BUILTINS.with(|b| *b.borrow_mut() = Some(builtins));
}

pub(crate) fn get_builtin_class(name: &str) -> Option<PyObjectRef> {
    CURRENT_BUILTINS.with(|b| {
        let map = b.borrow().clone()?;
        let id = crate::interner::intern(name);
        map.get(&id).cloned()
    })
}

/// Add `cls` to an ABC's `_abc_registry` (CPython's `ABC.register(cls)`).
fn abc_register_class(abc: &PyObjectRef, cls: &PyObjectRef) {
    if let PyObject::Type { dict, .. } = &mut *abc.borrow_mut() {
        let mut items = if let Some(r) = dict.get_str("_abc_registry") {
            if let PyObject::FrozenSet(s) = &*r.borrow() {
                s.to_vec()
            } else {
                vec![]
            }
        } else {
            vec![]
        };
        if !items.iter().any(|r| r.is(cls)) {
            items.push(cls.clone());
        }
        let mut set = PySet::new();
        for i in items {
            let _ = set.add(i);
        }
        dict.insert_str("_abc_registry", PyObjectRef::imm(PyObject::FrozenSet(set)));
    }
}

/// Register the builtin container types as virtual subclasses of their
/// `collections.abc` ABCs (CPython's `_collections_abc` module does this at
/// startup) — so `issubclass(dict, Mapping)`, `issubclass(list, Sequence)`
/// etc. hold. Must run AFTER the builtins map is available.
pub(crate) fn register_collections_abc_builtins() {
    let abc = get_module("collections.abc");
    let Some(abc) = abc else { return };
    let get_name = |d: &HashMap<String, PyObjectRef>, n: &str| d.get(n).cloned();
    let abc_entries: HashMap<String, PyObjectRef> = {
        let b = abc.borrow();
        if let PyObject::Module { dict, .. } = &*b {
            dict.iter()
                .map(|(k, v)| (crate::interner::lookup_str(*k).to_string(), v.clone()))
                .collect()
        } else {
            return;
        }
    };
    let builtin = |n: &str| get_builtin_class(n);
    let reg = |abc_name: &str, builtin_name: &str| {
        if let (Some(abc), Some(b)) = (get_name(&abc_entries, abc_name), builtin(builtin_name)) {
            abc_register_class(&abc, &b);
        }
    };
    reg("Mapping", "dict");
    reg("MutableMapping", "dict");
    reg("Sequence", "list");
    reg("Sequence", "str");
    reg("Sequence", "tuple");
    reg("Sequence", "bytes");
    reg("Sequence", "bytearray");
    reg("Sequence", "range");
    reg("MutableSequence", "list");
    reg("MutableSequence", "bytearray");
    reg("Set", "set");
    reg("Set", "frozenset");
    reg("MutableSet", "set");
    reg("Iterable", "list");
    reg("Iterable", "tuple");
    reg("Iterable", "dict");
    reg("Iterable", "set");
    reg("Iterable", "frozenset");
    reg("Iterable", "str");
    reg("Iterable", "bytes");
    reg("Iterable", "bytearray");
    reg("Iterable", "range");
    reg("Collection", "list");
    reg("Collection", "tuple");
    reg("Collection", "dict");
    reg("Collection", "set");
    reg("Collection", "frozenset");
    reg("Collection", "str");
    reg("Collection", "bytes");
    reg("Collection", "bytearray");
    reg("Reversible", "list");
    reg("Reversible", "tuple");
    reg("Reversible", "str");
    reg("Reversible", "bytes");
    reg("Reversible", "bytearray");
    reg("Reversible", "range");
    reg("Sized", "list");
    reg("Sized", "tuple");
    reg("Sized", "dict");
    reg("Sized", "set");
    reg("Sized", "frozenset");
    reg("Sized", "str");
    reg("Sized", "bytes");
    reg("Sized", "bytearray");
    reg("Sized", "range");
    reg("Hashable", "str");
    reg("Hashable", "bytes");
    reg("Hashable", "tuple");
    reg("Hashable", "frozenset");
    reg("Iterator", "list_iterator");
}

/// Look up a module by name through the live `sys.modules` dict (no VM
/// needed — a plain dict read; safe from inside a native closure that is
/// itself running under the VM).
pub(crate) fn get_module(name: &str) -> Option<PyObjectRef> {
    CURRENT_SYS_MODULE.with(|m| {
        let sys_mod = m.borrow().clone()?;
        let modules = {
            let b = sys_mod.borrow();
            if let PyObject::Module { dict, .. } = &*b {
                dict.get_str("modules").cloned()
            } else {
                None
            }
        }?;
        let mb = modules.borrow();
        if let PyObject::Dict(d) = &*mb {
            d.get(&py_str(name)).ok().flatten()
        } else {
            None
        }
    })
}

fn get_current_unraisablehook() -> Option<PyObjectRef> {
    CURRENT_SYS_MODULE.with(|m| {
        let mod_ref = m.borrow().clone()?;
        let borrowed = mod_ref.borrow();
        if let PyObject::Module { dict, .. } = &*borrowed {
            dict.get_str("unraisablehook").cloned()
        } else {
            None
        }
    })
}

// `UnraisableHookArgs`-shaped object for a raising atexit callback (real
// CPython passes object=None for atexit callbacks, the func's repr in
// err_msg, and the exception's type/value). exc_type is the real builtin
// exception class (looked up through sys.modules['builtins'], so identity
// matches what Python code holds) and exc_value a real PyObject::Exception.
fn build_unraisable_args(func: &PyObjectRef, err: &PyError) -> PyObjectRef {
    let exc_name = py_error_type_name(err);
    if std::env::var("RPY_DEBUG_UNRAISABLE").is_ok() {
        eprintln!(
            "UNRAISABLE name={} err={:?} builtin={:?}",
            exc_name,
            err,
            get_builtin_class(&exc_name).map(|b| b.repr())
        );
    }
    let exc_value = PyObjectRef::new(PyObject::Exception {
        typ: exc_name.clone(),
        args: py_error_args(err),
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra: None,
    });
    let exc_type = CURRENT_SYS_MODULE.with(|m| {
        let mod_ref = m.borrow().clone()?;
        let borrowed = mod_ref.borrow();
        let modules = if let PyObject::Module { dict, .. } = &*borrowed {
            dict.get_str("modules").cloned()
        } else {
            None
        };
        let modules = modules?;
        let builtins_mod = {
            let mb = modules.borrow();
            if let PyObject::Dict(d) = &*mb {
                d.get(&py_str("builtins")).ok().flatten()
            } else {
                None
            }
        }?;
        let bb = builtins_mod.borrow();
        if let PyObject::Module { dict, .. } = &*bb {
            dict.get_str(&exc_name).cloned()
        } else {
            None
        }
    });
    let mut attrs = crate::object::AttrMap::new();
    attrs.insert_str("object", py_none());
    attrs.insert_str(
        "err_msg",
        py_str(&format!(
            "Exception ignored in atexit callback {}",
            func.repr()
        )),
    );
    attrs.insert_str("exc_type", exc_type.unwrap_or_else(|| py_none()));
    attrs.insert_str("exc_value", exc_value);
    attrs.insert_str("exc_traceback", py_none());
    let typ = PyObjectRef::new(PyObject::Type {
        name: "UnraisableHookArgs".to_string(),
        dict: Box::new(crate::object::str_map_to_typedict(
            std::collections::HashMap::new(),
        )),
        bases: vec![],
        mro: vec![],
    });
    PyObjectRef::new(PyObject::Instance { typ, dict: attrs })
}

fn py_error_type_name(err: &PyError) -> String {
    match err {
        PyError::TypeError(_) => "TypeError".to_string(),
        PyError::ValueError(_) => "ValueError".to_string(),
        PyError::NameError(_) => "NameError".to_string(),
        PyError::AttributeError(_) => "AttributeError".to_string(),
        PyError::IndexError(_) => "IndexError".to_string(),
        PyError::KeyError(_) => "KeyError".to_string(),
        PyError::ZeroDivisionError(_) => "ZeroDivisionError".to_string(),
        PyError::RuntimeError(_) => "RuntimeError".to_string(),
        PyError::SystemExit(_) => "SystemExit".to_string(),
        PyError::Exception(name, exc) => {
            // `raise SomeClass` (bare class, no message) comes through as
            // PyError::Exception("", exc) — the NAME field is empty, so
            // recover the exception type from the exc object itself.
            if name.is_empty() {
                match &*exc.borrow() {
                    PyObject::Exception { typ, .. } => typ.clone(),
                    PyObject::ExceptionGroup { typ, .. } => typ.clone(),
                    PyObject::Instance { typ, .. } => typ.borrow().type_name(),
                    _ => "Exception".to_string(),
                }
            } else {
                name.clone()
            }
        }
        PyError::OsError(_) => "OSError".to_string(),
        PyError::ImportError(_) => "ImportError".to_string(),
        PyError::RecursionError(_) => "RecursionError".to_string(),
        _ => "Exception".to_string(),
    }
}

fn py_error_args(err: &PyError) -> Vec<PyObjectRef> {
    match err {
        PyError::TypeError(m)
        | PyError::ValueError(m)
        | PyError::NameError(m)
        | PyError::AttributeError(m)
        | PyError::IndexError(m)
        | PyError::KeyError(m)
        | PyError::ZeroDivisionError(m)
        | PyError::RuntimeError(m)
        | PyError::ImportError(m)
        | PyError::RecursionError(m)
        | PyError::OsError(m) => vec![py_str(m)],
        PyError::Exception(_, exc) => {
            if let PyObject::Exception { args, .. } = &*exc.borrow() {
                args.clone()
            } else {
                vec![exc.clone()]
            }
        }
        _ => vec![],
    }
}

pub fn create_atexit_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "register",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "register".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "register() requires a callable argument",
                    ));
                }
                // Real `atexit.register(func, *args, **kwargs)` stores the extra
                // positional args (and, if present, a trailing keyword dict) and
                // passes them to `func` when it runs at shutdown — `test_atexit`
                // registers `print` with a message arg, and `test_shutdown`
                // asserts the printed output.
                let func = args[0].clone();
                let mut extra = args[1..].to_vec();
                let mut kwargs: Vec<(String, PyObjectRef)> = Vec::new();
                let trailing_is_dict = extra
                    .last()
                    .map(|l| matches!(&*l.borrow(), PyObject::Dict(_)))
                    .unwrap_or(false);
                if trailing_is_dict {
                    // Extract the trailing keyword-dict's items into `kwargs`
                    // (cloned so no borrow is held across `extra.pop()`).
                    let items: Vec<(String, PyObjectRef)> = {
                        let b = extra.last().unwrap().borrow();
                        if let PyObject::Dict(d) = &*b {
                            d.items().into_iter().map(|(k, v)| (k.str(), v)).collect()
                        } else {
                            Vec::new()
                        }
                    };
                    extra.pop();
                    kwargs = items;
                }
                EXIT_CALLBACKS.with(|cb| cb.borrow_mut().push((func, extra, kwargs)));
                Ok(py_none())
            },
        }),
    );
    d.insert_str(
        "unregister",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "unregister".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "unregister() requires a callable argument",
                    ));
                }
                let target = args[0].clone();
                // Real CPython compares callbacks with `==` (a callback's own
                // `__eq__` may even call unregister re-entrantly — see CPython
                // issue #112127 / _test_atexit's test_eq_unregister), NOT
                // identity. Evaluate equality WITHOUT holding the callbacks
                // borrow (re-entrant unregister needs borrow_mut), removing each
                // match from the live list as it is found.
                let funcs: Vec<PyObjectRef> = EXIT_CALLBACKS
                    .with(|cb| cb.borrow().iter().map(|(f, _, _)| f.clone()).collect());
                for f in &funcs {
                    let eq = crate::object::py_compare(f, &target, 2)
                        .map(|v| v.truthy())
                        .unwrap_or(false);
                    if eq {
                        EXIT_CALLBACKS.with(|cb| cb.borrow_mut().retain(|(g, _, _)| !g.is(f)));
                    }
                }
                Ok(py_none())
            },
        }),
    );
    d.insert_str("__name__", py_str("atexit"));
    d.insert_str(
        "_clear",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_clear".to_string(),
            func: |_| {
                EXIT_CALLBACKS.with(|cb| cb.borrow_mut().clear());
                Ok(py_none())
            },
        }),
    );
    // `atexit._ncallbacks` — real CPython's internal count of registered
    // callbacks, read directly by `test_atexit.py`'s `test_callbacks_leak`/
    // `test_callbacks_leak_refcycle` to detect leaked registrations. Backed
    // by the live `EXIT_CALLBACKS` list length so it stays in sync.
    d.insert_str(
        "_ncallbacks",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_ncallbacks".to_string(),
            func: |_| Ok(py_int(EXIT_CALLBACKS.with(|cb| cb.borrow().len() as i64))),
        }),
    );
    // `atexit.is_tracing()` — real CPython returns True iff a Python-level
    // trace function is currently set (`sys.gettrace() != None`). This
    // interpreter's `sys.settrace` is a no-op stub, so no tracing is ever
    // active; `test_atexit.py`'s leak tests call it during callback
    // iteration.
    d.insert_str(
        "is_tracing",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "is_tracing".to_string(),
            func: |_| Ok(py_bool(false)),
        }),
    );
    // `atexit._run_exitfuncs()` — runs all registered callbacks in LIFO
    // order and CLEARS them (real CPython's internal function, exercised
    // directly by the vendored `_test_atexit.py`, which runs it in-process
    // to verify ordering/arg-passing/unraisable handling without exiting).
    d.insert_str(
        "_run_exitfuncs",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_run_exitfuncs".to_string(),
            func: |_| {
                let callbacks: Vec<(PyObjectRef, Vec<PyObjectRef>, Vec<(String, PyObjectRef)>)> =
                    EXIT_CALLBACKS.with(|cb| cb.borrow().clone());
                for (func, extra, kwargs) in callbacks.iter().rev() {
                    // A raising callback is "unraisable" — real CPython reports
                    // it via sys.unraisablehook (the current hook, which
                    // catch_unraisable_exception-style contexts may have
                    // reassigned), then continues with the next callback.
                    let result = crate::object::call_function_disposable(
                        func,
                        extra.clone(),
                        kwargs.clone(),
                    );
                    if let Err(err) = result {
                        let unraisable = build_unraisable_args(func, &err);
                        if let Some(hook) = get_current_unraisablehook() {
                            let _ = crate::object::call_function_disposable(
                                &hook,
                                vec![unraisable],
                                vec![],
                            );
                        }
                    }
                }
                EXIT_CALLBACKS.with(|cb| cb.borrow_mut().clear());
                Ok(py_none())
            },
        }),
    );
    d
}

/// Run all registered atexit handlers, using the provided VM.
pub fn run_atexit_handlers(vm: &mut crate::vm::VirtualMachine) {
    // Opcode histogram dump (RPY_OPCODE_HIST=1) — every normal exit path
    // funnels through here, so this is the one reliable dump point.
    crate::vm::opcode_hist_dump();
    // Real CPython runs exit handlers in LIFO order (last registered runs
    // FIRST) — `test_shutdown`'s `atexit.register(print, "one"); atexit.
    // register(print, "two")` expects output `two` then `one`.
    let callbacks: Vec<(PyObjectRef, Vec<PyObjectRef>, Vec<(String, PyObjectRef)>)> =
        EXIT_CALLBACKS.with(|cb| cb.borrow().clone());
    for (func, extra, kwargs) in callbacks.iter().rev() {
        let mut call_args = extra.clone();
        if !kwargs.is_empty() {
            let mut kwd = PyDict::new();
            for (k, v) in kwargs {
                let _ = kwd.set(py_str(k), v.clone());
            }
            call_args.push(PyObjectRef::new(PyObject::Dict(Box::new(kwd))));
        }
        let _ = vm.call_function(func.clone(), call_args, vec![]);
    }
}
