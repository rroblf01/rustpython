use crate::object::*;
use std::collections::HashMap;

pub fn container_ptr(o: &PyObjectRef) -> Option<*const ()> {
    match o {
        PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => Some(std::rc::Rc::as_ptr(rc) as *const ()),
        _ => None,
    }
}

thread_local! {
    /// Class objects seen by the serializer, by simple class name. The
    /// custom pickle format is same-process only (round-trips inside one
    /// interpreter run), so a name -> type map lets the deserializer
    /// rebuild user-class instances without touching import machinery.
    pub static PICKLE_CLASS_REGISTRY: std::cell::RefCell<HashMap<String, PyObjectRef>> =
        std::cell::RefCell::new(HashMap::new());
}

pub fn pickle_serialize(
    obj: &PyObjectRef,
    buf: &mut Vec<u8>,
    memo: &mut Vec<*const ()>,
    protocol: i32,
) -> PyResult<()> {
    match &*obj.borrow() {
        PyObject::None => buf.push(b'N'),
        PyObject::Bool(true) => {
            // Protocol 0-1: True is serialized as integer 1 (I01\n)
            // Protocol 2+: NEWTRUE opcode (\x88)
            if protocol <= 1 {
                buf.push(b'I');
                buf.extend_from_slice(b"01\n");
            } else {
                buf.push(0x88); // NEWTRUE
            }
        }
        PyObject::Bool(false) => {
            if protocol <= 1 {
                buf.push(b'I');
                buf.extend_from_slice(b"00\n");
            } else {
                buf.push(0x89); // NEWFALSE
            }
        }
        PyObject::Int(n) => {
            buf.push(b'I');
            buf.extend_from_slice(n.to_string().as_bytes());
            buf.push(b'\n');
        }
        PyObject::Float(f) => {
            buf.push(b'G');
            let s = if f.is_nan() {
                "nan".to_string()
            } else if f.is_infinite() && f.is_sign_positive() {
                "inf".to_string()
            } else if f.is_infinite() {
                "-inf".to_string()
            } else {
                let s = format!("{:.17}", f);
                let s = s.trim_end_matches('0').to_string();
                if s.ends_with('.') {
                    format!("{}0", s)
                } else {
                    s
                }
            };
            buf.extend_from_slice(s.as_bytes());
            buf.push(b'\n');
        }
        PyObject::Str(s) => {
            buf.push(b'S');
            let bytes = s.as_bytes();
            buf.extend_from_slice(bytes.len().to_string().as_bytes());
            buf.push(b':');
            buf.extend_from_slice(bytes);
        }
        PyObject::Bytes(b) => {
            buf.push(b'B');
            buf.extend_from_slice(b.len().to_string().as_bytes());
            buf.push(b':');
            buf.extend_from_slice(b);
        }
        PyObject::List(items) => {
            if let Some(ptr) = container_ptr(obj) {
                if let Some(id) = memo.iter().position(|&p| p == ptr) {
                    buf.push(b'@');
                    buf.extend_from_slice(id.to_string().as_bytes());
                    buf.push(b'\n');
                    return Ok(());
                }
                memo.push(ptr);
            }
            buf.push(b'[');
            for item in items {
                pickle_serialize(item, buf, memo, protocol)?;
            }
            buf.push(b']');
        }
        PyObject::Deque { data, maxlen } => {
            if let Some(ptr) = container_ptr(obj) {
                if let Some(id) = memo.iter().position(|&p| p == ptr) {
                    buf.push(b'@');
                    buf.extend_from_slice(id.to_string().as_bytes());
                    buf.push(b'\n');
                    return Ok(());
                }
                memo.push(ptr);
            }
            buf.push(b'D');
            match maxlen {
                Some(n) => {
                    buf.push(b'M');
                    buf.extend_from_slice(n.to_string().as_bytes());
                    buf.push(b'\n');
                }
                None => buf.push(b'N'),
            }
            buf.push(b'[');
            for item in data.iter() {
                pickle_serialize(item, buf, memo, protocol)?;
            }
            buf.push(b']');
        }
        PyObject::DequeIter {
            deque,
            index,
            start_len,
        } => {
            buf.push(b'q');
            pickle_serialize(deque, buf, memo, protocol)?;
            pickle_serialize(&py_int(*index as i64), buf, memo, protocol)?;
            pickle_serialize(&py_int(*start_len as i64), buf, memo, protocol)?;
        }
        PyObject::Tuple(items) => {
            buf.push(b'(');
            for item in items {
                pickle_serialize(item, buf, memo, protocol)?;
            }
            buf.push(b')');
        }
        PyObject::Dict(d) => {
            if let Some(ptr) = container_ptr(obj) {
                if let Some(id) = memo.iter().position(|&p| p == ptr) {
                    buf.push(b'@');
                    buf.extend_from_slice(id.to_string().as_bytes());
                    buf.push(b'\n');
                    return Ok(());
                }
                memo.push(ptr);
            }
            buf.push(b'{');
            for (k, v) in d.items() {
                pickle_serialize(&k, buf, memo, protocol)?;
                pickle_serialize(&v, buf, memo, protocol)?;
            }
            buf.push(b'}');
        }
        PyObject::Slice { start, stop, step } => {
            buf.push(b's');
            pickle_serialize(start, buf, memo, protocol)?;
            pickle_serialize(stop, buf, memo, protocol)?;
            pickle_serialize(step, buf, memo, protocol)?;
        }
        PyObject::Range { start, stop, step } => {
            buf.push(b'R');
            pickle_serialize(&py_int(start.clone()), buf, memo, protocol)?;
            pickle_serialize(&py_int(stop.clone()), buf, memo, protocol)?;
            pickle_serialize(&py_int(step.clone()), buf, memo, protocol)?;
        }
        PyObject::ListIter { list, index } => {
            buf.push(b'i');
            pickle_serialize(&py_list(list.clone()), buf, memo, protocol)?;
            pickle_serialize(&py_int(*index as i64), buf, memo, protocol)?;
        }
        PyObject::GetItemIter { obj, index } => {
            buf.push(b'g');
            pickle_serialize(obj, buf, memo, protocol)?;
            pickle_serialize(&py_int(*index as i64), buf, memo, protocol)?;
        }
        PyObject::RangeIter {
            current,
            stop,
            step,
        } => {
            buf.push(b'r');
            pickle_serialize(&py_int(current.clone()), buf, memo, protocol)?;
            pickle_serialize(&py_int(stop.clone()), buf, memo, protocol)?;
            pickle_serialize(&py_int(step.clone()), buf, memo, protocol)?;
        }
        PyObject::DictIter { keys, index, .. } => {
            buf.push(b'i');
            let remaining: Vec<PyObjectRef> = keys[*index..].to_vec();
            pickle_serialize(&py_list(remaining), buf, memo, protocol)?;
            pickle_serialize(&py_int(0), buf, memo, protocol)?;
        }
        PyObject::DictValuesIter { values, index, .. } => {
            buf.push(b'i');
            let remaining: Vec<PyObjectRef> = values[*index..].to_vec();
            pickle_serialize(&py_list(remaining), buf, memo, protocol)?;
            pickle_serialize(&py_int(0), buf, memo, protocol)?;
        }
        PyObject::DictItemsIter { items, index, .. } => {
            buf.push(b'i');
            let remaining: Vec<PyObjectRef> = items[*index..].iter().map(|(k, v)| py_tuple(vec![k.clone(), v.clone()])).collect();
            pickle_serialize(&py_list(remaining), buf, memo, protocol)?;
            pickle_serialize(&py_int(0), buf, memo, protocol)?;
        }
        PyObject::DictRevIter { keys, index, .. } => {
            buf.push(b'i');
            let idx = *index;
            let remaining: Vec<PyObjectRef> = if idx < 0 { vec![] } else { keys[..=idx as usize].iter().rev().cloned().collect() };
            pickle_serialize(&py_list(remaining), buf, memo, protocol)?;
            pickle_serialize(&py_int(0), buf, memo, protocol)?;
        }
        // A `fractions.Fraction` (or subclass) instance — serialize the
        // class reference + a plain instance dict carrying numerator/
        // denominator. `__reduce__`-style reconstruction isn't needed since
        // the dict IS the state.
        PyObject::Instance { typ, dict }
            if crate::modules::frac_instance_num_den(obj).is_some() =>
        {
            let (module, name) = {
                let tb = typ.borrow();
                if let PyObject::Type {
                    name: tname,
                    dict: tdict,
                    ..
                } = &*tb
                {
                    let module = tdict
                        .get_str("__module__")
                        .map(|m| m.str())
                        .unwrap_or_else(|| "fractions".to_string());
                    (module, tname.clone())
                } else {
                    return Err(PyError::type_error("cannot pickle non-type instance"));
                }
            };
            buf.push(b'C');
            pickle_serialize(&py_str(&module), buf, memo, protocol)?;
            pickle_serialize(&py_str(&name), buf, memo, protocol)?;
            buf.push(b'F');
            buf.push(b'{');
            for (k, v) in dict.iter() {
                pickle_serialize(&py_str(&k), buf, memo, protocol)?;
                pickle_serialize(&v, buf, memo, protocol)?;
            }
            buf.push(b'}');
        }
        // A deque-backed SUBCLASS instance (`class Deque(deque): pass; d =
        // Deque('abc')`) — serialize the class reference (module+name), the
        // deque content (iterated through the instance's own `__iter__`, so
        // a subclass that overrides `__iter__` to raise — real CPython's
        // `DequeWithBadIter`, whose `__reduce_ex__` does `list(self)` —
        // correctly makes `pickle.dumps` raise TypeError), and the
        // instance dict. The instance's own pointer is memoized so both the
        // deque content and the instance dict can self-reference it
        // (`d.append(d)`, `d.x = d`).
        PyObject::Instance { typ, dict }
            if crate::object::native_backing_of(obj)
                .map(|n| {
                    matches!(
                        &*n.borrow(),
                        PyObject::Deque { .. } | PyObject::List(_) | PyObject::Dict(_)
                    )
                })
                .unwrap_or(false) =>
        {
            if let Some(ptr) = container_ptr(obj) {
                if let Some(id) = memo.iter().position(|&p| p == ptr) {
                    buf.push(b'@');
                    buf.extend_from_slice(id.to_string().as_bytes());
                    buf.push(b'\n');
                    return Ok(());
                }
                memo.push(ptr);
            }
            let (module, name) = {
                let tb = typ.borrow();
                if let PyObject::Type {
                    name: tname,
                    dict: tdict,
                    ..
                } = &*tb
                {
                    let module = tdict
                        .get_str("__module__")
                        .map(|m| m.str())
                        .unwrap_or_else(|| "builtins".to_string());
                    (module, tname.clone())
                } else {
                    return Err(PyError::type_error("cannot pickle non-type instance"));
                }
            };
            buf.push(b'C');
            pickle_serialize(&py_str(&module), buf, memo, protocol)?;
            pickle_serialize(&py_str(&name), buf, memo, protocol)?;
            // kind byte selects how the backing is (re)built
            let backing = crate::object::native_backing_of(obj).unwrap();
            let kind: u8 = {
                let nb = backing.borrow();
                match &*nb {
                    PyObject::Deque { .. } => b'D',
                    PyObject::List(_) => b'L',
                    PyObject::Dict(_) => b'Y',
                    _ => unreachable!(),
                }
            };
            buf.push(kind);
            if kind == b'D' {
                let maxlen = {
                    let nb = backing.borrow();
                    if let PyObject::Deque { maxlen, .. } = &*nb {
                        *maxlen
                    } else {
                        None
                    }
                };
                match maxlen {
                    Some(m) => {
                        buf.push(b'M');
                        buf.extend_from_slice(m.to_string().as_bytes());
                        buf.push(b'\n');
                    }
                    None => buf.push(b'N'),
                }
            }
            if kind == b'Y' {
                // dict-backed subclass: serialize key/value pairs directly
                buf.push(b'{');
                let items = {
                    let nb = backing.borrow();
                    if let PyObject::Dict(d) = &*nb {
                        d.items()
                    } else {
                        Vec::new()
                    }
                };
                for (k, v) in items {
                    pickle_serialize(&k, buf, memo, protocol)?;
                    pickle_serialize(&v, buf, memo, protocol)?;
                }
                buf.push(b'}');
            } else {
                // list/deque-backed subclass: items via the instance's own
                // __iter__ protocol (a subclass overriding __iter__ to raise —
                // e.g. CPython's `DequeWithBadIter`, whose `__reduce_ex__`
                // does `list(self)` — correctly makes `pickle.dumps` raise).
                buf.push(b'[');
                let it = builtin_iter(&[obj.clone()])?;
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(v) => pickle_serialize(&v, buf, memo, protocol)?,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                buf.push(b']');
            }
            // instance dict (excluding the internal native backing)
            buf.push(b'{');
            for (k, v) in dict.iter() {
                if k == crate::object::NATIVE_BACKING_KEY {
                    continue;
                }
                pickle_serialize(&py_str(&k), buf, memo, protocol)?;
                pickle_serialize(&v, buf, memo, protocol)?;
            }
            buf.push(b'}');
        }
        // A module-level function — serialized BY REFERENCE (module +
        // name), like real pickle's save_global. Unpickling resolves the
        // global again.
        PyObject::Function(f) => {
            buf.push(b'E');
            let module = f
                .dict
                .get("__module__")
                .map(|m| m.str())
                .or_else(|| {
                    f.globals
                        .borrow()
                        .get(&crate::interner::intern("__name__"))
                        .map(|m| m.str())
                })
                .unwrap_or_else(|| "builtins".to_string());
            pickle_serialize(&py_str(&module), buf, memo, protocol)?;
            pickle_serialize(
                &py_str(&crate::interner::lookup_str(f.code.name)),
                buf,
                memo,
                protocol,
            )?;
        }
        PyObject::BuiltinFunction { .. } | PyObject::Closure(_) => {
            let own_name = match &*obj.borrow() {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                _ => String::new(),
            };
            let target_ptr = container_ptr(obj);
            let mut found: Option<(String, String)> = None;
            crate::object::with_vm_mut(|vm| {
                for (mod_name_str, mref) in vm.modules.iter() {
                    let mname = mod_name_str.clone();
                    let mborrow = mref.borrow();
                    if let PyObject::Module { dict, .. } = &*mborrow {
                        for (k, v) in dict.iter() {
                            let ptr_match = if let (Some(tp), Some(vp)) =
                                (target_ptr, container_ptr(&v))
                            {
                                tp == vp
                            } else {
                                false
                            };
                            let k_str = crate::interner::lookup_str(*k);
                            let name_match = !own_name.is_empty()
                                && k_str == own_name
                                && v.borrow().type_name() == "builtin_function_or_method";
                            if ptr_match || name_match {
                                found = Some((mname.clone(), k_str.to_string()));
                                break;
                            }
                        }
                        if found.is_some() {
                            break;
                        }
                    }
                }
                if found.is_none() {
                    for (k, v) in vm.builtins.iter() {
                        let bname = crate::interner::lookup_str(*k);
                        let ptr_match = if let (Some(tp), Some(vp)) =
                            (target_ptr, container_ptr(&v))
                        {
                            tp == vp
                        } else {
                            false
                        };
                        let name_match = !own_name.is_empty() && bname == own_name;
                        if ptr_match || name_match {
                            found = Some(("builtins".to_string(), bname.to_string()));
                            break;
                        }
                    }
                }
            })
            .ok();
            if let Some((mod_name, attr_name)) = found {
                buf.push(b'E');
                pickle_serialize(&py_str(&mod_name), buf, memo, protocol)?;
                pickle_serialize(&py_str(&attr_name), buf, memo, protocol)?;
            } else if !own_name.is_empty() {
                let mod_guess = if [
                    "add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lt", "le",
                    "eq", "ne", "ge", "gt", "and_", "or_", "xor", "not_", "getitem",
                    "setitem", "delitem", "contains", "index", "indexOf", "countOf",
                    "truth", "neg", "pos", "abs", "inv", "lshift", "rshift", "length_hint",
                    "is_", "is_not", "itemgetter", "attrgetter", "methodcaller",
                ]
                .contains(&own_name.as_str())
                {
                    "operator"
                } else {
                    "builtins"
                };
                buf.push(b'E');
                pickle_serialize(&py_str(mod_guess), buf, memo, protocol)?;
                pickle_serialize(&py_str(&own_name), buf, memo, protocol)?;
            } else {
                return Err(PyError::type_error(format!(
                    "cannot pickle {} object",
                    obj.borrow().type_name()
                )));
            }
        }
        PyObject::Exception {
            typ, args, extra, ..
        } => {
            // Exceptions serialize as: tag 'X', type name, args tuple, extra
            // dict (or 'N'). test_exceptions' testAttributes/test_copy_pickle
            // round-trip every exception and its attributes.
            buf.push(b'X');
            pickle_serialize(&py_str(typ), buf, memo, protocol)?;
            buf.push(b'(');
            for a in args {
                pickle_serialize(a, buf, memo, protocol)?;
            }
            buf.push(b')');
            if let Some(extra) = extra {
                buf.push(b'{');
                for (k, v) in extra.iter() {
                    pickle_serialize(&py_str(k), buf, memo, protocol)?;
                    pickle_serialize(&v, buf, memo, protocol)?;
                }
                buf.push(b'}');
            } else {
                buf.push(b'N');
            }
        }
        PyObject::Type { name, dict: tdict, .. } => {
            // Classes-as-values (e.g. defaultdict's factory argument):
            // register in the same name->type registry the instance
            // deserializer uses, then emit 'T' <name>.
            let cname = name.clone();
            if let Some(ptr) = container_ptr(obj) {
                if let Some(id) = memo.iter().position(|&p| p == ptr) {
                    buf.push(b'@');
                    buf.extend_from_slice(id.to_string().as_bytes());
                    buf.push(b'\n');
                    return Ok(());
                }
                memo.push(ptr);
            }
            let module = tdict
                .get_str("__module__")
                .map(|m| m.str())
                .unwrap_or_else(|| "builtins".into());
            let _ = module;
            PICKLE_CLASS_REGISTRY.with(|r| {
                r.borrow_mut().insert(cname.clone(), obj.clone());
            });
            buf.push(b'P');
            pickle_serialize(&py_str(&cname), buf, memo, protocol)?;
        }
        PyObject::Instance { typ, dict } => {
            // Plain user-class instance (no native backing): memoize by
            // pointer, register the CLASS for the deserializer, emit
            //   'K' <class-name-str> <attrs-as-dict>
            if let Some(ptr) = container_ptr(obj) {
                if let Some(id) = memo.iter().position(|&p| p == ptr) {
                    buf.push(b'@');
                    buf.extend_from_slice(id.to_string().as_bytes());
                    buf.push(b'\n');
                    return Ok(());
                }
                memo.push(ptr);
            }
            let cname = {
                let tb = typ.borrow();
                match &*tb {
                    PyObject::Type { name, .. } => name.clone(),
                    _ => {
                        return Err(PyError::type_error("cannot pickle non-type instance"))
                    }
                }
            };
            PICKLE_CLASS_REGISTRY.with(|r| {
                r.borrow_mut().insert(cname.clone(), typ.clone());
            });
            buf.push(b'K');
            pickle_serialize(&py_str(&cname), buf, memo, protocol)?;
            let mut flat = crate::object::PyDict::new();
            for k in dict.keys() {
                if let Some(v) = dict.get(k) {
                    let _ = flat.set(crate::object::py_str(k), v.clone());
                }
            }
            pickle_serialize(
                &PyObjectRef::new(PyObject::Dict(Box::new(flat))),
                buf,
                memo,
                protocol,
            )?;
        }
        _ => {
            // Try set/frozenset/complex before failing
            let type_name = obj.borrow().type_name().to_string();
            match type_name.as_str() {
                "set" => {
                    if let PyObject::Set(s) = &*obj.borrow() {
                        // Use dedicated set opcode 'Y' with [elements]
                        buf.push(b'Y');
                        buf.push(b'[');
                        for item in s.iter() {
                            pickle_serialize(&item, buf, memo, protocol)?;
                        }
                        buf.push(b']');
                    }
                }
                "frozenset" => {
                    if let PyObject::FrozenSet(s) = &*obj.borrow() {
                        // Use dedicated frozenset opcode 'Z' with [elements]
                        buf.push(b'Z');
                        buf.push(b'[');
                        for item in s.iter() {
                            pickle_serialize(&item, buf, memo, protocol)?;
                        }
                        buf.push(b']');
                    }
                }
                _ => {
                    return Err(PyError::type_error(format!(
                        "cannot pickle {} object",
                        type_name
                    )));
                }
            }
        }
    }
    Ok(())
}
