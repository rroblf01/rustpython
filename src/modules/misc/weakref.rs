use crate::object::*;
use std::collections::HashMap;
// ---- weakref registry ----
use std::rc::Weak as RcWeak;
use std::cell::RefCell as StdRefCell;

thread_local! {
    static WEAKREF_REGISTRY: StdRefCell<HashMap<usize, Vec<WeakEntry>>> = StdRefCell::new(HashMap::new());
}
struct WeakEntry {
    weakref: RcWeak<StdRefCell<PyObject>>,
    callback: Option<PyObjectRef>,
}
fn is_weakrefable(obj: &PyObjectRef) -> bool {
    !matches!(
        &*obj.borrow(),
        PyObject::None
            | PyObject::Bool(_)
            | PyObject::Int(_)
            | PyObject::Float(_)
            | PyObject::Complex(..)
            | PyObject::Str(_)
            | PyObject::Bytes(_)
            | PyObject::Tuple(_)
            | PyObject::Code { .. }
            | PyObject::BuiltinFunction { .. }
    ) && matches!(obj, PyObjectRef::Mut(_) | PyObjectRef::Imm(_))
}
fn target_ptr(obj: &PyObjectRef) -> Option<usize> {
    match obj {
        PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => Some(std::rc::Rc::as_ptr(rc) as usize),
        _ => None,
    }
}
fn register_weakref(target: usize, weakref_obj: &PyObjectRef, callback: Option<PyObjectRef>) {
    let weak = match weakref_obj {
        PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => std::rc::Rc::downgrade(rc),
        _ => return,
    };
    WEAKREF_REGISTRY.with(|r| {
        let mut m = r.borrow_mut();
        let entry = WeakEntry { weakref: weak, callback };
        m.entry(target).or_default().push(entry);
    });
}
fn find_shared_weakref(target: usize) -> Option<PyObjectRef> {
    WEAKREF_REGISTRY.with(|r| {
        let mut m = r.borrow_mut();
        if let Some(vec) = m.get_mut(&target) {
            vec.retain(|e| e.weakref.upgrade().is_some());
            for e in vec.iter() {
                if e.callback.is_none() {
                    if let Some(rc) = e.weakref.upgrade() {
                        return Some(PyObjectRef::Imm(rc));
                    }
                }
            }
        }
        None
    })
}
pub fn run_weakref_callbacks() {
    let mut to_call: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
    WEAKREF_REGISTRY.with(|r| {
        let mut m = r.borrow_mut();
        let keys: Vec<usize> = m.keys().cloned().collect();
        for k in keys {
            let mut live_entries = Vec::new();
            if let Some(vec) = m.get(&k) {
                for e in vec {
                    if let Some(rc) = e.weakref.upgrade() {
                        let target_alive = {
                            let b = rc.borrow();
                            match &*b {
                                PyObject::WeakRef { target, .. } | PyObject::WeakProxy { target, .. } => target.upgrade().is_some(),
                                _ => true,
                            }
                        };
                        if target_alive {
                            live_entries.push(WeakEntry { weakref: std::rc::Rc::downgrade(&rc), callback: e.callback.clone() });
                        } else {
                            if let Some(cb) = e.callback.clone() {
                                let wr_obj = PyObjectRef::Imm(rc.clone());
                                to_call.push((wr_obj, cb));
                            }
                        }
                    }
                }
            }
            if live_entries.is_empty() {
                m.remove(&k);
            } else {
                m.insert(k, live_entries);
            }
        }
    });
    for (wr, cb) in to_call {
        let _ = crate::object::call_function_disposable(&cb, vec![wr.clone()], vec![]);
    }
}

pub fn create_weakref_weak_val_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakValueDictionary".to_string(),
        func: |args| {
            if args.len() > 0 {
                // Copy items from the argument
                if let PyObject::Dict(items) = &*args[0].borrow() {
                    let mut new_dict = PyDict::new();
                    for (k, v) in items.items() {
                        let _ = new_dict.set(k, v);
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))));
                }
            }
            Ok(py_dict())
        },
    })
}

