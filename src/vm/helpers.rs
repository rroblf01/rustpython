use crate::bytecode::ConstValue;
use crate::object::{PyError, PyObject, PyObjectRef, PyResult};
use crate::vm::frame::Frame;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::rc::Rc;

/// Locate the bundled `Lib/` directory relative to the running executable
/// rather than the current working directory, so the interpreter works when
/// invoked from anywhere (not just the repo root). Walks up from the
/// executable's directory looking for a `Lib` subdirectory (covers both
/// `target/{debug,release}/rustpython` during development and a real
/// install layout), falling back to the old CWD-relative behavior only if
/// that search fails.
pub(crate) fn find_lib_dir() -> String {
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        for _ in 0..5 {
            match dir {
                Some(d) => {
                    let candidate = d.join("Lib");
                    if candidate.is_dir() {
                        return candidate.to_string_lossy().into_owned();
                    }
                    dir = d.parent().map(|p| p.to_path_buf());
                }
                None => break,
            }
        }
    }
    "./Lib".to_string()
}

/// Finds `key`'s slot in `varnames` IF it names a real formal parameter
/// (positional or keyword-only) — NOT just any local variable. `varnames`
/// (CPython's `co_varnames` layout) holds positional params, then kwonly
/// params, then `*args`/`**kwargs` names, then EVERY OTHER plain local the
/// function body ever assigns — a naive `varnames.iter().position(...)`
/// scan over the whole list (the bug this replaced) meant a keyword
/// argument whose name happened to match some unrelated local variable used
/// later in the function body (e.g. `def f(**kwargs): dest =
/// kwargs.pop('dest', None)` called as `f(dest=...)`) got silently
/// misrouted into that local's fast-locals slot instead of `**kwargs`,
/// making it vanish from `kwargs` entirely.
pub(crate) fn formal_param_index(
    varnames: &[crate::interner::StrId],
    arg_count: usize,
    _posonlyarg_count: usize,
    kwonlyarg_count: usize,
    kwonly_start: usize,
    key: &str,
) -> Option<usize> {
    let key_id = crate::interner::intern(key);
    if let Some(idx) = varnames
        .get(0..arg_count)
        .and_then(|s| s.iter().position(|&n| n == key_id))
    {
        return Some(idx);
    }
    if kwonlyarg_count > 0 {
        let end = kwonly_start + kwonlyarg_count;
        if let Some(rel) = varnames
            .get(kwonly_start..end)
            .and_then(|s| s.iter().position(|&n| n == key_id))
        {
            return Some(kwonly_start + rel);
        }
    }
    None
}

/// Parses a single `ConstValue` (the compiler's own, still-textual constant
/// representation — e.g. `ConstValue::Int(String)` holds the LITERAL SOURCE
/// TEXT of an int literal, not a pre-parsed number) into the real
/// `PyObjectRef` a `LOAD_CONST` of it should push. Factored out of
/// `LOAD_CONST`'s own opcode handler so its result can be cached on the
/// `CodeObject` (see `CodeObject::const_cache`'s doc comment) — this
/// function itself is unaware of caching, it's just the (moderately
/// expensive, for `Int`/`Float`/`Complex`) one-time parse.
pub(crate) fn unbound_local_msg(f: &Frame, idx: usize) -> PyError {
    let name = crate::interner::lookup_str(
        f.code.varnames.get(idx).copied().unwrap_or(crate::interner::intern("?")),
    );
    PyError::unbound_local_error(format!(
        "cannot access local variable '{name}' where it is not associated with a value"
    ))
}

pub(crate) fn deref_proxy(obj: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let PyObject::WeakProxy { target, .. } = &*obj.borrow() {
        if let Some(rc) = target.upgrade() {
            Ok(PyObjectRef::Imm(rc))
        } else {
            Err(PyError::reference_error("weakly-referenced object no longer exists"))
        }
    } else {
        Ok(obj.clone())
    }
}

