use crate::object::PyObjectRef;
// The `extern "C" fn jit_*` functions below are called directly from
// Cranelift-compiled machine code (see the CALL-with-known-target codegen
// further down in this file) whenever an operation falls outside the
// JIT's inline fast paths. SAFETY, shared by all of them: the JIT codegen
// that emits calls to these functions always passes the address of a live
// stack slot holding a valid `PyObjectRef` for every `*const`/`*mut
// PyObjectRef` parameter — that invariant is enforced by construction in
// this file's codegen, not by the callee, so it isn't re-derived per function.
pub(crate) extern "C" fn jit_py_add(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if let Some(sum) = av.checked_add(*bv) {
                std::ptr::write(out, PyObjectRef::SmallInt(sum));
                return;
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_add(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_sub(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if let Some(diff) = av.checked_sub(*bv) {
                std::ptr::write(out, PyObjectRef::SmallInt(diff));
                return;
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_sub(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_mul(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if let Some(prod) = av.checked_mul(*bv) {
                std::ptr::write(out, PyObjectRef::SmallInt(prod));
                return;
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_mul(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_div(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            // Integer division returns float, so only fast-path if exact
            if *bv != 0 && av % bv == 0 {
                std::ptr::write(out, PyObjectRef::SmallFloat(*av as f64 / *bv as f64));
                return;
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_div(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_floor_div(
    a: *const PyObjectRef,
    b: *const PyObjectRef,
    out: *mut PyObjectRef,
) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if let Some(q) = av.checked_div(*bv) {
                // Rust's / on i64 truncates toward zero; Python floor-div truncates toward -inf.
                // Adjust when signs differ and there's a remainder.
                let adjusted = if (av ^ bv) < 0 && av % *bv != 0 {
                    q - 1
                } else {
                    q
                };
                std::ptr::write(out, PyObjectRef::SmallInt(adjusted));
                return;
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_floor_div(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_mod(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if *bv != 0 {
                // Python modulus: result has sign of divisor (bv)
                let r = av % *bv;
                let adjusted = if (r ^ bv) >= 0 { r } else { r + *bv };
                std::ptr::write(out, PyObjectRef::SmallInt(adjusted));
                return;
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_mod(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_pow(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if *bv >= 0 && *bv <= 20 {
                if let Some(p) = av.checked_pow(*bv as u32) {
                    std::ptr::write(out, PyObjectRef::SmallInt(p));
                    return;
                }
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_pow(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_lshift(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if *bv >= 0 && *bv < 64 {
                if let Some(s) = av.checked_shl(*bv as u32) {
                    std::ptr::write(out, PyObjectRef::SmallInt(s));
                    return;
                }
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_lshift(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_rshift(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            if *bv >= 0 && *bv < 64 {
                // Rust right-shift is arithmetic (sign-extending) for i64, matching Python
                std::ptr::write(out, PyObjectRef::SmallInt(av >> *bv));
                return;
            }
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_rshift(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_bit_and(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            std::ptr::write(out, PyObjectRef::SmallInt(av & bv));
            return;
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_bit_and(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_bit_or(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            std::ptr::write(out, PyObjectRef::SmallInt(av | bv));
            return;
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_bit_or(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_bit_xor(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        if let (PyObjectRef::SmallInt(av), PyObjectRef::SmallInt(bv)) = (&*a, &*b) {
            std::ptr::write(out, PyObjectRef::SmallInt(av ^ bv));
            return;
        }
    }
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_bit_xor(&*a, &*b).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_py_compare(
    a: *const PyObjectRef,
    b: *const PyObjectRef,
    op: i64,
    out: *mut PyObjectRef,
) {
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_compare(&*a, &*b, op as u32)
                .unwrap_or_else(|_| crate::object::py_bool(false)),
        );
    }
}
/// Mirrors `vm.rs`'s `BINARY_OP` handler for `arg >= 100` (the in-place
/// variant emitted by `AugAssign`, e.g. `x += y`): try `__iadd__`/`__isub__`/
/// etc. first, but ONLY when `left` is a `PyObject::Instance` (a native
/// type falling into this helper has no such dunder, and the interpreter
/// itself never checks for one on natives either — see vm.rs's own
/// `matches!(&*left.borrow(), PyObject::Instance { .. })` guard), then fall
/// back to the exact same op dispatch as the plain (non-augmented) operator.
/// `op` uses the same 0..=12 encoding as `BINARY_OP`'s non-in-place arg.
pub(crate) extern "C" fn jit_py_inplace_binop(
    a: *const PyObjectRef,
    b: *const PyObjectRef,
    op: i64,
    out: *mut PyObjectRef,
) {
    unsafe {
        let left = &*a;
        let right = &*b;
        let idunder = match op {
            0 => Some("__iadd__"),
            1 => Some("__isub__"),
            2 => Some("__imul__"),
            3 => Some("__itruediv__"),
            4 => Some("__ifloordiv__"),
            5 => Some("__imod__"),
            6 => Some("__ipow__"),
            7 => Some("__ilshift__"),
            8 => Some("__irshift__"),
            9 => Some("__ior__"),
            10 => Some("__ixor__"),
            11 => Some("__iand__"),
            12 => Some("__imatmul__"),
            _ => None,
        };
        if let Some(name) = idunder {
            if matches!(&*left.borrow(), crate::object::PyObject::Instance { .. }) {
                if let Ok(Some(r)) = crate::object::try_dunder_binop(left, right, name) {
                    std::ptr::write(out, r);
                    return;
                }
            }
        }
        let result = match op {
            0 => crate::object::py_add(left, right),
            1 => crate::object::py_sub(left, right),
            2 => crate::object::py_mul(left, right),
            3 => crate::object::py_div(left, right),
            4 => crate::object::py_floor_div(left, right),
            5 => crate::object::py_mod(left, right),
            6 => crate::object::py_pow(left, right),
            7 => crate::object::py_lshift(left, right),
            8 => crate::object::py_rshift(left, right),
            9 => crate::object::py_bit_or(left, right),
            10 => crate::object::py_bit_xor(left, right),
            11 => crate::object::py_bit_and(left, right),
            12 => {
                crate::object::try_dunder_binop(left, right, "__matmul__").and_then(|r| match r {
                    Some(v) => Ok(v),
                    None => {
                        crate::object::try_dunder_binop(right, left, "__rmatmul__").and_then(|r2| {
                            r2.ok_or_else(|| {
                                crate::object::PyError::type_error(
                                    "unsupported operand type(s) for @",
                                )
                            })
                        })
                    }
                })
            }
            _ => Err(crate::object::PyError::runtime_error("unknown binary op")),
        };
        std::ptr::write(out, result.unwrap_or_else(|_| crate::object::py_none()));
    }
}
pub(crate) extern "C" fn jit_is_true(val: *const PyObjectRef) -> i64 {
    unsafe { (*val).truthy() as i64 }
}
pub(crate) extern "C" fn jit_neg(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        std::ptr::write(
            out,
            crate::object::py_neg(&*val).unwrap_or_else(|_| crate::object::py_none()),
        );
    }
}
pub(crate) extern "C" fn jit_not(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        std::ptr::write(out, crate::object::py_not(&*val));
    }
}
pub(crate) extern "C" fn jit_build_list(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut v: Vec<PyObjectRef> = Vec::with_capacity(n as usize);
        for i in 0..n as isize {
            v.push((*items.offset(i)).clone());
        }
        std::ptr::write(out, crate::object::py_list(v));
    }
}
pub(crate) extern "C" fn jit_build_tuple(n: i64, items: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let mut v: Vec<PyObjectRef> = Vec::with_capacity(n as usize);
        for i in 0..n as isize {
            v.push((*items.offset(i)).clone());
        }
        std::ptr::write(out, crate::object::py_tuple(v));
    }
}
pub(crate) extern "C" fn jit_list_append(
    lst: *const PyObjectRef,
    val: *const PyObjectRef,
    out: *mut PyObjectRef,
) {
    unsafe {
        if let crate::object::PyObject::List(v) = &mut *(*lst).borrow_mut() {
            v.push((*val).clone());
        }
        std::ptr::write(out, crate::object::py_none());
    }
}
pub(crate) extern "C" fn jit_contains(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let result =
            crate::object::py_contains(&*a, &*b).unwrap_or_else(|_| crate::object::py_bool(false));
        std::ptr::write(out, result);
    }
}
pub(crate) extern "C" fn jit_getitem(obj: *const PyObjectRef, idx: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        let result =
            crate::object::py_getitem(&*obj, &*idx).unwrap_or_else(|_| crate::object::py_none());
        std::ptr::write(out, result);
    }
}
pub(crate) extern "C" fn jit_get_iter(val: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe {
        use crate::object::ObjectAccess;
        let obj = &*val;
        let iter_method = obj.borrow().get_attribute("__iter__").ok();
        let result = if let Some(method) = iter_method {
            crate::object::call_bound_method(method, (*val).clone(), vec![])
                .unwrap_or_else(|_| (*val).clone())
        } else {
            (*val).clone()
        };
        std::ptr::write(out, result);
    }
}

pub(crate) extern "C" fn jit_call(
    func: *const PyObjectRef,
    nargs: i64,
    args: *const PyObjectRef,
    out: *mut PyObjectRef,
) {
    unsafe {
        let func_ref = &*func;
        let mut v: Vec<PyObjectRef> = Vec::with_capacity(nargs as usize);
        for i in 0..nargs as isize {
            v.push((*args.offset(i)).clone());
        }
        // Try builtins/closures first (fast path)
        if let Ok(val) = crate::object::call_function(func_ref, v.clone()) {
            std::ptr::write(out, val);
            return;
        }
        // For regular Python functions, delegate to VM via a FRESH VM — the
        // JIT region is running under the shared VM's dispatch, so
        // `with_vm_mut` reborrowing that same VM here is aliasing UB
        // (hashbrown's copy_nonoverlapping abort in test_shlex).
        let func_val = (*func).clone();
        let cb_result = crate::object::call_function_disposable(&func_val, v.clone(), Vec::new());
        if let Ok(val) = cb_result {
            std::ptr::write(out, val);
            return;
        }
        if std::env::var("RPY_DEBUG_JITCALL").is_ok() {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "JITCALL FAIL func={} nargs={}",
                func_ref.repr(),
                nargs
            );
        }
        std::ptr::write(out, crate::object::py_none());
    }
}

/// Keyword-argument-aware sibling of `jit_call` — the `CALL` opcode's `arg`
/// packs BOTH counts into one `u32` (`npos | (nkw << 8)`, matching vm.rs's
/// own `Opcode::CALL` handler exactly: `npos = arg & 0xFF`, `nkw = (arg >>
/// 8) & 0xFF`), with `items` holding `npos` positional values followed by
/// `nkw` (name, value) pairs. `jit_call`/this file's codegen previously
/// only ever read `instr.arg` directly AS `nargs` — for ANY call with at
/// least one keyword argument this treated `nkw`'s packed-in bits as
/// thousands of extra positional args to pop (real trigger: `import_helper.
/// forget()`'s own `cache_from_source(source, optimization=opt)` call
/// compiled to `CALL arg=257` — `1 | (1 << 8)` — inside a for-loop, JIT-
/// compiled because it has a loop; popped 257 stack slots instead of 2,
/// underflowing and panicking "called `Option::unwrap()` on a `None`
/// value"). Any keyword-argument call reaching a JIT-eligible function
/// was broken this way.
pub(crate) extern "C" fn jit_call_kw(
    func: *const PyObjectRef,
    npos: i64,
    nkw: i64,
    items: *const PyObjectRef,
    out: *mut PyObjectRef,
) {
    unsafe {
        let npos = npos as usize;
        let nkw = nkw as usize;
        let mut args: Vec<PyObjectRef> = Vec::with_capacity(npos);
        for i in 0..npos as isize {
            args.push((*items.offset(i)).clone());
        }
        let mut keywords: Vec<(String, PyObjectRef)> = Vec::with_capacity(nkw);
        for k in 0..nkw {
            let name_ref = &*items.offset((npos + 2 * k) as isize);
            let val_ref = (*items.offset((npos + 2 * k + 1) as isize)).clone();
            if let crate::object::PyObject::Str(name) = &*name_ref.borrow() {
                keywords.push((name.to_string(), val_ref));
            }
        }
        let func_val = (*func).clone();
        // Same "builtins/closures first, then VM callback" shape as
        // `jit_call` — but `crate::object::call_function` (the fast path)
        // has no keywords parameter at all, so any call with `nkw > 0`
        // must go straight to `vm.call_function`.
        if nkw == 0 {
            if let Ok(val) = crate::object::call_function(&func_val, args.clone()) {
                std::ptr::write(out, val);
                return;
            }
        }
        let func_val = (*func).clone();
        let cb_result = crate::object::call_function_disposable(&func_val, args, keywords);
        if let Ok(val) = cb_result {
            std::ptr::write(out, val);
            return;
        }
        std::ptr::write(out, crate::object::py_none());
    }
}
thread_local! {
    static ATTR_CACHE: std::cell::RefCell<std::collections::HashMap<(String, String), crate::object::PyObjectRef>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

pub(crate) extern "C" fn jit_load_attr(
    obj: *const PyObjectRef,
    names: *const PyObjectRef,
    name_idx: i64,
    out: *mut PyObjectRef,
) {
    unsafe {
        use crate::object::ObjectAccess;
        let name_obj = &*names.offset(name_idx as isize);
        let name_str = name_obj.str();
        let obj_ref = &*obj;
        let type_name = obj_ref.borrow().type_name();
        let cache_key = (type_name.clone(), name_str.clone());
        // Check thread-local cache first (type-aware)
        let cached = ATTR_CACHE.with(|cache| cache.borrow().get(&cache_key).cloned());
        if let Some(val) = cached {
            std::ptr::write(out, val);
            return;
        }
        let result = obj_ref
            .borrow()
            .get_attribute(&name_str)
            .unwrap_or_else(|_| crate::object::py_none());
        if std::env::var("RPY_DEBUG_JITATTR").is_ok() {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "JITATTR obj={} name={} -> {}",
                obj_ref.repr(),
                name_str,
                result.repr()
            );
        }
        // Bind the result to the actual object, mirroring the interpreter's
        // LOAD_ATTR/`resolve_descriptor_attr` — `get_attribute` returns an
        // UNBOUND template (self_obj = None) for native-type methods like
        // int.bit_length, which only works once bound to `obj`.
        let result = {
            let rb = result.borrow();
            match &*rb {
                crate::object::PyObject::BuiltinMethod { name: n, func, .. } => {
                    let name = n.clone();
                    let func = *func;
                    drop(rb);
                    crate::object::PyObjectRef::imm(crate::object::PyObject::BuiltinMethod {
                        name,
                        func,
                        self_obj: obj_ref.clone(),
                    })
                }
                crate::object::PyObject::BoundMethod { func, .. } => {
                    let func = func.clone();
                    drop(rb);
                    crate::object::PyObjectRef::imm(crate::object::PyObject::BoundMethod {
                        func,
                        self_obj: obj_ref.clone(),
                    })
                }
                // A user-defined method (raw `Function` from the type dict)
                // and a native module-level function stored directly in a
                // type's dict (`PyObject::BuiltinFunction`, e.g. `Random`'s
                // `getrandbits`) are ALSO not auto-bound by `get_attribute`
                // — only `BuiltinMethod`/`BoundMethod` were handled above.
                // Without this, any JIT-compiled loop calling a plain
                // Python method or such a native method (`self.method()`)
                // called it with `self` missing entirely (confirmed via
                // `RPY_DEBUG_JITCALL`: `getrandbits` invoked with 1 arg
                // instead of 2, `self.get()` with 0 instead of 1) —
                // matching the interpreter-side `call_method_rebound` gap
                // fixed for the same reason. Skip the same two shapes that
                // are never real bound methods (a builtin exception "class"
                // reference, `open`).
                crate::object::PyObject::Function(_) => {
                    drop(rb);
                    crate::object::PyObjectRef::new(crate::object::PyObject::BoundMethod {
                        func: result.clone(),
                        self_obj: obj_ref.clone(),
                    })
                }
                crate::object::PyObject::BuiltinFunction { name: n, func }
                    if !(crate::object::is_builtin_exception_class_name(n)
                        || std::ptr::fn_addr_eq(*func, crate::object::builtin_open as crate::object::BuiltinFunc)) =>
                {
                    drop(rb);
                    crate::object::PyObjectRef::new(crate::object::PyObject::BoundMethod {
                        func: result.clone(),
                        self_obj: obj_ref.clone(),
                    })
                }
                _ => result.clone(),
            }
        };
        // Only cache the result if it does NOT capture `self` — a bound
        // method (BuiltinMethod/BoundMethod) embeds the specific object it
        // was looked up on, so caching by (type, name) alone and reusing it
        // for a DIFFERENT object of the same type is wrong (`n.bit_length()`
        // returned a method bound to a previous `n`, calling bit_length on
        // the wrong int — the JIT loop/`while` bug). Plain values and
        // unbound class attributes are safe to cache.
        if !matches!(
            &*result.borrow(),
            crate::object::PyObject::BuiltinMethod { .. }
                | crate::object::PyObject::BoundMethod { .. }
        ) {
            ATTR_CACHE.with(|cache| {
                cache.borrow_mut().insert(cache_key, result.clone());
            });
        }
        std::ptr::write(out, result);
    }
}