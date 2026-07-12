use std::collections::HashSet;
use std::collections::HashMap;
use std::cell::RefCell;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use crate::object::PyObjectRef;
use crate::bytecode::*;

extern "C" fn jit_py_add(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_add(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_sub(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_sub(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_mul(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_mul(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_div(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_div(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_floor_div(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_floor_div(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_mod(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_mod(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_pow(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_pow(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_lshift(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_lshift(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_rshift(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_rshift(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_bit_and(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_bit_and(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_bit_or(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_bit_or(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_bit_xor(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_bit_xor(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_compare(a: *const PyObjectRef, b: *const PyObjectRef, op: i64, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_compare(&*a, &*b, op as u32).unwrap_or_else(|_| crate::object::py_bool(false))); }
}
extern "C" fn jit_is_true(val: *const PyObjectRef) -> i64 {
    unsafe { (*val).truthy() as i64 }
}
extern "C" fn jit_neg(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_neg(&*val).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_not(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_not(&*val)); }
}
extern "C" fn jit_build_list(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut v: Vec<PyObjectRef> = Vec::with_capacity(n as usize);
        for i in 0..n as isize {
            v.push((*items.offset(i)).clone());
        }
        std::ptr::write(out, crate::object::py_list(v));
    }
}
extern "C" fn jit_build_tuple(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut v: Vec<PyObjectRef> = Vec::with_capacity(n as usize);
        for i in 0..n as isize {
            v.push((*items.offset(i)).clone());
        }
        std::ptr::write(out, crate::object::py_tuple(v));
    }
}
extern "C" fn jit_list_append(lst: *const PyObjectRef, val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let crate::object::PyObject::List(v) = &mut *(*lst).borrow_mut() {
            v.push((*val).clone());
        }
        std::ptr::write(out, crate::object::py_none());
    }
}
extern "C" fn jit_contains(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let result = crate::object::py_contains(&*a, &*b).unwrap_or_else(|_| crate::object::py_bool(false));
        std::ptr::write(out, result);
    }
}
extern "C" fn jit_get_iter(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let obj = &*val;
        let iter_method = obj.borrow().get_attribute("__iter__").ok();
        let result = if let Some(method) = iter_method {
            crate::object::call_bound_method(method, (*val).clone(), vec![]).unwrap_or_else(|_| (*val).clone())
        } else {
            (*val).clone()
        };
        std::ptr::write(out, result);
    }
}

extern "C" fn jit_call(func: *const PyObjectRef, nargs: i64, args: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let func_ref = &*func;
        let mut v: Vec<PyObjectRef> = Vec::with_capacity(nargs as usize);
        for i in 0..nargs as isize {
            v.push((*args.offset(i)).clone());
        }
        std::ptr::write(out, crate::object::call_function(func_ref, v).unwrap_or_else(|_| crate::object::py_none()));
    }
}
thread_local! {
    static ATTR_CACHE: std::cell::RefCell<std::collections::HashMap<(String, String), crate::object::PyObjectRef>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

extern "C" fn jit_load_attr(obj: *const PyObjectRef, names: *const PyObjectRef, name_idx: i64, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let name_obj = &*names.offset(name_idx as isize);
        let name_str = name_obj.str();
        let obj_ref = &*obj;
        let type_name = obj_ref.borrow().type_name();
        let cache_key = (type_name.clone(), name_str.clone());
        // Check thread-local cache first (type-aware)
        let cached = ATTR_CACHE.with(|cache| {
            cache.borrow().get(&cache_key).cloned()
        });
        if let Some(val) = cached {
            std::ptr::write(out, val);
            return;
        }
        let result = obj_ref.borrow().get_attribute(&name_str).unwrap_or_else(|_| crate::object::py_none());
        // Cache for next time
        ATTR_CACHE.with(|cache| {
            cache.borrow_mut().insert(cache_key, result.clone());
        });
        std::ptr::write(out, result);
    }
}

// FOR_ITER: calls __next__, returns 0 on success (value in out), 1 on StopIteration
extern "C" fn jit_for_iter(iter: *const PyObjectRef, out: *mut PyObjectRef) -> i64 {
    unsafe {
        use crate::object::ObjectAccess;
        let iter_ref = &*iter;
        let next_method = iter_ref.borrow().get_attribute("__next__").ok();
        if let Some(method) = next_method {
            match crate::object::call_bound_method(method, (*iter).clone(), vec![]) {
                Ok(val) => { std::ptr::write(out, val); 0 }
                Err(_) => { std::ptr::write(out, crate::object::py_none()); 1 }
            }
        } else {
            std::ptr::write(out, crate::object::py_none()); 1
        }
    }
}

// BUILD_MAP: n key-value pairs as flat array
extern "C" fn jit_build_map(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut d = crate::object::PyDict::new();
        for i in 0..n as isize {
            let key = &*items.offset(i * 2);
            let val = &*items.offset(i * 2 + 1);
            let _ = d.set(key.clone(), val.clone());
        }
        std::ptr::write(out, crate::object::PyObjectRef::new(crate::object::PyObject::Dict(d)));
    }
}

// STORE_ATTR: obj.name = val
extern "C" fn jit_store_attr(obj: *const PyObjectRef, names: *const PyObjectRef, name_idx: i64, val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let name_obj = &*names.offset(name_idx as isize);
        let name_str = name_obj.str();
        let obj_ref = &*obj;
        let val_ref = &*val;
        let _ = obj_ref.borrow_mut().set_attribute(&name_str, val_ref.clone());
        std::ptr::write(out, crate::object::py_none());
    }
}

// UNPACK_SEQUENCE: unpack iterable into n items, returns 0 on success
extern "C" fn jit_unpack_sequence(seq: *const PyObjectRef, n: i64, items: *mut PyObjectRef, out: *mut PyObjectRef) -> i64 {
    unsafe {
        let seq_ref = &*seq;
        let mut collected: Vec<PyObjectRef> = Vec::new();
        // Try sequence protocol: __getitem__
        if let Ok(len) = crate::object::builtin_len(&[seq_ref.clone()]).map(|l| l.as_i64().unwrap_or(0)) {
            if len == n {
                for i in 0..n as isize {
                    let idx = crate::object::py_int(i as i64);
                    let item = crate::object::py_getitem(seq_ref, &idx);
                    if let Ok(item) = item {
                        collected.push(item);
                    } else { return 1; }
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
extern "C" fn jit_import_name(consts: *const PyObjectRef, names_offset: i64, name_idx: i64, out: *mut PyObjectRef) {
    unsafe {
        let name_obj = &*consts.offset((names_offset + name_idx) as isize);
        let name = name_obj.str();
        crate::object::VM_PTR.with(|p| {
            if let Some(ptr) = *p.borrow() {
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
extern "C" fn jit_import_from(module_ptr: *const PyObjectRef, consts: *const PyObjectRef, names_offset: i64, name_idx: i64, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let name_obj = &*consts.offset((names_offset + name_idx) as isize);
        let name = name_obj.str();
        let module_ref = &*module_ptr;
        let obj = module_ref.borrow();
        if let crate::object::PyObject::Module { dict, .. } = &*obj {
            if let Some(val) = dict.get(&name) {
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
extern "C" fn jit_unpack_ex(seq: *const PyObjectRef, n_before: i64, n_after: i64, items: *mut PyObjectRef, out: *mut PyObjectRef) -> i64 {
    unsafe {
        let seq_ref = &*seq;
        let mut collected: Vec<PyObjectRef> = Vec::new();
        if let Ok(len) = crate::object::builtin_len(&[seq_ref.clone()]).map(|l| l.as_i64().unwrap_or(0)) {
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
                    } else { return 1; }
                }
                // Starred portion as a list
                let mut star_items = Vec::with_capacity(star_count);
                for i in nb..nb + star_count {
                    let idx = crate::object::py_int(i as i64);
                    if let Ok(item) = crate::object::py_getitem(seq_ref, &idx) {
                        star_items.push(item);
                    } else { return 1; }
                }
                collected.push(crate::object::py_list(star_items));
                // Collect items after *
                for i in nb + star_count..total {
                    let idx = crate::object::py_int(i as i64);
                    if let Ok(item) = crate::object::py_getitem(seq_ref, &idx) {
                        collected.push(item);
                    } else { return 1; }
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
extern "C" fn jit_setup_with(mgr: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let mgr_ref = &*mgr;
        let enter_raw = mgr_ref.borrow().get_attribute("__enter__").ok();
        if let Some(enter_raw) = enter_raw {
            let is_builtin = matches!(&*enter_raw.borrow(), crate::object::PyObject::BuiltinMethod { .. });
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
extern "C" fn jit_with_exit(mgr: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let mgr_ref = &*mgr;
        let exit_raw = mgr_ref.borrow().get_attribute("__exit__").ok();
        if let Some(exit_raw) = exit_raw {
            let is_builtin = matches!(&*exit_raw.borrow(), crate::object::PyObject::BuiltinMethod { .. });
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
            let result = crate::object::call_bound_method(exit, mgr_ref.clone(), vec![none.clone(), none.clone(), none.clone()])
                .unwrap_or_else(|_| crate::object::py_none());
            std::ptr::write(out, result);
        } else {
            std::ptr::write(out, crate::object::py_none());
        }
    }
}

// LOAD_NAME: lookup in locals, globals, builtins
extern "C" fn jit_load_name(names: *const PyObjectRef, name_idx: i64, locals: *const PyObjectRef, globals: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let name_obj = &*names.offset(name_idx as isize);
        let name_str = name_obj.str();
        // Try locals first (as a dict), then globals, then builtins
        // Locals reference
        let locals_ref = &*locals;
        let result = locals_ref.borrow().get_attribute(&name_str).ok()
            .or_else(|| {
                let globals_ref = &*globals;
                globals_ref.borrow().get_attribute(&name_str).ok()
            })
            .unwrap_or_else(|| {
                crate::object::py_none()
            });
        std::ptr::write(out, result);
    }
}

extern "C" fn jit_build_set(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut s = crate::object::PySet::new();
        for i in 0..n as isize {
            let _ = s.add((*items.offset(i)).clone());
        }
        std::ptr::write(out, crate::object::PyObjectRef::new(crate::object::PyObject::Set(s)));
    }
}

extern "C" fn jit_build_string(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut parts = Vec::with_capacity(n as usize);
        for i in 0..n as isize {
            parts.push((*items.offset(i)).str());
        }
        std::ptr::write(out, crate::object::py_str(&parts.join("")));
    }
}

extern "C" fn jit_build_slice(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let start = if n >= 1 { Some((*items.offset(0)).clone()) } else { None };
        let stop = if n >= 2 { Some((*items.offset(1)).clone()) } else { None };
        let step = if n >= 3 { Some((*items.offset(2)).clone()) } else { None };
        std::ptr::write(out, crate::object::PyObjectRef::imm(crate::object::PyObject::Slice {
            start: start.unwrap_or_else(|| crate::object::py_none()),
            stop: stop.unwrap_or_else(|| crate::object::py_none()),
            step: step.unwrap_or_else(|| crate::object::py_none()),
        }));
    }
}

extern "C" fn jit_store_subscr(obj: *const PyObjectRef, idx: *const PyObjectRef, val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let _ = crate::object::py_setitem(&*obj, &*idx, (*val).clone());
        std::ptr::write(out, crate::object::py_none());
    }
}

extern "C" fn jit_is_op(a: *const PyObjectRef, b: *const PyObjectRef, invert: i64, out: *mut PyObjectRef) {
    unsafe {
        let result = (*a).is(&*b);
        let result = if invert != 0 { !result } else { result };
        std::ptr::write(out, crate::object::py_bool(result));
    }
}

extern "C" fn jit_invert(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let borrowed = (*val).borrow();
        match &*borrowed {
            crate::object::PyObject::Int(i) => std::ptr::write(out, crate::object::py_int(!(i.clone()))),
            _ => std::ptr::write(out, crate::object::py_none()),
        }
    }
}

pub struct JitCompiler {
    builder_context: FunctionBuilderContext,
    module: JITModule,
    add_func: cranelift_module::FuncId,
    sub_func: cranelift_module::FuncId,
    mul_func: cranelift_module::FuncId,
    div_func: cranelift_module::FuncId,
    floor_div_func: cranelift_module::FuncId,
    mod_func: cranelift_module::FuncId,
    pow_func: cranelift_module::FuncId,
    lshift_func: cranelift_module::FuncId,
    rshift_func: cranelift_module::FuncId,
    bit_and_func: cranelift_module::FuncId,
    bit_or_func: cranelift_module::FuncId,
    bit_xor_func: cranelift_module::FuncId,
    cmp_func: cranelift_module::FuncId,
    truthy_func: cranelift_module::FuncId,
    neg_func: cranelift_module::FuncId,
    not_func: cranelift_module::FuncId,
    build_list_func: cranelift_module::FuncId,
    build_tuple_func: cranelift_module::FuncId,
    list_append_func: cranelift_module::FuncId,
    contains_func: cranelift_module::FuncId,
    get_iter_func: cranelift_module::FuncId,
    call_func: cranelift_module::FuncId,
    load_attr_func: cranelift_module::FuncId,
    for_iter_func: cranelift_module::FuncId,
    build_map_func: cranelift_module::FuncId,
    store_attr_func: cranelift_module::FuncId,
    unpack_sequence_func: cranelift_module::FuncId,
    load_name_func: cranelift_module::FuncId,
    build_set_func: cranelift_module::FuncId,
    build_string_func: cranelift_module::FuncId,
    build_slice_func: cranelift_module::FuncId,
    store_subscr_func: cranelift_module::FuncId,
    is_op_func: cranelift_module::FuncId,
    invert_func: cranelift_module::FuncId,
    import_name_func: cranelift_module::FuncId,
    import_from_func: cranelift_module::FuncId,
    unpack_ex_func: cranelift_module::FuncId,
    setup_with_func: cranelift_module::FuncId,
    with_exit_func: cranelift_module::FuncId,
}

impl JitCompiler {
    pub fn new() -> Self {
        let flag_builder = settings::builder();
        let flags = settings::Flags::new(flag_builder);
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder.finish(flags).unwrap();
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("jit_py_add", jit_py_add as *const u8);
        builder.symbol("jit_py_sub", jit_py_sub as *const u8);
        builder.symbol("jit_py_mul", jit_py_mul as *const u8);
        builder.symbol("jit_py_div", jit_py_div as *const u8);
        builder.symbol("jit_py_floor_div", jit_py_floor_div as *const u8);
        builder.symbol("jit_py_mod", jit_py_mod as *const u8);
        builder.symbol("jit_py_pow", jit_py_pow as *const u8);
        builder.symbol("jit_py_lshift", jit_py_lshift as *const u8);
        builder.symbol("jit_py_rshift", jit_py_rshift as *const u8);
        builder.symbol("jit_py_bit_and", jit_py_bit_and as *const u8);
        builder.symbol("jit_py_bit_or", jit_py_bit_or as *const u8);
        builder.symbol("jit_py_bit_xor", jit_py_bit_xor as *const u8);
        builder.symbol("jit_py_compare", jit_py_compare as *const u8);
        builder.symbol("jit_is_true", jit_is_true as *const u8);
        builder.symbol("jit_neg", jit_neg as *const u8);
        builder.symbol("jit_not", jit_not as *const u8);
        builder.symbol("jit_build_list", jit_build_list as *const u8);
        builder.symbol("jit_build_tuple", jit_build_tuple as *const u8);
        builder.symbol("jit_list_append", jit_list_append as *const u8);
        builder.symbol("jit_contains", jit_contains as *const u8);
        builder.symbol("jit_get_iter", jit_get_iter as *const u8);
        builder.symbol("jit_call", jit_call as *const u8);
        builder.symbol("jit_load_attr", jit_load_attr as *const u8);
        builder.symbol("jit_for_iter", jit_for_iter as *const u8);
        builder.symbol("jit_build_map", jit_build_map as *const u8);
        builder.symbol("jit_store_attr", jit_store_attr as *const u8);
        builder.symbol("jit_unpack_sequence", jit_unpack_sequence as *const u8);
        builder.symbol("jit_load_name", jit_load_name as *const u8);
        builder.symbol("jit_build_set", jit_build_set as *const u8);
        builder.symbol("jit_build_string", jit_build_string as *const u8);
        builder.symbol("jit_build_slice", jit_build_slice as *const u8);
        builder.symbol("jit_store_subscr", jit_store_subscr as *const u8);
        builder.symbol("jit_is_op", jit_is_op as *const u8);
        builder.symbol("jit_invert", jit_invert as *const u8);
        builder.symbol("jit_import_name", jit_import_name as *const u8);
        builder.symbol("jit_import_from", jit_import_from as *const u8);
        builder.symbol("jit_unpack_ex", jit_unpack_ex as *const u8);
        builder.symbol("jit_setup_with", jit_setup_with as *const u8);
        builder.symbol("jit_with_exit", jit_with_exit as *const u8);
        let mut module = JITModule::new(builder);
        let binop_sig = Self::make_binop_sig();
        let cmp_sig = Self::make_cmp_sig();
        let truthy_sig = Self::make_truthy_sig();
        let unary_sig = Self::make_unary_sig();
        let call_sig = Self::make_call_sig();
        let load_attr_sig = Self::make_load_attr_sig();
        let store_attr_sig = Self::make_store_attr_sig();
        let unpack_sig = Self::make_unpack_sig();
        let import_sig = Self::make_import_sig();
        let import_from_sig = Self::make_import_from_sig();
        let unpack_ex_sig = Self::make_unpack_ex_sig();
        let context_sig = Self::make_context_sig();
        let add_func = module.declare_function("jit_py_add", Linkage::Import, &binop_sig).unwrap();
        let sub_func = module.declare_function("jit_py_sub", Linkage::Import, &binop_sig).unwrap();
        let mul_func = module.declare_function("jit_py_mul", Linkage::Import, &binop_sig).unwrap();
        let div_func = module.declare_function("jit_py_div", Linkage::Import, &binop_sig).unwrap();
        let floor_div_func = module.declare_function("jit_py_floor_div", Linkage::Import, &binop_sig).unwrap();
        let mod_func = module.declare_function("jit_py_mod", Linkage::Import, &binop_sig).unwrap();
        let pow_func = module.declare_function("jit_py_pow", Linkage::Import, &binop_sig).unwrap();
        let lshift_func = module.declare_function("jit_py_lshift", Linkage::Import, &binop_sig).unwrap();
        let rshift_func = module.declare_function("jit_py_rshift", Linkage::Import, &binop_sig).unwrap();
        let bit_and_func = module.declare_function("jit_py_bit_and", Linkage::Import, &binop_sig).unwrap();
        let bit_or_func = module.declare_function("jit_py_bit_or", Linkage::Import, &binop_sig).unwrap();
        let bit_xor_func = module.declare_function("jit_py_bit_xor", Linkage::Import, &binop_sig).unwrap();
        let cmp_func = module.declare_function("jit_py_compare", Linkage::Import, &cmp_sig).unwrap();
        let truthy_func = module.declare_function("jit_is_true", Linkage::Import, &truthy_sig).unwrap();
        let neg_func = module.declare_function("jit_neg", Linkage::Import, &unary_sig).unwrap();
        let not_func = module.declare_function("jit_not", Linkage::Import, &unary_sig).unwrap();
        let build_list_func = module.declare_function("jit_build_list", Linkage::Import, &binop_sig).unwrap();
        let build_tuple_func = module.declare_function("jit_build_tuple", Linkage::Import, &binop_sig).unwrap();
        let list_append_func = module.declare_function("jit_list_append", Linkage::Import, &binop_sig).unwrap();
        let contains_func = module.declare_function("jit_contains", Linkage::Import, &binop_sig).unwrap();
        let get_iter_func = module.declare_function("jit_get_iter", Linkage::Import, &unary_sig).unwrap();
        let call_func = module.declare_function("jit_call", Linkage::Import, &call_sig).unwrap();
        let load_attr_func = module.declare_function("jit_load_attr", Linkage::Import, &load_attr_sig).unwrap();
        let for_iter_func = module.declare_function("jit_for_iter", Linkage::Import, &truthy_sig).unwrap();
        let build_map_func = module.declare_function("jit_build_map", Linkage::Import, &call_sig).unwrap();
        let store_attr_func = module.declare_function("jit_store_attr", Linkage::Import, &store_attr_sig).unwrap();
        let unpack_sequence_func = module.declare_function("jit_unpack_sequence", Linkage::Import, &unpack_sig).unwrap();
        let load_name_func = module.declare_function("jit_load_name", Linkage::Import, &store_attr_sig).unwrap();
        let build_set_func = module.declare_function("jit_build_set", Linkage::Import, &binop_sig).unwrap();
        let build_string_func = module.declare_function("jit_build_string", Linkage::Import, &binop_sig).unwrap();
        let build_slice_func = module.declare_function("jit_build_slice", Linkage::Import, &binop_sig).unwrap();
        let store_subscr_func = module.declare_function("jit_store_subscr", Linkage::Import, &call_sig).unwrap();
        let is_op_func = module.declare_function("jit_is_op", Linkage::Import, &call_sig).unwrap();
        let invert_func = module.declare_function("jit_invert", Linkage::Import, &unary_sig).unwrap();
        let import_name_func = module.declare_function("jit_import_name", Linkage::Import, &import_sig).unwrap();
        let import_from_func = module.declare_function("jit_import_from", Linkage::Import, &import_from_sig).unwrap();
        let unpack_ex_func = module.declare_function("jit_unpack_ex", Linkage::Import, &unpack_ex_sig).unwrap();
        let setup_with_func = module.declare_function("jit_setup_with", Linkage::Import, &context_sig).unwrap();
        let with_exit_func = module.declare_function("jit_with_exit", Linkage::Import, &context_sig).unwrap();
        JitCompiler {
            builder_context: FunctionBuilderContext::new(),
            module,
            add_func,
            sub_func,
            mul_func,
            div_func,
            floor_div_func,
            mod_func,
            pow_func,
            lshift_func,
            rshift_func,
            bit_and_func,
            bit_or_func,
            bit_xor_func,
            cmp_func,
            truthy_func,
            neg_func,
            not_func,
            build_list_func,
            build_tuple_func,
            list_append_func,
            contains_func,
            get_iter_func,
            call_func,
            load_attr_func,
            for_iter_func,
            build_map_func,
            store_attr_func,
            unpack_sequence_func,
            load_name_func,
            build_set_func,
            build_string_func,
            build_slice_func,
            store_subscr_func,
            is_op_func,
            invert_func,
            import_name_func,
            import_from_func,
            unpack_ex_func,
            setup_with_func,
            with_exit_func,
        }
    }

    fn make_binop_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_cmp_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_truthy_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    fn make_unary_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_call_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64)); // func ptr
        s.params.push(AbiParam::new(types::I64)); // nargs
        s.params.push(AbiParam::new(types::I64)); // args array ptr
        s.params.push(AbiParam::new(types::I64)); // out ptr
        s
    }

    fn make_load_attr_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64)); // obj ptr
        s.params.push(AbiParam::new(types::I64)); // names array ptr
        s.params.push(AbiParam::new(types::I64)); // name_idx
        s.params.push(AbiParam::new(types::I64)); // out ptr
        s
    }

    fn make_store_attr_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_unpack_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    // make_import_sig: consts ptr, names_offset, name_idx, out ptr
    fn make_import_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    // make_import_from_sig: module ptr, consts ptr, names_offset, name_idx, out ptr
    fn make_import_from_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    // make_unpack_ex_sig: seq ptr, n_before, n_after, items ptr, out ptr -> i64
    fn make_unpack_ex_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    // make_context_sig: mgr ptr, out ptr
    fn make_context_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    pub fn is_enabled() -> bool { true }

    pub fn precompute_consts(code: &CodeObject) -> Vec<PyObjectRef> {
        code.consts.iter().map(|cv| match cv {
            ConstValue::None => crate::object::py_none(),
            ConstValue::Bool(b) => crate::object::py_bool(*b),
            ConstValue::Int(s) => {
                if let Ok(n) = s.parse::<i64>() { crate::object::py_int(n) }
                else { crate::object::PyObjectRef::new(crate::object::PyObject::Int(s.parse().unwrap())) }
            }
            ConstValue::Float(f) => crate::object::py_float(f.parse().unwrap_or(0.0)),
            ConstValue::String(s) => crate::object::py_str(s),
            ConstValue::Bytes(b) => crate::object::PyObjectRef::new(crate::object::PyObject::Bytes(b.clone())),
            ConstValue::Code(_) => crate::object::py_none(),
        }).collect()
    }

    /// Precompute constants AND resolve globals for JIT.
    /// Returns [consts..., globals...] so LOAD_GLOBAL can index past consts.
    pub fn precompute_with_globals(
        code: &CodeObject,
        globals: &std::collections::HashMap<String, crate::object::PyObjectRef>,
        builtins: &std::collections::HashMap<String, crate::object::PyObjectRef>,
    ) -> Vec<crate::object::PyObjectRef> {
        let mut result = Self::precompute_consts(code);
        let base = result.len();
        result.resize(base + code.names.len(), crate::object::py_none());
        for (i, name) in code.names.iter().enumerate() {
            let val = globals.get(name)
                .or_else(|| builtins.get(name))
                .cloned()
                .unwrap_or_else(crate::object::py_none);
            result[base + i] = val;
        }
        result
    }

    pub fn precompute_with_names(code: &CodeObject) -> Vec<PyObjectRef> {
        let mut result = Self::precompute_consts(code);
        for name in &code.names {
            result.push(crate::object::py_str(name));
        }
        result
    }

    /// Build a Cranelift function that implements the given bytecode.
    /// Supports straight-line code, loops (via JUMP_BACKWARD), and conditional branches.
    pub fn compile(
        &mut self,
        code: &CodeObject,
    ) -> Option<extern "C" fn(*const PyObjectRef, usize, *const PyObjectRef, *mut PyObjectRef)> {
        if !Self::is_enabled() { return None; }
        if code.vararg_name.is_some() || code.kwarg_name.is_some() || code.kwonlyarg_count > 0 || code.num_defaults > 0 { return None; }
        if code.instructions.is_empty() || code.instructions.len() > 200 {
            return None;
        }

        let supported: &[Opcode] = &[
            Opcode::LOAD_FAST, Opcode::LOAD_CONST,
            Opcode::BINARY_OP, Opcode::RETURN_VALUE,
            Opcode::STORE_FAST, Opcode::DUP_TOP,
            Opcode::POP_TOP, Opcode::COMPARE_OP,
            Opcode::POP_JUMP_IF_FALSE, Opcode::JUMP_BACKWARD,
            Opcode::LOAD_GLOBAL,
            Opcode::UNARY_NEGATIVE, Opcode::UNARY_NOT,
            Opcode::BUILD_LIST, Opcode::BUILD_TUPLE,
            Opcode::LIST_APPEND, Opcode::CONTAINS_OP,
            Opcode::CALL,
            Opcode::LOAD_ATTR,
            Opcode::GET_ITER,
            Opcode::FOR_ITER,
            Opcode::BUILD_MAP,
            Opcode::STORE_ATTR,
            Opcode::UNPACK_SEQUENCE,
            Opcode::LOAD_NAME,
            Opcode::POP_JUMP_IF_TRUE, Opcode::POP_JUMP_IF_NONE, Opcode::POP_JUMP_IF_NOT_NONE,
            Opcode::COPY, Opcode::SWAP,
            Opcode::BUILD_SET, Opcode::BUILD_SLICE, Opcode::BUILD_STRING,
            Opcode::STORE_SUBSCR, Opcode::IS_OP, Opcode::UNARY_INVERT,
            Opcode::IMPORT_NAME, Opcode::IMPORT_FROM, Opcode::UNPACK_EX,
            Opcode::SETUP_WITH, Opcode::WITH_EXIT,
        ];
        for instr in &code.instructions {
            if !supported.contains(&instr.op) { return None; }
        }

        let _consts = Self::precompute_with_names(code);

        let mut sig = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));

        let mut ctx = cranelift::codegen::Context::new();
        ctx.func.signature = sig.clone();
        let func = self.module.declare_function("jit_fn", Linkage::Local, &sig).ok()?;

        let add_func_ref = self.module.declare_func_in_func(self.add_func, &mut ctx.func);
        let sub_func_ref = self.module.declare_func_in_func(self.sub_func, &mut ctx.func);
        let mul_func_ref = self.module.declare_func_in_func(self.mul_func, &mut ctx.func);
        let div_func_ref = self.module.declare_func_in_func(self.div_func, &mut ctx.func);
        let floor_div_func_ref = self.module.declare_func_in_func(self.floor_div_func, &mut ctx.func);
        let mod_func_ref = self.module.declare_func_in_func(self.mod_func, &mut ctx.func);
        let pow_func_ref = self.module.declare_func_in_func(self.pow_func, &mut ctx.func);
        let lshift_func_ref = self.module.declare_func_in_func(self.lshift_func, &mut ctx.func);
        let rshift_func_ref = self.module.declare_func_in_func(self.rshift_func, &mut ctx.func);
        let bit_and_func_ref = self.module.declare_func_in_func(self.bit_and_func, &mut ctx.func);
        let bit_or_func_ref = self.module.declare_func_in_func(self.bit_or_func, &mut ctx.func);
        let bit_xor_func_ref = self.module.declare_func_in_func(self.bit_xor_func, &mut ctx.func);
        let cmp_func_ref = self.module.declare_func_in_func(self.cmp_func, &mut ctx.func);
        let truthy_func_ref = self.module.declare_func_in_func(self.truthy_func, &mut ctx.func);
        let neg_func_ref = self.module.declare_func_in_func(self.neg_func, &mut ctx.func);
        let not_func_ref = self.module.declare_func_in_func(self.not_func, &mut ctx.func);
        let build_list_func_ref = self.module.declare_func_in_func(self.build_list_func, &mut ctx.func);
        let build_tuple_func_ref = self.module.declare_func_in_func(self.build_tuple_func, &mut ctx.func);
        let list_append_func_ref = self.module.declare_func_in_func(self.list_append_func, &mut ctx.func);
        let contains_func_ref = self.module.declare_func_in_func(self.contains_func, &mut ctx.func);
        let get_iter_func_ref = self.module.declare_func_in_func(self.get_iter_func, &mut ctx.func);
        let call_func_ref = self.module.declare_func_in_func(self.call_func, &mut ctx.func);
        let load_attr_func_ref = self.module.declare_func_in_func(self.load_attr_func, &mut ctx.func);
        let for_iter_func_ref = self.module.declare_func_in_func(self.for_iter_func, &mut ctx.func);
        let build_map_func_ref = self.module.declare_func_in_func(self.build_map_func, &mut ctx.func);
        let store_attr_func_ref = self.module.declare_func_in_func(self.store_attr_func, &mut ctx.func);
        let unpack_sequence_func_ref = self.module.declare_func_in_func(self.unpack_sequence_func, &mut ctx.func);
        let load_name_func_ref = self.module.declare_func_in_func(self.load_name_func, &mut ctx.func);
        let build_set_func_ref = self.module.declare_func_in_func(self.build_set_func, &mut ctx.func);
        let build_string_func_ref = self.module.declare_func_in_func(self.build_string_func, &mut ctx.func);
        let build_slice_func_ref = self.module.declare_func_in_func(self.build_slice_func, &mut ctx.func);
        let store_subscr_func_ref = self.module.declare_func_in_func(self.store_subscr_func, &mut ctx.func);
        let is_op_func_ref = self.module.declare_func_in_func(self.is_op_func, &mut ctx.func);
        let invert_func_ref = self.module.declare_func_in_func(self.invert_func, &mut ctx.func);
        let import_name_func_ref = self.module.declare_func_in_func(self.import_name_func, &mut ctx.func);
        let import_from_func_ref = self.module.declare_func_in_func(self.import_from_func, &mut ctx.func);
        let unpack_ex_func_ref = self.module.declare_func_in_func(self.unpack_ex_func, &mut ctx.func);
        let setup_with_func_ref = self.module.declare_func_in_func(self.setup_with_func, &mut ctx.func);
        let with_exit_func_ref = self.module.declare_func_in_func(self.with_exit_func, &mut ctx.func);

        // Pre-scan for branch targets
        let mut targets: HashSet<usize> = HashSet::new();
        targets.insert(0);
        for (i, instr) in code.instructions.iter().enumerate() {
            match instr.op {
                Opcode::POP_JUMP_IF_FALSE => {
                    // Both the target and the fallthrough are potential block starts
                    if instr.arg as usize != i + 1 {
                        targets.insert(instr.arg as usize);
                    }
                    targets.insert(i + 1);
                }
                Opcode::JUMP_BACKWARD => {
                    let target = i.wrapping_sub(instr.arg as usize).wrapping_sub(1);
                    targets.insert(target);
                    targets.insert(i + 1);
                }
                Opcode::FOR_ITER => {
                    let target = instr.arg as usize;
                    targets.insert(target);
                    targets.insert(i + 1);
                }
                Opcode::POP_JUMP_IF_TRUE | Opcode::POP_JUMP_IF_NONE | Opcode::POP_JUMP_IF_NOT_NONE => {
                    if instr.arg as usize != i + 1 {
                        targets.insert(instr.arg as usize);
                    }
                    targets.insert(i + 1);
                }
                _ => {}
            }
        }

        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_context);

        // Create blocks for each target
        let mut block_of: HashMap<usize, Block> = HashMap::new();
        let mut sorted_targets: Vec<usize> = targets.into_iter().collect();
        sorted_targets.sort();
        for &t in &sorted_targets {
            let b = builder.create_block();
            block_of.insert(t, b);
        }

        // Map each instruction to its containing block
        let mut instr_to_block: HashMap<usize, Block> = HashMap::new();
        let mut current_block_idx = 0;
        for i in 0..code.instructions.len() {
            if block_of.contains_key(&i) {
                current_block_idx = i;
            }
            instr_to_block.insert(i, block_of[&current_block_idx]);
        }

        // Track which blocks have been entered
        let mut blocks_entered: HashSet<Block> = HashSet::new();

        // Process entry block
        let entry_block = block_of[&0];
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        blocks_entered.insert(entry_block);

        let args_ptr = builder.block_params(entry_block)[0];
        let _nargs = builder.block_params(entry_block)[1];
        let consts_ptr = builder.block_params(entry_block)[2];
        let result_ptr = builder.block_params(entry_block)[3];

        // Allocate locals array on stack
        let locals_size = (code.nlocals.max(1) * 16) as u32;
        let locals_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot, locals_size, 0,
        ));

        // Copy args to locals
        for i in 0..code.arg_count.min(code.nlocals) {
            let src = builder.ins().iadd_imm(args_ptr, (i * 16) as i64);
            let dst = builder.ins().stack_addr(types::I64, locals_slot, (i * 16) as i32);
            let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
            let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
            builder.ins().store(cranelift::codegen::ir::MemFlags::new(), lo, dst, 0);
            builder.ins().store(cranelift::codegen::ir::MemFlags::new(), hi, dst, 8);
        }

        // Evaluation stack
        let mut eval_stack: Vec<[Value; 2]> = Vec::new();

        // Generate code for each instruction
        for i in 0..code.instructions.len() {
            let block = instr_to_block[&i];

            // Switch to the correct block if not already there
            if builder.current_block() != Some(block) {
                // Switch to the new block
                builder.switch_to_block(block);
                blocks_entered.insert(block);
            }

            let instr = &code.instructions[i];
            match instr.op {
                Opcode::LOAD_FAST => {
                    let idx = instr.arg as i32;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let src = builder.ins().stack_addr(types::I64, locals_slot, idx * 16);
                    let lo = builder.ins().load(types::I64, memflags, src, 0);
                    let hi = builder.ins().load(types::I64, memflags, src, 8);
                    eval_stack.push([lo, hi]);
                }
                Opcode::LOAD_CONST => {
                    let idx = instr.arg as i32;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let src = builder.ins().iadd_imm(consts_ptr, (idx * 16) as i64);
                    let lo = builder.ins().load(types::I64, memflags, src, 0);
                    let hi = builder.ins().load(types::I64, memflags, src, 8);
                    eval_stack.push([lo, hi]);
                }
                Opcode::LOAD_GLOBAL => {
                    let name_idx = instr.arg as i32;
                    let consts_count = code.consts.len() as i64;
                    let idx = name_idx as i64 + consts_count;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let src = builder.ins().iadd_imm(consts_ptr, (idx * 16) as i64);
                    let lo = builder.ins().load(types::I64, memflags, src, 0);
                    let hi = builder.ins().load(types::I64, memflags, src, 8);
                    eval_stack.push([lo, hi]);
                }
                Opcode::BINARY_OP => {
                    let b = eval_stack.pop().unwrap();
                    let a = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();

                    let tmp_a = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_b = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));

                    let a_addr = builder.ins().stack_addr(types::I64, tmp_a, 0);
                    let b_addr = builder.ins().stack_addr(types::I64, tmp_b, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);

                    builder.ins().store(memflags, a[0], a_addr, 0);
                    builder.ins().store(memflags, a[1], a_addr, 8);
                    builder.ins().store(memflags, b[0], b_addr, 0);
                    builder.ins().store(memflags, b[1], b_addr, 8);

                    let a_tag = builder.ins().load(types::I32, memflags, a_addr, 0);
                    let b_tag = builder.ins().load(types::I32, memflags, b_addr, 0);
                    let zero = builder.ins().iconst(types::I32, 0);
                    let a_small = builder.ins().icmp(IntCC::Equal, a_tag, zero);
                    let b_small = builder.ins().icmp(IntCC::Equal, b_tag, zero);
                    let both_small = builder.ins().band(a_small, b_small);

                    let fast_block = builder.create_block();
                    let fallback_block = builder.create_block();
                    let merge_block = builder.create_block();

                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);

                    builder.ins().brif(both_small, fast_block, &[a_addr, b_addr, out_addr],
                                                fallback_block, &[a_addr, b_addr, out_addr]);

                    builder.seal_block(fast_block);
                    builder.switch_to_block(fast_block);
                    let a_fast = builder.block_params(fast_block)[0];
                    let b_fast = builder.block_params(fast_block)[1];
                    let out_fast = builder.block_params(fast_block)[2];
                    let a_val = builder.ins().load(types::I64, memflags, a_fast, 8);
                    let b_val = builder.ins().load(types::I64, memflags, b_fast, 8);
                    let result_val = match instr.arg {
                        0 => builder.ins().iadd(a_val, b_val),
                        1 => builder.ins().isub(a_val, b_val),
                        2 => builder.ins().imul(a_val, b_val),
                        7 => builder.ins().ishl(a_val, b_val),
                        8 => builder.ins().ushr(a_val, b_val),
                        9 => builder.ins().bor(a_val, b_val),
                        10 => builder.ins().band(a_val, b_val),
                        11 => builder.ins().bxor(a_val, b_val),
                        // div(3), floor_div(4), mod(5), pow(6) — use fallback
                        _ => return None,
                    };
                    if instr.arg <= 2 || instr.arg >= 7 {
                        // Bitwise ops (7-11) and add/sub/mul can use native i64 directly
                        builder.ins().store(memflags, zero, out_fast, 0);
                        builder.ins().store(memflags, result_val, out_fast, 8);
                        builder.ins().jump(merge_block, &[]);
                    } else {
                        return None;
                    }

                    builder.seal_block(fallback_block);
                    builder.switch_to_block(fallback_block);
                    let a_fall = builder.block_params(fallback_block)[0];
                    let b_fall = builder.block_params(fallback_block)[1];
                    let out_fall = builder.block_params(fallback_block)[2];
                    let func_ref = match instr.arg {
                        0 => add_func_ref,
                        1 => sub_func_ref,
                        2 => mul_func_ref,
                        3 => div_func_ref,
                        4 => floor_div_func_ref,
                        5 => mod_func_ref,
                        6 => pow_func_ref,
                        7 => lshift_func_ref,
                        8 => rshift_func_ref,
                        9 => bit_or_func_ref,
                        10 => bit_and_func_ref,
                        11 => bit_xor_func_ref,
                        _ => return None,
                    };
                    builder.ins().call(func_ref, &[a_fall, b_fall, out_fall]);
                    builder.ins().jump(merge_block, &[]);

                    builder.seal_block(merge_block);
                    builder.switch_to_block(merge_block);

                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::COMPARE_OP => {
                    let b = eval_stack.pop().unwrap();
                    let a = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();

                    let tmp_a = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_b = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));

                    let a_addr = builder.ins().stack_addr(types::I64, tmp_a, 0);
                    let b_addr = builder.ins().stack_addr(types::I64, tmp_b, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);

                    builder.ins().store(memflags, a[0], a_addr, 0);
                    builder.ins().store(memflags, a[1], a_addr, 8);
                    builder.ins().store(memflags, b[0], b_addr, 0);
                    builder.ins().store(memflags, b[1], b_addr, 8);

                    // SmallInt fast path: check if both operands are SmallInt (tag == 0)
                    let a_tag = builder.ins().load(types::I32, memflags, a_addr, 0);
                    let b_tag = builder.ins().load(types::I32, memflags, b_addr, 0);
                    let zero_tag = builder.ins().iconst(types::I32, 0);
                    let a_small = builder.ins().icmp(IntCC::Equal, a_tag, zero_tag);
                    let b_small = builder.ins().icmp(IntCC::Equal, b_tag, zero_tag);
                    let both_small = builder.ins().band(a_small, b_small);

                    let fast_block = builder.create_block();
                    let fallback_block = builder.create_block();
                    let merge_block = builder.create_block();

                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);

                    builder.ins().brif(both_small, fast_block, &[a_addr, b_addr, out_addr],
                                                fallback_block, &[a_addr, b_addr, out_addr]);

                    // Fast path: inline comparison
                    builder.seal_block(fast_block);
                    builder.switch_to_block(fast_block);
                    let a_fast = builder.block_params(fast_block)[0];
                    let b_fast = builder.block_params(fast_block)[1];
                    let out_fast = builder.block_params(fast_block)[2];
                    let a_val = builder.ins().load(types::I64, memflags, a_fast, 8);
                    let b_val = builder.ins().load(types::I64, memflags, b_fast, 8);
                    let cmp_val = match instr.arg {
                        0 => builder.ins().icmp(IntCC::Equal, a_val, b_val),
                        1 => builder.ins().icmp(IntCC::NotEqual, a_val, b_val),
                        2 => builder.ins().icmp(IntCC::SignedLessThan, a_val, b_val),
                        3 => builder.ins().icmp(IntCC::SignedLessThanOrEqual, a_val, b_val),
                        4 => builder.ins().icmp(IntCC::SignedGreaterThan, a_val, b_val),
                        5 => builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, a_val, b_val),
                        _ => return None,
                    };
                    let cmp_i64 = builder.ins().uextend(types::I64, cmp_val);
                    builder.ins().store(memflags, zero_tag, out_fast, 0);
                    builder.ins().store(memflags, cmp_i64, out_fast, 8);
                    builder.ins().jump(merge_block, &[]);

                    // Fallback: call cmp_func_ref
                    builder.seal_block(fallback_block);
                    builder.switch_to_block(fallback_block);
                    let a_fall = builder.block_params(fallback_block)[0];
                    let b_fall = builder.block_params(fallback_block)[1];
                    let out_fall = builder.block_params(fallback_block)[2];
                    let op_val = builder.ins().iconst(types::I64, instr.arg as i64);
                    builder.ins().call(cmp_func_ref, &[a_fall, b_fall, op_val, out_fall]);
                    builder.ins().jump(merge_block, &[]);

                    builder.seal_block(merge_block);
                    builder.switch_to_block(merge_block);

                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::STORE_FAST => {
                    let idx = instr.arg as i32;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let val = eval_stack.pop().unwrap();
                    let dst = builder.ins().stack_addr(types::I64, locals_slot, idx * 16);
                    builder.ins().store(memflags, val[0], dst, 0);
                    builder.ins().store(memflags, val[1], dst, 8);
                }
                Opcode::DUP_TOP => {
                    let val = eval_stack.last().unwrap();
                    eval_stack.push([val[0], val[1]]);
                }
                Opcode::POP_TOP => {
                    eval_stack.pop();
                }
                Opcode::POP_JUMP_IF_FALSE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();

                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));

                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);

                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    let truthy_inst = builder.ins().call(truthy_func_ref, &[val_addr]);
                    let truthy = builder.inst_results(truthy_inst)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::Equal, truthy, zero);

                    let target = instr.arg as usize;
                    let target_block = block_of[&target];
                    let next_block = block_of[&(i + 1)];

                    builder.ins().brif(cmp, target_block, &[], next_block, &[]);
                }
                Opcode::JUMP_BACKWARD => {
                    let target = i.wrapping_sub(instr.arg as usize).wrapping_sub(1);
                    let target_block = block_of[&target];
                    builder.ins().jump(target_block, &[]);
                }
                Opcode::UNARY_NEGATIVE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);

                    // SmallInt fast path: check if operand is SmallInt (tag == 0)
                    let v_tag = builder.ins().load(types::I32, memflags, val_addr, 0);
                    let zero_tag = builder.ins().iconst(types::I32, 0);
                    let is_small = builder.ins().icmp(IntCC::Equal, v_tag, zero_tag);

                    let fast_block = builder.create_block();
                    let fallback_block = builder.create_block();
                    let merge_block = builder.create_block();

                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fast_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);
                    builder.append_block_param(fallback_block, types::I64);

                    builder.ins().brif(is_small, fast_block, &[val_addr, out_addr],
                                                fallback_block, &[val_addr, out_addr]);

                    // Fast path: inline negation
                    builder.seal_block(fast_block);
                    builder.switch_to_block(fast_block);
                    let v_fast = builder.block_params(fast_block)[0];
                    let out_fast = builder.block_params(fast_block)[1];
                    let v_val = builder.ins().load(types::I64, memflags, v_fast, 8);
                    let neg_val = builder.ins().irsub_imm(v_val, 0);
                    builder.ins().store(memflags, zero_tag, out_fast, 0);
                    builder.ins().store(memflags, neg_val, out_fast, 8);
                    builder.ins().jump(merge_block, &[]);

                    // Fallback: call neg_func_ref
                    builder.seal_block(fallback_block);
                    builder.switch_to_block(fallback_block);
                    let v_fall = builder.block_params(fallback_block)[0];
                    let out_fall = builder.block_params(fallback_block)[1];
                    builder.ins().call(neg_func_ref, &[v_fall, out_fall]);
                    builder.ins().jump(merge_block, &[]);

                    builder.seal_block(merge_block);
                    builder.switch_to_block(merge_block);

                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::UNARY_NOT => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().call(not_func_ref, &[val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::BUILD_LIST => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 2]> = Vec::with_capacity(n);
                    for _ in 0..n { items.push(eval_stack.pop().unwrap()); }
                    items.reverse();
                    let array_size = ((n * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                    }
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder.ins().call(build_list_func_ref, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::BUILD_TUPLE => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 2]> = Vec::with_capacity(n);
                    for _ in 0..n { items.push(eval_stack.pop().unwrap()); }
                    items.reverse();
                    let array_size = ((n * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                    }
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder.ins().call(build_tuple_func_ref, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::LIST_APPEND => {
                    let val = eval_stack.pop().unwrap();
                    let lst = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_lst = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let lst_addr = builder.ins().stack_addr(types::I64, tmp_lst, 0);
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, lst[0], lst_addr, 0);
                    builder.ins().store(memflags, lst[1], lst_addr, 8);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().call(list_append_func_ref, &[lst_addr, val_addr, out_addr]);
                }
                Opcode::CONTAINS_OP => {
                    let b = eval_stack.pop().unwrap();
                    let a = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_a = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_b = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let a_addr = builder.ins().stack_addr(types::I64, tmp_a, 0);
                    let b_addr = builder.ins().stack_addr(types::I64, tmp_b, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, a[0], a_addr, 0);
                    builder.ins().store(memflags, a[1], a_addr, 8);
                    builder.ins().store(memflags, b[0], b_addr, 0);
                    builder.ins().store(memflags, b[1], b_addr, 8);
                    builder.ins().call(contains_func_ref, &[a_addr, b_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::GET_ITER => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().call(get_iter_func_ref, &[val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::CALL => {
                    let nargs = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut args: Vec<[Value; 2]> = Vec::with_capacity(nargs);
                    for _ in 0..nargs { args.push(eval_stack.pop().unwrap()); }
                    let func = eval_stack.pop().unwrap();
                    args.reverse();
                    let array_size = ((nargs * 16).max(16)) as u32;
                    let tmp_func = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let func_addr = builder.ins().stack_addr(types::I64, tmp_func, 0);
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, func[0], func_addr, 0);
                    builder.ins().store(memflags, func[1], func_addr, 8);
                    for (i, item) in args.iter().enumerate() {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                    }
                    let nargs_val = builder.ins().iconst(types::I64, nargs as i64);
                    builder.ins().call(call_func_ref, &[func_addr, nargs_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::LOAD_ATTR => {
                    let name_idx = instr.arg as i64;
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let names_offset = code.consts.len() as i64;
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    let names_ptr = builder.ins().iadd_imm(consts_ptr, names_offset * 16);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(load_attr_func_ref, &[val_addr, names_ptr, name_idx_val, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::RETURN_VALUE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    builder.ins().store(memflags, val[0], result_ptr, 0);
                    builder.ins().store(memflags, val[1], result_ptr, 8);
                    builder.ins().return_(&[]);
                }
                Opcode::FOR_ITER => {
                    let iter = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_iter = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let iter_addr = builder.ins().stack_addr(types::I64, tmp_iter, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, iter[0], iter_addr, 0);
                    builder.ins().store(memflags, iter[1], iter_addr, 8);
                    let iter_result = builder.ins().call(for_iter_func_ref, &[iter_addr, out_addr]);
                    let status = builder.inst_results(iter_result)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let has_value = builder.ins().icmp(IntCC::Equal, status, zero);
                    let target = instr.arg as usize;
                    let target_block = block_of[&target];
                    let next_block = block_of[&(i + 1)];
                    builder.ins().brif(has_value, next_block, &[], target_block, &[]);
                    // Load result — always valid in the reachable block
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::BUILD_MAP => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 2]> = Vec::with_capacity(n * 2);
                    for _ in 0..n * 2 { items.push(eval_stack.pop().unwrap()); }
                    items.reverse();
                    let array_size = ((n * 2 * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                    }
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder.ins().call(build_map_func_ref, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::STORE_ATTR => {
                    let name_idx = instr.arg as i64;
                    let val = eval_stack.pop().unwrap();
                    let obj = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let names_offset = code.consts.len() as i64;
                    let tmp_obj = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let obj_addr = builder.ins().stack_addr(types::I64, tmp_obj, 0);
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, obj[0], obj_addr, 0);
                    builder.ins().store(memflags, obj[1], obj_addr, 8);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    let names_ptr = builder.ins().iadd_imm(consts_ptr, names_offset * 16);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(store_attr_func_ref, &[obj_addr, names_ptr, name_idx_val, val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::UNPACK_SEQUENCE => {
                    let n = instr.arg as usize;
                    let seq = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_seq = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_size = ((n * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let seq_addr = builder.ins().stack_addr(types::I64, tmp_seq, 0);
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, seq[0], seq_addr, 0);
                    builder.ins().store(memflags, seq[1], seq_addr, 8);
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder.ins().call(unpack_sequence_func_ref, &[seq_addr, n_val, array_addr, out_addr]);
                    // Push unpacked items onto stack in order
                    for i in 0..n {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        let ilo = builder.ins().load(types::I64, memflags, item_addr, 0);
                        let ihi = builder.ins().load(types::I64, memflags, item_addr, 8);
                        eval_stack.push([ilo, ihi]);
                    }
                }
                Opcode::LOAD_NAME => {
                    let name_idx = instr.arg as i64;
                    let names_offset = code.consts.len() as i64;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_locals = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_globals = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    // For simplicity, use the result_ptr as the "globals" reference
                    // In a real JIT we'd need the actual globals dict
                    // Store locals array as a dict-like proxy  
                    let locals_addr = builder.ins().stack_addr(types::I64, tmp_locals, 0);
                    let globals_addr = builder.ins().stack_addr(types::I64, tmp_globals, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    // Use consts_ptr as globals proxy (precomputed_with_globals stores globals after consts)
                    // For simplicity, write the consts_ptr into both locals and globals slots
                    builder.ins().store(memflags, consts_ptr, locals_addr, 0);
                    builder.ins().store(memflags, consts_ptr, locals_addr, 8);
                    builder.ins().store(memflags, consts_ptr, globals_addr, 0);
                    builder.ins().store(memflags, consts_ptr, globals_addr, 8);
                    let names_ptr = builder.ins().iadd_imm(consts_ptr, names_offset * 16);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(load_name_func_ref, &[names_ptr, name_idx_val, locals_addr, globals_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::COPY => {
                    let depth = instr.arg as usize;
                    let idx = eval_stack.len() - 1 - depth;
                    let val = eval_stack[idx];
                    eval_stack.push([val[0], val[1]]);
                }
                Opcode::SWAP => {
                    let idx = eval_stack.len() - 1;
                    let idx2 = idx - instr.arg as usize;
                    eval_stack.swap(idx, idx2);
                }
                Opcode::POP_JUMP_IF_TRUE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    let truthy_inst = builder.ins().call(truthy_func_ref, &[val_addr]);
                    let truthy = builder.inst_results(truthy_inst)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::NotEqual, truthy, zero);
                    let target = instr.arg as usize;
                    let target_block = block_of[&target];
                    let next_block = block_of[&(i + 1)];
                    builder.ins().brif(cmp, target_block, &[], next_block, &[]);
                }
                Opcode::POP_JUMP_IF_NONE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    let truthy_inst = builder.ins().call(truthy_func_ref, &[val_addr]);
                    let truthy = builder.inst_results(truthy_inst)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let is_none = builder.ins().icmp(IntCC::Equal, truthy, zero);
                    let target = instr.arg as usize;
                    let target_block = block_of[&target];
                    let next_block = block_of[&(i + 1)];
                    builder.ins().brif(is_none, target_block, &[], next_block, &[]);
                }
                Opcode::POP_JUMP_IF_NOT_NONE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    let truthy_inst = builder.ins().call(truthy_func_ref, &[val_addr]);
                    let truthy = builder.inst_results(truthy_inst)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let is_not_none = builder.ins().icmp(IntCC::NotEqual, truthy, zero);
                    let target = instr.arg as usize;
                    let target_block = block_of[&target];
                    let next_block = block_of[&(i + 1)];
                    builder.ins().brif(is_not_none, target_block, &[], next_block, &[]);
                }
                Opcode::BUILD_SET => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 2]> = Vec::with_capacity(n);
                    for _ in 0..n { items.push(eval_stack.pop().unwrap()); }
                    items.reverse();
                    let array_size = ((n * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                    }
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder.ins().call(build_set_func_ref, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::BUILD_STRING => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 2]> = Vec::with_capacity(n);
                    for _ in 0..n { items.push(eval_stack.pop().unwrap()); }
                    items.reverse();
                    let array_size = ((n * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                    }
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder.ins().call(build_string_func_ref, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::BUILD_SLICE => {
                    let nargs = instr.arg as usize;
                    if nargs < 2 || nargs > 3 { return None; }
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 2]> = Vec::with_capacity(3);
                    if nargs >= 3 { items.push(eval_stack.pop().unwrap()); }
                    items.push(eval_stack.pop().unwrap());
                    items.push(eval_stack.pop().unwrap());
                    // items now has [start, stop, step_or_none] but we need [start, stop, step]
                    // items were pushed: start (3rd pop), stop (2nd pop), [step (1st pop if nargs==3)]
                    items.reverse();
                    let array_size = ((3 * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                    }
                    let n_val = builder.ins().iconst(types::I64, nargs as i64);
                    builder.ins().call(build_slice_func_ref, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::STORE_SUBSCR => {
                    let val = eval_stack.pop().unwrap();
                    let idx = eval_stack.pop().unwrap();
                    let obj = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_obj = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_idx = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let obj_addr = builder.ins().stack_addr(types::I64, tmp_obj, 0);
                    let idx_addr = builder.ins().stack_addr(types::I64, tmp_idx, 0);
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, obj[0], obj_addr, 0);
                    builder.ins().store(memflags, obj[1], obj_addr, 8);
                    builder.ins().store(memflags, idx[0], idx_addr, 0);
                    builder.ins().store(memflags, idx[1], idx_addr, 8);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().call(store_subscr_func_ref, &[obj_addr, idx_addr, val_addr, out_addr]);
                }
                Opcode::IS_OP => {
                    let invert = instr.arg as i64;
                    let b = eval_stack.pop().unwrap();
                    let a = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_a = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_b = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let a_addr = builder.ins().stack_addr(types::I64, tmp_a, 0);
                    let b_addr = builder.ins().stack_addr(types::I64, tmp_b, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, a[0], a_addr, 0);
                    builder.ins().store(memflags, a[1], a_addr, 8);
                    builder.ins().store(memflags, b[0], b_addr, 0);
                    builder.ins().store(memflags, b[1], b_addr, 8);
                    let invert_val = builder.ins().iconst(types::I64, invert);
                    builder.ins().call(is_op_func_ref, &[a_addr, b_addr, invert_val, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::UNARY_INVERT => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().call(invert_func_ref, &[val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::IMPORT_NAME => {
                    let name_idx = instr.arg as i64;
                    let names_offset = code.consts.len() as i64;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    let names_offset_val = builder.ins().iconst(types::I64, names_offset);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(import_name_func_ref, &[consts_ptr, names_offset_val, name_idx_val, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::IMPORT_FROM => {
                    let name_idx = instr.arg as i64;
                    let names_offset = code.consts.len() as i64;
                    let module = *eval_stack.last().unwrap(); // peek, don't pop
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_module = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let module_addr = builder.ins().stack_addr(types::I64, tmp_module, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, module[0], module_addr, 0);
                    builder.ins().store(memflags, module[1], module_addr, 8);
                    let names_offset_val = builder.ins().iconst(types::I64, names_offset);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(import_from_func_ref, &[module_addr, consts_ptr, names_offset_val, name_idx_val, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::UNPACK_EX => {
                    let n_before = instr.arg & 0xFF;
                    let n_after = (instr.arg >> 8) & 0xFF;
                    let n_total = n_before + 1 + n_after; // before + starred + after
                    let seq = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_seq = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let array_size = ((n_total as usize * 16).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, array_size, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let seq_addr = builder.ins().stack_addr(types::I64, tmp_seq, 0);
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, seq[0], seq_addr, 0);
                    builder.ins().store(memflags, seq[1], seq_addr, 8);
                    let n_before_val = builder.ins().iconst(types::I64, n_before as i64);
                    let n_after_val = builder.ins().iconst(types::I64, n_after as i64);
                    builder.ins().call(unpack_ex_func_ref, &[seq_addr, n_before_val, n_after_val, array_addr, out_addr]);
                    // Push unpacked items onto stack: items before *, starred list, items after *
                    for i in 0..n_total {
                        let offset = (i as i32 * 16) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        let ilo = builder.ins().load(types::I64, memflags, item_addr, 0);
                        let ihi = builder.ins().load(types::I64, memflags, item_addr, 8);
                        eval_stack.push([ilo, ihi]);
                    }
                }
                Opcode::SETUP_WITH => {
                    let mgr = eval_stack.last().unwrap(); // peek, don't pop
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_mgr = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let mgr_addr = builder.ins().stack_addr(types::I64, tmp_mgr, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, mgr[0], mgr_addr, 0);
                    builder.ins().store(memflags, mgr[1], mgr_addr, 8);
                    builder.ins().call(setup_with_func_ref, &[mgr_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                Opcode::WITH_EXIT => {
                    let mgr = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_mgr = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot, 16, 0,
                    ));
                    let mgr_addr = builder.ins().stack_addr(types::I64, tmp_mgr, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, mgr[0], mgr_addr, 0);
                    builder.ins().store(memflags, mgr[1], mgr_addr, 8);
                    builder.ins().call(with_exit_func_ref, &[mgr_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    eval_stack.push([res_lo, res_hi]);
                }
                _ => return None,
            }
        }

        // Seal remaining unsealed blocks
        for &idx in &sorted_targets {
            let block = block_of[&idx];
            if blocks_entered.contains(&block) {
                builder.seal_block(block);
            }
        }

        builder.seal_all_blocks();

        builder.finalize();
        self.module.define_function(func, &mut ctx).ok()?;
        self.module.finalize_definitions().ok()?;
        let code_ptr = self.module.get_finalized_function(func);
        if code_ptr.is_null() { return None; }
        Some(unsafe { std::mem::transmute(code_ptr) })
    }
}