/// In-place (`arg >= 100`) BINARY_OP semantics shared by the opcode handler
/// and fused superinstructions. Returns `Ok(None)` when the operation has no
/// dedicated in-place form and must fall through to `plain_binary_op`.
pub(crate) fn inplace_binary_op(
    left: &PyObjectRef,
    right: &PyObjectRef,
    op: u32,
) -> PyResult<Option<PyObjectRef>> {
    let left = deref_proxy(left)?;
    let right = deref_proxy(right)?;
                let right = right.clone();
                let in_place = true;
                if in_place {
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
                        if matches!(&*left.borrow(), PyObject::Instance { .. }) {
                            if let Some(r) = crate::object::try_dunder_binop(&left, &right, name)? {
                                return Ok(Some(r));
                            }
                        }
                    }
                }
                // Native `deque` has no real Python-callable `__iadd__`/
                // `__imul__` dunder in its type dict (native methods are
                // dispatched via `attrs.rs`'s `get_attribute_impl`, which
                // doesn't fire for operator opcodes) — so `d += 'bcd'` /
                // `d *= 3` on a raw deque would otherwise fall through to
                // `py_add`/`py_mul` below, which are correct for `d + e`/
                // `d * n` (both build a NEW deque) but wrong for the
                // in-place forms (`d += 'bcd'` must EXTEND the live deque
                // even though `d + 'bcd'` raises TypeError). Handle the
                // in-place forms directly here.
                if in_place {
                    let is_list = matches!(&*left.borrow(), PyObject::List(_));
                    if is_list {
                        match op {
                            // `l += iterable` — extend in place (CPython's
                            // list.__iadd__); `u2 = u; u += [2,3]` must keep
                            // `u is u2` (test_list::test_iadd).
                            // For `list += UserList` (or any UserList subclass), CPython's
                            // list.__iadd__ returns NotImplemented so the reflected
                            // UserList.__radd__ (which returns a UserList) wins and
                            // the result becomes a UserList, not a list (test_userlist::
                            // test_mixed_iadd). Detect that case and fall through to
                            // plain_binary_op (which tries __radd__) instead of
                            // unconditionally extending the list.
                            0 => {
                                let is_instance_rhs = matches!(&*right.borrow(), PyObject::Instance { .. });
                                if is_instance_rhs {
                                    // Check if RHS is a UserList (or subclass) via its type name/MRO.
                                    let is_userlist = if let PyObject::Instance { typ, .. } = &*right.borrow() {
                                        let tb = typ.borrow();
                                        if let PyObject::Type { mro, name, .. } = &*tb {
                                            if name == "UserList" {
                                                true
                                            } else {
                                                mro.iter().any(|base| {
                                                    if let PyObject::Type { name, .. } = &*base.borrow() {
                                                        name == "UserList"
                                                    } else {
                                                        false
                                                    }
                                                })
                                            }
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    };
                                    if is_userlist {
                                        return Ok(None);
                                    }
                                }
                                let it = crate::object::builtin_iter(&[right])?;
                                let mut items = Vec::new();
                                loop {
                                    match crate::object::builtin_next(&[it.clone()]) {
                                        Ok(v) => items.push(v),
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                                if let PyObject::List(list) = &mut *left.borrow_mut() {
                                    list.extend(items);
                                }
                                return Ok(Some(left.clone()));
                            }
                            // `l *= n` — repeat in place (list.__imul__).
                            2 => {
                                let n = crate::object::to_index(&right)
                                    .map(|n| n.to_i64().unwrap_or(0).max(0))
                                    .unwrap_or(0) as usize;
                                if let PyObject::List(list) = &mut *left.borrow_mut() {
                                    let items: Vec<crate::object::PyObjectRef> = list.clone();
                                    // Fail fast on overflow like list_resize
                                    // (`[0] *= sys.maxsize` -> MemoryError).
                                    let mut reserve: Vec<crate::object::PyObjectRef> = Vec::new();
                                    match items.len().checked_mul(n) {
                                        Some(total) if reserve.try_reserve_exact(total).is_ok() => {
                                            list.clear();
                                            for _ in 0..n {
                                                list.extend(items.clone());
                                            }
                                        }
                                        _ => {
                                            return Err(PyError::memory_error(
                                                "could not allocate list",
                                            ))
                                        }
                                    }
                                }
                                return Ok(Some(left.clone()));
                            }
                            _ => {}
                        }
                    }
                    let is_deque = matches!(&*left.borrow(), PyObject::Deque { .. });
                    if is_deque {
                        match op {
                            // `d += iterable` — extend in place (real
                            // CPython's `deque.__iadd__`), accepts any
                            // iterable. Materialize the source FIRST so
                            // self-extend (`d += d`) doesn't trip the deque
                            // iterator's own mutation detection mid-iteration.
                            0 => {
                                let it = crate::object::builtin_iter(&[right])?;
                                let mut items = Vec::new();
                                loop {
                                    match crate::object::builtin_next(&[it.clone()]) {
                                        Ok(v) => items.push(v),
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                                {
                                    if let PyObject::Deque { data, maxlen } =
                                        &mut *left.borrow_mut()
                                    {
                                        for item in items {
                                            data.push_back(item);
                                            if let Some(maxlen) = maxlen {
                                                while data.len() > *maxlen {
                                                    data.pop_front();
                                                }
                                            }
                                        }
                                    }
                                }
                                return Ok(Some(left.clone()));
                            }
                            // `d *= n` — repeat in place, truncated to maxlen.
                            2 => {
                                let n = right
                                    .as_i64()
                                    .ok_or_else(|| PyError::type_error("an integer is required"))?;
                                if let PyObject::Deque { data, maxlen } = &mut *left.borrow_mut() {
                                    let n = n.max(0) as usize;
                                    let items: Vec<crate::object::PyObjectRef> =
                                        data.iter().cloned().collect();
                                    data.clear();
                                    for _ in 0..n {
                                        for item in &items {
                                            data.push_back(item.clone());
                                            if let Some(maxlen) = maxlen {
                                                while data.len() > *maxlen {
                                                    data.pop_front();
                                                }
                                            }
                                        }
                                    }
                                }
                                return Ok(Some(left.clone()));
                            }
                            _ => {}
                        }
                    }
                }
    Ok(None)
}

/// The non-inplace BINARY_OP semantics shared by the `BINARY_OP` opcode
/// handler and the fused SUPER_*_BIN superinstructions.
pub(crate) fn plain_binary_op(
    left: &PyObjectRef,
    right: &PyObjectRef,
    op: u32,
) -> PyResult<PyObjectRef> {
    let left_d = deref_proxy(left)?;
    let right_d = deref_proxy(right)?;
    Ok(match op {
        0 => crate::object::py_add(&left_d, &right_d)?,
        1 => crate::object::py_sub(&left_d, &right_d)?,
        2 => crate::object::py_mul(&left_d, &right_d)?,
        3 => crate::object::py_div(&left_d, &right_d)?,
        4 => crate::object::py_floor_div(&left_d, &right_d)?,
        5 => crate::object::py_mod(&left_d, &right_d)?,
        6 => crate::object::py_pow(&left_d, &right_d)?,
        7 => crate::object::py_lshift(&left_d, &right_d)?,
        8 => crate::object::py_rshift(&left_d, &right_d)?,
        9 => crate::object::py_bit_or(&left_d, &right_d)?,
        10 => crate::object::py_bit_xor(&left_d, &right_d)?,
        11 => crate::object::py_bit_and(&left_d, &right_d)?,
        12 => {
            if let Some(r) = crate::object::try_dunder_binop(&left_d, &right_d, "__matmul__")? {
                r
            } else if let Some(r) = crate::object::try_dunder_binop(&right_d, &left_d, "__rmatmul__")? {
                r
            } else {
                return Err(PyError::type_error(format!(
                    "unsupported operand type(s) for @: '{}' and '{}'",
                    left_d.borrow().type_name(),
                    right_d.borrow().type_name()
                )));
            }
        }
        13 => crate::object::py_getitem(&left_d, &right_d)?,
        _ => return Err(PyError::runtime_error(format!("unknown binary op: {}", op))),
    })
}

pub(crate) fn eval_const_value(const_val: ConstValue) -> PyResult<PyObjectRef> {
    use crate::object::{py_bool, py_float, py_int, py_none, py_str};
    Ok(match const_val {
        ConstValue::None => py_none(),
        ConstValue::Bool(b) => py_bool(b),
        ConstValue::Int(s) => {
            // Strips ALL underscores (digit separators, e.g. `1_000_000`),
            // not just leading ones — `try_exec_simple`'s OWN independent
            // copy of this same parsing logic used `s.trim_start_matches
            // ('_')` instead (fixed to match, in the same pass as adding
            // its own const-cache use, since both copies must agree).
            let s_clean: String = s.chars().filter(|&c| c != '_').collect();
            if let Some(oct) = s_clean
                .strip_prefix("0o")
                .or_else(|| s_clean.strip_prefix("0O"))
            {
                if let Ok(n) = i64::from_str_radix(oct, 8) {
                    py_int(n)
                } else {
                    let n = BigInt::parse_bytes(oct.as_bytes(), 8)
                        .ok_or_else(|| PyError::value_error(format!("invalid integer: {}", s)))?;
                    PyObjectRef::imm(PyObject::Int(n))
                }
            } else if let Some(hex) = s_clean
                .strip_prefix("0x")
                .or_else(|| s_clean.strip_prefix("0X"))
            {
                if let Ok(n) = i64::from_str_radix(hex, 16) {
                    py_int(n)
                } else {
                    let n = BigInt::parse_bytes(hex.as_bytes(), 16)
                        .ok_or_else(|| PyError::value_error(format!("invalid integer: {}", s)))?;
                    PyObjectRef::imm(PyObject::Int(n))
                }
            } else if let Some(bin) = s_clean
                .strip_prefix("0b")
                .or_else(|| s_clean.strip_prefix("0B"))
            {
                if let Ok(n) = i64::from_str_radix(bin, 2) {
                    py_int(n)
                } else {
                    let n = BigInt::parse_bytes(bin.as_bytes(), 2)
                        .ok_or_else(|| PyError::value_error(format!("invalid integer: {}", s)))?;
                    PyObjectRef::imm(PyObject::Int(n))
                }
            } else if let Ok(n) = s_clean.parse::<i64>() {
                py_int(n) // uses small int cache
            } else {
                let n: BigInt = s_clean
                    .parse()
                    .map_err(|_| PyError::value_error(format!("invalid integer: {}", s)))?;
                PyObjectRef::imm(PyObject::Int(n))
            }
        }
        ConstValue::Float(s) => {
            let s_clean: String = crate::object::validate_underscores(&s)?
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let f: f64 = s_clean
                .parse()
                .map_err(|_| PyError::value_error(format!("invalid float: {}", s)))?;
            py_float(f)
        }
        ConstValue::String(s) => py_str(&s),
        ConstValue::Bytes(b) => PyObjectRef::imm(PyObject::Bytes(b)),
        ConstValue::Complex { real, imag } => {
            let re: f64 = real
                .parse()
                .map_err(|_| PyError::value_error(format!("invalid complex literal: {}", real)))?;
            let im: f64 = imag
                .parse()
                .map_err(|_| PyError::value_error(format!("invalid complex literal: {}", imag)))?;
            PyObjectRef::imm(PyObject::Complex(re, im))
        }
        ConstValue::Code(code) => PyObjectRef::imm(PyObject::Code(Rc::from(code))),
        ConstValue::Tuple(items) => {
            let objs: Vec<PyObjectRef> = items.into_iter().map(|s| py_str(&s)).collect();
            PyObjectRef::imm(PyObject::Tuple(objs))
        }
    })
}
