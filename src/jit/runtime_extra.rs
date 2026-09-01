use crate::object::PyObjectRef;
use super::globals::CURRENT_JIT_GLOBALS;
// FOR_ITER: calls __next__, returns 0 on success (value in out), 1 on StopIteration
pub(crate) extern "C" fn jit_for_iter(iter: *const PyObjectRef, out: *mut PyObjectRef) -> i64 {
    unsafe {
        use crate::object::ObjectAccess;
        let iter_ref = &*iter;
        let next_method = iter_ref.borrow().get_attribute("__next__").ok();
        if let Some(method) = next_method {
            match crate::object::call_bound_method(method, (*iter).clone(), vec![]) {
                Ok(val) => {
                    std::ptr::write(out, val);
                    0
                }
                Err(_) => {
                    std::ptr::write(out, crate::object::py_none());
                    1
                }
            }
        } else {
            std::ptr::write(out, crate::object::py_none());
            1
        }
    }
}

// BUILD_MAP: n key-value pairs as flat array
pub(crate) extern "C" fn jit_build_map(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut d = crate::object::PyDict::new();
        for i in 0..n as isize {
            let key = &*items.offset(i * 2);
            let val = &*items.offset(i * 2 + 1);
            let _ = d.set(key.clone(), val.clone());
        }
        std::ptr::write(
            out,
            crate::object::PyObjectRef::new(crate::object::PyObject::Dict(Box::new(d))),
        );
    }
}

// STORE_ATTR: obj.name = val
pub(crate) extern "C" fn jit_store_attr(
    obj: *const PyObjectRef,
    names: *const PyObjectRef,
    name_idx: i64,
    val: *const PyObjectRef,
    out: *mut PyObjectRef,
) {
    unsafe {
        use crate::object::ObjectAccess;
        let name_obj = &*names.offset(name_idx as isize);
        let name_str = name_obj.str();
        let obj_ref = &*obj;
        let val_ref = &*val;
        let _ = obj_ref
            .borrow_mut()
            .set_attribute(&name_str, val_ref.clone());
        std::ptr::write(out, (*val).clone());
    }
}

// UNPACK_SEQUENCE: unpack iterable into n items, returns 0 on success
pub(crate) extern "C" fn jit_unpack_sequence(
    seq: *const PyObjectRef,
    n: i64,
    items: *mut PyObjectRef,
    out: *mut PyObjectRef,
) -> i64 {
    unsafe {
        let seq_ref = &*seq;
        let mut collected: Vec<PyObjectRef> = Vec::new();
        // Try sequence protocol: __getitem__
        if let Ok(len) =
            crate::object::builtin_len(&[seq_ref.clone()]).map(|l| l.as_i64().unwrap_or(0))
        {
            if len == n {
                for i in 0..n as isize {
                    let idx = crate::object::py_int(i as i64);
                    let item = crate::object::py_getitem(seq_ref, &idx);
                    if let Ok(item) = item {
                        collected.push(item);
                    } else {
                        return 1;
                    }
                }
                for (i, item) in collected.iter().enumerate() {
                    std::ptr::write(items.offset(i as isize), item.clone());
                }
                std::ptr::write(out, crate::object::py_none());
                return 0;
            }
        }
        1
    }
}

// IMPORT_NAME: import a module by name
pub(crate) extern "C" fn jit_import_name(
    consts: *const PyObjectRef,
    names_offset: i64,
    name_idx: i64,
    out: *mut PyObjectRef,
) {
    unsafe {
        let name_obj = &*consts.offset((names_offset + name_idx) as isize);
        let name = name_obj.str();
        crate::object::VM_PTR.with(|p| {
            if let Some(ptr) = p.get() {
                let vm = unsafe { &mut *ptr };
                if let Some(module) = vm.modules.get(&name) {
                    std::ptr::write(out, module.clone());
                } else if let Ok(module) = vm.import_module_from_file(&name) {
                    vm.modules.insert(name.clone(), module.clone());
                    std::ptr::write(out, module);
                } else {
                    std::ptr::write(out, crate::object::py_none());
                }
            } else {
                std::ptr::write(out, crate::object::py_none());
            }
        });
    }
}