pub fn create_weakref_weak_key_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakKeyDictionary".to_string(),
        func: |args| {
            if args.len() > 0 {
                if let PyObject::Dict(items) = &*args[0].borrow() {
                    let mut new_dict = PyDict::new();
                    for (k, v) in items.items() {
                        let _ = new_dict.set(k, v);
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))));
                }
            }
            Ok(py_dict())
        },
    })
}

pub fn create_weakref_weak_set() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakSet".to_string(),
        func: |args| {
            if args.len() > 0 {
                if let PyObject::Set(_s) = &*args[0].borrow() {
                    return Ok(args[0].clone());
                }
                if let PyObject::List(items) = &*args[0].borrow() {
                    let mut s = PySet::new();
                    for item in items {
                        let _ = s.add(item.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Set(s)));
                }
            }
            Ok(PyObjectRef::new(PyObject::Set(PySet::new())))
        },
    })
}


pub fn create_weakref_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! wr_func {
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

    // ref(obj) returns a REAL weak reference backed by `Rc::downgrade` —
    // it does NOT keep the referent alive (the old implementation wrapped
    // the object as a BuiltinMethod's `self_obj`, i.e. a strong reference,
    // which defeated the entire point and made every `weakref.ref(x)`
    // permanently pin x). Calling the returned object yields the referent
    // while it lives, `None` once collected.
    wr_func!("ref", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("ref() requires at least 1 argument"));
        }
        let obj = &args[0];
        if !is_weakrefable(obj) {
            return Err(PyError::type_error(format!(
                "cannot create weak reference to '{}' object",
                obj.borrow().type_name()
            )));
        }
        let callback = if args.len() > 1 && !matches!(&*args[1].borrow(), PyObject::None) {
            Some(args[1].clone())
        } else { None };
        let tptr = target_ptr(obj).unwrap();
        if callback.is_none() {
            if let Some(existing) = find_shared_weakref(tptr) {
                return Ok(existing);
            }
        }
        let target = match obj {
            PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => std::rc::Rc::downgrade(rc),
            _ => unreachable!(),
        };
        let wr = PyObjectRef::imm(PyObject::WeakRef { target, callback: callback.clone() });
        register_weakref(tptr, &wr, callback);
        Ok(wr)
    });

    wr_func!("proxy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("proxy() requires at least 1 argument"));
        }
        let obj = &args[0];
        if !is_weakrefable(obj) {
            return Err(PyError::type_error(format!(
                "cannot create weak reference to '{}' object",
                obj.borrow().type_name()
            )));
        }
        let callback = if args.len() > 1 && !matches!(&*args[1].borrow(), PyObject::None) {
            Some(args[1].clone())
        } else { None };
        let tptr = target_ptr(obj).unwrap();
        if callback.is_none() {
            let existing = WEAKREF_REGISTRY.with(|r| {
                let m = r.borrow();
                if let Some(vec) = m.get(&tptr) {
                    for e in vec {
                        if e.callback.is_none() {
                            if let Some(rc) = e.weakref.upgrade() {
                                let b = rc.borrow();
                                if matches!(&*b, PyObject::WeakProxy { .. }) {
                                    return Some(PyObjectRef::Imm(rc.clone()));
                                }
                            }
                        }
                    }
                }
                None
            });
            if let Some(e) = existing { return Ok(e); }
        }
        let target = match obj {
            PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => std::rc::Rc::downgrade(rc),
            _ => unreachable!(),
        };
        let wr = PyObjectRef::imm(PyObject::WeakProxy { target, callback: callback.clone() });
        register_weakref(tptr, &wr, callback);
        Ok(wr)
    });

    wr_func!("getweakrefcount", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getweakrefcount() requires 1 argument"));
        }
        let obj = &args[0];
        let tptr = match target_ptr(obj) { Some(p) => p, None => return Ok(py_int(0)) };
        run_weakref_callbacks();
        let cnt = WEAKREF_REGISTRY.with(|r| {
            let mut m = r.borrow_mut();
            if let Some(vec) = m.get_mut(&tptr) {
                vec.retain(|e| {
                    if let Some(rc) = e.weakref.upgrade() {
                        let b = rc.borrow();
                        match &*b {
                            PyObject::WeakRef { target, .. } | PyObject::WeakProxy { target, .. } => target.upgrade().is_some(),
                            _ => false,
                        }
                    } else { false }
                });
                vec.len() as i64
            } else { 0 }
        });
        Ok(py_int(cnt))
    });
    wr_func!("getweakrefs", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getweakrefs() requires 1 argument"));
        }
        let obj = &args[0];
        let tptr = match target_ptr(obj) { Some(p) => p, None => return Ok(py_list(vec![])) };
        run_weakref_callbacks();
        let refs = WEAKREF_REGISTRY.with(|r| {
            let m = r.borrow();
            if let Some(vec) = m.get(&tptr) {
                vec.iter().filter_map(|e| {
                    if let Some(rc) = e.weakref.upgrade() {
                        let b = rc.borrow();
                        match &*b {
                            PyObject::WeakRef { target, .. } | PyObject::WeakProxy { target, .. } if target.upgrade().is_some() => Some(PyObjectRef::Imm(rc.clone())),
                            _ => None,
                        }
                    } else { None }
                }).collect::<Vec<_>>()
            } else { vec![] }
        });
        Ok(py_list(refs))
    });

    // finalize(obj, func, *args, **kwargs) — real semantics call `func` when
    // `obj` is garbage collected; this interpreter has no GC hooks to key
    // that off of, so this only supports the "call it directly" path
    // (finalize_obj()) — the common real-world use (e.g. Django's signal
    // dispatcher) just registers cleanup and never inspects the return
    // value, so not firing automatically on collection is a silent no-op
    // rather than a crash.
    wr_func!("finalize", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "finalize() requires at least 2 arguments (obj, func)",
            ));
        }
        let func = args[1].clone();
        let extra_args: Vec<PyObjectRef> = args[2..].to_vec();
        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "finalize".to_string(),
            func: |args| {
                // self_obj holds (func, extra_args) packed as a tuple
                if let PyObject::Tuple(items) = &*args[0].borrow() {
                    let func = items[0].clone();
                    let extra = if let PyObject::Tuple(a) = &*items[1].borrow() {
                        a.clone()
                    } else {
                        vec![]
                    };
                    return call_function(&func, extra);
                }
                Ok(py_none())
            },
            self_obj: PyObjectRef::imm(PyObject::Tuple(vec![
                func,
                PyObjectRef::imm(PyObject::Tuple(extra_args)),
            ])),
        }))
    });

    // WeakMethod(bound_method) — like ref() but for bound methods; same
    // simplification as ref() above (no real weak semantics, just holds on).
    wr_func!("WeakMethod", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "WeakMethod() requires at least 1 argument",
            ));
        }
        let obj = args[0].clone();
        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "weakmethod".to_string(),
            func: |args| Ok(args[0].clone()),
            self_obj: obj,
        }))
    });

    // Type constants
    d.insert_str("ReferenceType", py_str("weakref"));
    d.insert_str("ProxyType", py_str("weakproxy"));
    d.insert_str("CallableProxyType", py_str("weakcallableproxy"));

    // Internal function used by weakrefset: _remove_dead_weakref(dict, key)
    wr_func!("_remove_dead_weakref", |args| {
        if args.len() >= 2 {
            let dict = &args[0];
            let key = &args[1];
            if matches!(&*dict.borrow(), PyObject::Dict(_)) {
                let _ = crate::object::pydict_safe_remove(dict, key);
            }
        }
        Ok(py_none())
    });

    d
}