// IMPORT_FROM: import an attribute from a module
pub(crate) extern "C" fn jit_import_from(
    module_ptr: *const PyObjectRef,
    consts: *const PyObjectRef,
    names_offset: i64,
    name_idx: i64,
    out: *mut PyObjectRef,
) {
    unsafe {
        use crate::object::DictMap;
        use crate::object::ObjectAccess;
        let name_obj = &*consts.offset((names_offset + name_idx) as isize);
        let name = name_obj.str();
        let module_ref = &*module_ptr;
        let obj = module_ref.borrow();
        if let crate::object::PyObject::Module { dict, .. } = &*obj {
            if let Some(val) = dict.get_str(&name) {
                std::ptr::write(out, val.clone());
                return;
            }
        }
        // Try get_attribute fallback
        drop(obj);
        if let Ok(val) = module_ref.borrow().get_attribute(&name) {
            std::ptr::write(out, val);
        } else {
            std::ptr::write(out, crate::object::py_none());
        }
    }
}

// UNPACK_EX: unpack iterable with starred target (a, *b, c = seq)
pub(crate) extern "C" fn jit_unpack_ex(
    seq: *const PyObjectRef,
    n_before: i64,
    n_after: i64,
    items: *mut PyObjectRef,
    out: *mut PyObjectRef,
) -> i64 {
    unsafe {
        let seq_ref = &*seq;
        let mut collected: Vec<PyObjectRef> = Vec::new();
        if let Ok(len) =
            crate::object::builtin_len(&[seq_ref.clone()]).map(|l| l.as_i64().unwrap_or(0))
        {
            let nb = n_before as usize;
            let na = n_after as usize;
            let total = len as usize;
            if total >= nb + na {
                let star_count = total - nb - na;
                // Collect items before *
                for i in 0..nb {
                    let idx = crate::object::py_int(i as i64);
                    if let Ok(item) = crate::object::py_getitem(seq_ref, &idx) {
                        collected.push(item);
                    } else {
                        return 1;
                    }
                }
                // Starred portion as a list
                let mut star_items = Vec::with_capacity(star_count);
                for i in nb..nb + star_count {
                    let idx = crate::object::py_int(i as i64);
                    if let Ok(item) = crate::object::py_getitem(seq_ref, &idx) {
                        star_items.push(item);
                    } else {
                        return 1;
                    }
                }
                collected.push(crate::object::py_list(star_items));
                // Collect items after *
                for i in nb + star_count..total {
                    let idx = crate::object::py_int(i as i64);
                    if let Ok(item) = crate::object::py_getitem(seq_ref, &idx) {
                        collected.push(item);
                    } else {
                        return 1;
                    }
                }
                for (i, item) in collected.iter().enumerate() {
                    std::ptr::write(items.offset(i as isize), item.clone());
                }
                std::ptr::write(out, crate::object::py_none());
                return 0;
            }
        }
        1
    }
}

// SETUP_WITH: call __enter__ on a context manager, push result
pub(crate) extern "C" fn jit_setup_with(mgr: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let mgr_ref = &*mgr;
        let enter_raw = mgr_ref.borrow().get_attribute("__enter__").ok();
        if let Some(enter_raw) = enter_raw {
            let is_builtin = matches!(
                &*enter_raw.borrow(),
                crate::object::PyObject::BuiltinMethod { .. }
            );
            let enter = if is_builtin {
                let b = enter_raw.borrow();
                match &*b {
                    crate::object::PyObject::BuiltinMethod { name, func, .. } => {
                        crate::object::PyObjectRef::imm(crate::object::PyObject::BuiltinMethod {
                            name: name.clone(),
                            func: *func,
                            self_obj: mgr_ref.clone(),
                        })
                    }
                    _ => unreachable!(),
                }
            } else {
                crate::object::PyObjectRef::imm(crate::object::PyObject::BoundMethod {
                    func: enter_raw,
                    self_obj: mgr_ref.clone(),
                })
            };
            let result = crate::object::call_bound_method(enter, mgr_ref.clone(), vec![])
                .unwrap_or_else(|_| crate::object::py_none());
            std::ptr::write(out, result);
        } else {
            std::ptr::write(out, crate::object::py_none());
        }
    }
}

// WITH_EXIT: call __exit__ on a context manager with (None, None, None)
pub(crate) extern "C" fn jit_with_exit(mgr: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let mgr_ref = &*mgr;
        let exit_raw = mgr_ref.borrow().get_attribute("__exit__").ok();
        if let Some(exit_raw) = exit_raw {
            let is_builtin = matches!(
                &*exit_raw.borrow(),
                crate::object::PyObject::BuiltinMethod { .. }
            );
            let exit = if is_builtin {
                let b = exit_raw.borrow();
                match &*b {
                    crate::object::PyObject::BuiltinMethod { name, func, .. } => {
                        crate::object::PyObjectRef::imm(crate::object::PyObject::BuiltinMethod {
                            name: name.clone(),
                            func: *func,
                            self_obj: mgr_ref.clone(),
                        })
                    }
                    _ => unreachable!(),
                }
            } else {
                crate::object::PyObjectRef::imm(crate::object::PyObject::BoundMethod {
                    func: exit_raw,
                    self_obj: mgr_ref.clone(),
                })
            };
            let none = crate::object::py_none();
            let result = crate::object::call_bound_method(
                exit,
                mgr_ref.clone(),
                vec![none.clone(), none.clone(), none.clone()],
            )
            .unwrap_or_else(|_| crate::object::py_none());
            std::ptr::write(out, result);
        } else {
            std::ptr::write(out, crate::object::py_none());
        }
    }
}

// LOAD_NAME: lookup in locals, globals, builtins
pub(crate) extern "C" fn jit_load_name(
    names: *const PyObjectRef,
    name_idx: i64,
    locals: *const PyObjectRef,
    globals: *const PyObjectRef,
    out: *mut PyObjectRef,
) {
    unsafe {
        use crate::object::ObjectAccess;
        let name_obj = &*names.offset(name_idx as isize);
        let name_str = name_obj.str();
        // Try locals first (as a dict), then globals, then builtins
        // Locals reference
        let locals_ref = &*locals;
        let result = locals_ref
            .borrow()
            .get_attribute(&name_str)
            .ok()
            .or_else(|| {
                let globals_ref = &*globals;
                globals_ref.borrow().get_attribute(&name_str).ok()
            })
            .unwrap_or_else(|| crate::object::py_none());
        std::ptr::write(out, result);
    }
}

pub(crate) extern "C" fn jit_build_set(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut s = crate::object::PySet::new();
        for i in 0..n as isize {
            let _ = s.add((*items.offset(i)).clone());
        }
        std::ptr::write(
            out,
            crate::object::PyObjectRef::new(crate::object::PyObject::Set(s)),
        );
    }
}

pub(crate) extern "C" fn jit_build_string(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut parts = Vec::with_capacity(n as usize);
        for i in 0..n as isize {
            parts.push((*items.offset(i)).str());
        }
        std::ptr::write(out, crate::object::py_str(&parts.join("")));
    }
}

pub(crate) extern "C" fn jit_build_slice(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let start = if n >= 1 {
            Some((*items.offset(0)).clone())
        } else {
            None
        };
        let stop = if n >= 2 {
            Some((*items.offset(1)).clone())
        } else {
            None
        };
        let step = if n >= 3 {
            Some((*items.offset(2)).clone())
        } else {
            None
        };
        std::ptr::write(
            out,
            crate::object::PyObjectRef::imm(crate::object::PyObject::Slice {
                start: start.unwrap_or_else(|| crate::object::py_none()),
                stop: stop.unwrap_or_else(|| crate::object::py_none()),
                step: step.unwrap_or_else(|| crate::object::py_none()),
            }),
        );
    }
}

pub(crate) extern "C" fn jit_store_subscr(
    obj: *const PyObjectRef,
    idx: *const PyObjectRef,
    val: *const PyObjectRef,
    out: *mut PyObjectRef,
) {
    unsafe {
        let _ = crate::object::py_setitem(&*obj, &*idx, (*val).clone());
        std::ptr::write(out, crate::object::py_none());
    }
}

pub(crate) extern "C" fn jit_is_op(
    a: *const PyObjectRef,
    b: *const PyObjectRef,
    invert: i64,
    out: *mut PyObjectRef,
) {
    unsafe {
        let result = (*a).is(&*b);
        let result = if invert != 0 { !result } else { result };
        std::ptr::write(out, crate::object::py_bool(result));
    }
}

pub(crate) extern "C" fn jit_make_function(
    items: *const crate::object::PyObjectRef,
    arg: i64,
    out: *mut crate::object::PyObjectRef,
) {
    unsafe {
        let has_closure = (arg & 0x100) != 0;
        let n_defaults = (arg & 0xFF) as usize;
        let n_kwdefaults = ((arg >> 9) & 0xFF) as usize;
        let mut total = n_defaults + n_kwdefaults;
        let mut kwdefaults: Vec<crate::object::PyObjectRef> = Vec::new();
        for _ in 0..n_kwdefaults {
            kwdefaults.push((*items.offset(total as isize - 1)).clone());
            total -= 1;
        }
        let mut defaults: Vec<crate::object::PyObjectRef> = Vec::new();
        for _ in 0..n_defaults {
            defaults.push((*items.offset(total as isize - 1)).clone());
            total -= 1;
        }
        defaults.reverse();
        kwdefaults.reverse();
        defaults.extend(kwdefaults);
        let code_obj = (*items.offset(total as isize - 1)).clone();
        total -= 1;
        let code = match &*code_obj.borrow() {
            crate::object::PyObject::Code(c) => c.clone(),
            _ => {
                std::ptr::write(out, crate::object::py_none());
                return;
            }
        };
        let closure = if has_closure {
            let closure_tuple = (*items.offset(total as isize - 1)).clone();
            total -= 1;
            let items_b = closure_tuple.borrow();
            if let crate::object::PyObject::Tuple(items_v) = &*items_b {
                items_v.clone()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let globals = CURRENT_JIT_GLOBALS.with(|g| g.borrow().clone());
        let func_obj = match globals {
            Some(g) => {
                let code_rc = code.clone();
                let func = crate::object::PyObject::Function(Box::new(crate::object::PyFunction {
                    code,
                    globals: g,
                    defaults,
                    closure,
                    dict: std::collections::HashMap::new(),
                    jit_ptr: std::cell::Cell::new(0),
                    jit_consts: std::cell::RefCell::new(Vec::new()),
                }));
                let func_ref = crate::object::PyObjectRef::new(func);
                // Set __code__ and __module__
                if let crate::object::PyObject::Function(ref mut inner_f) =
                    &mut *func_ref.borrow_mut()
                {
                    let dict = &mut inner_f.dict;
                    dict.insert(
                        "__code__".to_string(),
                        crate::object::PyObjectRef::imm(crate::object::PyObject::Code(code_rc)),
                    );
                    let mg_name = CURRENT_JIT_GLOBALS.with(|g| {
                        g.borrow().as_ref().and_then(|g| {
                            let b = g.borrow();
                            b.get(&crate::interner::intern("__name__")).cloned()
                        })
                    });
                    if let Some(name) = mg_name {
                        dict.insert("__module__".to_string(), name);
                    }
                }
                func_ref
            }
            None => crate::object::py_none(),
        };
        std::ptr::write(out, func_obj);
    }
}

pub(crate) extern "C" fn jit_invert(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let borrowed = (*val).borrow();
        match &*borrowed {
            crate::object::PyObject::Int(i) => {
                std::ptr::write(out, crate::object::py_int(!(i.clone())))
            }
            _ => std::ptr::write(out, crate::object::py_none()),
        }
    }
}
