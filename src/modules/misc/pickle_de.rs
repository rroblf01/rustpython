use crate::object::*;
use std::collections::HashMap;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;
use crate::modules::misc::pickle_ser::PICKLE_CLASS_REGISTRY;

pub fn try_unpickle_rangeiter_compat(data: &[u8]) -> Option<PyObjectRef> {
    // Quick reject: must contain "iter" and ("xrange" or "range").
    let has_iter = data.windows(4).any(|w| w == b"iter");
    let has_range = data.windows(5).any(|w| w == b"range");
    if !(has_iter && has_range) {
        return None;
    }
    // Minimal pickle stack machine for the compat patterns.
    #[derive(Clone, Debug)]
    enum StackVal {
        Mark,
        Val(PyObjectRef),
        GlobalRange,
        GlobalIter,
    }
    let mut stack: Vec<StackVal> = Vec::new();
    let mut pos = 0usize;
    // Skip PROTO 0x80 0x?? and FRAME 0x95 ...
    let mut _frame_end: Option<usize> = None;
    // Helper to parse BigInt from decimal string.
    let parse_bigint = |s: &str| -> Option<BigInt> {
        let t = s.trim().trim_end_matches('L');
        if t.is_empty() { return None; }
        t.parse::<BigInt>().ok()
    };
    // Helper to decode LONG1 n bytes LE signed.
    let decode_long1 = |n: usize, bytes: &[u8]| -> BigInt {
        if n == 0 { return BigInt::from(0); }
        let negative = bytes[n-1] & 0x80 != 0;
        let mut mag = BigInt::from(0);
        for &b in bytes.iter().rev() {
            mag = (mag << 8) | BigInt::from(b);
        }
        if negative {
            let bits = (n * 8) as u32;
            let modulus = BigInt::from(1u32) << bits;
            mag - modulus
        } else { mag }
    };
    while pos < data.len() {
        let op = data[pos];
        pos += 1;
        match op {
            0x80 => {
                // PROTO version byte
                if pos < data.len() { pos += 1; }
            }
            0x95 => {
                // FRAME: 8-byte LE length
                if pos + 8 > data.len() { return None; }
                let len = u64::from_le_bytes([
                    data[pos], data[pos+1], data[pos+2], data[pos+3],
                    data[pos+4], data[pos+5], data[pos+6], data[pos+7],
                ]) as usize;
                pos += 8;
                _frame_end = Some(pos + len);
            }
            0x8c => {
                // SHORT_BINUNICODE: 1-byte len + bytes
                if pos >= data.len() { return None; }
                let n = data[pos] as usize;
                pos += 1;
                if pos + n > data.len() { return None; }
                let s = std::str::from_utf8(&data[pos..pos+n]).ok()?;
                pos += n;
                // This is a unicode string value; for our hack we just push Val
                // It will be consumed by STACK_GLOBAL.
                stack.push(StackVal::Val(py_str(s)));
            }
            0x8a => {
                // LONG1: 1-byte n then n bytes LE
                if pos >= data.len() { return None; }
                let n = data[pos] as usize;
                pos += 1;
                if pos + n > data.len() { return None; }
                let v = decode_long1(n, &data[pos..pos+n]);
                pos += n;
                stack.push(StackVal::Val(py_int(v)));
            }
            0x8b => {
                // LONG4: 4-byte LE n then n bytes
                if pos + 4 > data.len() { return None; }
                let n = u32::from_le_bytes([data[pos],data[pos+1],data[pos+2],data[pos+3]]) as usize;
                pos += 4;
                if pos + n > data.len() { return None; }
                let v = decode_long1(n, &data[pos..pos+n]);
                pos += n;
                stack.push(StackVal::Val(py_int(v)));
            }
            b'c' => {
                // GLOBAL: module\n name\n
                let start = pos;
                while pos < data.len() && data[pos] != b'\n' { pos += 1; }
                if pos >= data.len() { return None; }
                let module = std::str::from_utf8(&data[start..pos]).ok()?.to_string();
                pos += 1;
                let start2 = pos;
                while pos < data.len() && data[pos] != b'\n' { pos += 1; }
                if pos >= data.len() { return None; }
                let name = std::str::from_utf8(&data[start2..pos]).ok()?.to_string();
                pos += 1;
                match (module.as_str(), name.as_str()) {
                    ("__builtin__", "iter") | ("builtins", "iter") => stack.push(StackVal::GlobalIter),
                    ("__builtin__", "xrange") | ("__builtin__", "range") | ("builtins", "range") => stack.push(StackVal::GlobalRange),
                    _ => return None,
                }
            }
            0x93 => {
                // STACK_GLOBAL: pops module and name (previously pushed by BINUNICODE)
                if stack.len() < 2 { return None; }
                let name_v = stack.pop().unwrap();
                let module_v = stack.pop().unwrap();
                let (module, name) = match (module_v, name_v) {
                    (StackVal::Val(m), StackVal::Val(n)) => (m.str(), n.str()),
                    _ => return None,
                };
                match (module.as_str(), name.as_str()) {
                    ("builtins", "iter") => stack.push(StackVal::GlobalIter),
                    ("builtins", "range") => stack.push(StackVal::GlobalRange),
                    _ => return None,
                }
            }
            b'(' => stack.push(StackVal::Mark),
            b'I' => {
                let start = pos;
                while pos < data.len() && data[pos] != b'\n' { pos += 1; }
                if pos >= data.len() { return None; }
                let s = std::str::from_utf8(&data[start..pos]).ok()?;
                pos += 1;
                let v = parse_bigint(s)?;
                stack.push(StackVal::Val(py_int(v)));
            }
            b'K' => {
                if pos >= data.len() { return None; }
                let v = data[pos] as i64;
                pos += 1;
                stack.push(StackVal::Val(py_int(v)));
            }
            b'M' => {
                if pos + 2 > data.len() { return None; }
                let v = u16::from_le_bytes([data[pos], data[pos+1]]) as i64;
                pos += 2;
                stack.push(StackVal::Val(py_int(v)));
            }
            b'J' => {
                if pos + 4 > data.len() { return None; }
                let v = i32::from_le_bytes([data[pos],data[pos+1],data[pos+2],data[pos+3]]) as i64;
                pos += 4;
                stack.push(StackVal::Val(py_int(v)));
            }
            b'L' => {
                let start = pos;
                while pos < data.len() && data[pos] != b'\n' { pos += 1; }
                if pos >= data.len() { return None; }
                let s = std::str::from_utf8(&data[start..pos]).ok()?;
                pos += 1;
                let v = parse_bigint(s)?;
                stack.push(StackVal::Val(py_int(v)));
            }
            b't' => {
                // TUPLE from MARK
                let mut items = Vec::new();
                while let Some(top) = stack.pop() {
                    match top {
                        StackVal::Mark => break,
                        StackVal::Val(v) => items.push(v),
                        _ => return None,
                    }
                }
                items.reverse();
                stack.push(StackVal::Val(py_tuple(items)));
            }
            0x85 => {
                // TUPLE1
                if let Some(StackVal::Val(v)) = stack.pop() {
                    stack.push(StackVal::Val(py_tuple(vec![v])));
                } else { return None; }
            }
            0x86 => {
                // TUPLE2
                if stack.len() < 2 { return None; }
                let b = match stack.pop().unwrap() { StackVal::Val(v)=>v, _=>return None };
                let a = match stack.pop().unwrap() { StackVal::Val(v)=>v, _=>return None };
                stack.push(StackVal::Val(py_tuple(vec![a,b])));
            }
            0x87 => {
                // TUPLE3
                if stack.len() < 3 { return None; }
                let c = match stack.pop().unwrap() { StackVal::Val(v)=>v, _=>return None };
                let b = match stack.pop().unwrap() { StackVal::Val(v)=>v, _=>return None };
                let a = match stack.pop().unwrap() { StackVal::Val(v)=>v, _=>return None };
                stack.push(StackVal::Val(py_tuple(vec![a,b,c])));
            }
            b'R' => {
                // REDUCE
                let args_v = stack.pop()?;
                let callable = stack.pop()?;
                let args = match args_v {
                    StackVal::Val(v) => {
                        if let PyObject::Tuple(items) = &*v.borrow() { items.clone() } else { return None; }
                    }
                    _ => return None,
                };
                match callable {
                    StackVal::GlobalRange => {
                        // range(*args)
                        let (start_v, stop_v, step_v) = match args.len() {
                            1 => (py_int(0), args[0].clone(), py_int(1)),
                            2 => (args[0].clone(), args[1].clone(), py_int(1)),
                            3 => (args[0].clone(), args[1].clone(), args[2].clone()),
                            _ => return None,
                        };
                        let s = crate::object::to_index(&start_v).ok()?;
                        let e = crate::object::to_index(&stop_v).ok()?;
                        let p = crate::object::to_index(&step_v).ok()?;
                        let r = PyObjectRef::imm(PyObject::Range { start: s, stop: e, step: p });
                        stack.push(StackVal::Val(r));
                    }
                    StackVal::GlobalIter => {
                        if args.len() != 1 { return None; }
                        let range_obj = args[0].clone();
                        let (start, stop, step) = match &*range_obj.borrow() {
                            PyObject::Range { start, stop, step } => (start.clone(), stop.clone(), step.clone()),
                            _ => return None,
                        };
                        let iter = PyObjectRef::new(PyObject::RangeIter { current: start.clone(), stop, step });
                        stack.push(StackVal::Val(iter));
                    }
                    _ => return None,
                }
            }
            b'b' => {
                // BUILD: pops state, then object, then calls __setstate__
                let state_v = stack.pop()?;
                let obj_v = stack.pop()?;
                let state = match state_v {
                    StackVal::Val(v) => crate::object::to_index(&v).ok()?,
                    _ => return None,
                };
                let obj = match obj_v { StackVal::Val(v)=>v, _=>return None };
                // RangeIter BUILD: state is index
                let (cur, st, stop_c) = {
                    let b = obj.borrow();
                    if let PyObject::RangeIter { current, stop, step } = &*b {
                        (current.clone(), step.clone(), stop.clone())
                    } else {
                        return None;
                    }
                };
                let new_current = cur + &st * &state;
                let new_iter = PyObjectRef::new(PyObject::RangeIter { current: new_current, stop: stop_c, step: st });
                stack.push(StackVal::Val(new_iter));
            }
            0x81 => {
                // NEWOBJ? not needed
                return None;
            }
            b'.' => {
                // STOP
                break;
            }
            b'\n' | b' ' => { /* whitespace? */ }
            _ => {
                // Unknown opcode - fail to fall back to normal path
                return None;
            }
        }
    }
    // After STOP, stack should contain single RangeIter
    if stack.len() == 1 {
        if let StackVal::Val(v) = &stack[0] {
            if matches!(&*v.borrow(), PyObject::RangeIter { .. }) {
                return Some(v.clone());
            }
        }
    }
    // Also handle case where there's extra marks? Try to find RangeIter in stack
    for sv in stack.iter().rev() {
        if let StackVal::Val(v) = sv {
            if matches!(&*v.borrow(), PyObject::RangeIter { .. }) {
                return Some(v.clone());
            }
        }
    }
    None
}

/// Deserialize a Python object from bytes using the custom pickle format.
/// Deserialize a Python object from bytes using the custom pickle format.
/// `memo` mirrors the serializer's container memo: each container's ref is
/// registered BEFORE its children are read, so a `@<id>` reference (a cycle
/// or an alias) resolves to the shared object being built.
pub fn pickle_deserialize(
    data: &[u8],
    pos: &mut usize,
    memo: &mut Vec<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    if *pos >= data.len() {
        return Err(PyError::type_error("unexpected end of pickle data"));
    }
    let marker = data[*pos];
    *pos += 1;
            match marker {
        b'N' => Ok(py_none()),
        b'T' => Ok(py_bool(true)),
        b'F' => Ok(py_bool(false)),
        0x80 => {
            // PROTO: protocol version byte — skip it
            *pos += 1;
            pickle_deserialize(data, pos, memo)
        }
        0x88 => Ok(py_bool(true)),  // NEWTRUE
        0x89 => Ok(py_bool(false)), // NEWFALSE
        b'I' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b'\n' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated integer in pickle data"));
            }
            let s = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle int"))?;
            *pos += 1; // skip '\n'
            let n: num_bigint::BigInt = s
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid integer: {}", s)))?;
            // Protocol 0: integers 0 and 1 followed by stop marker (.) are booleans
            if *pos < data.len() && data[*pos] == b'.' {
                if s == "0" || s == "00" {
                    return Ok(py_bool(false));
                } else if s == "1" || s == "01" {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_int(n))
        }
        b'G' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b'\n' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated float in pickle data"));
            }
            let s = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle float"))?;
            *pos += 1; // skip '\n'
            let f: f64 = s
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid float: {}", s)))?;
            Ok(py_float(f))
        }
        b'S' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b':' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error(
                    "unterminated string length in pickle data",
                ));
            }
            let len_str = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle string length"))?;
            let len: usize = len_str
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid string length: {}", len_str)))?;
            *pos += 1; // skip ':'
            if *pos + len > data.len() {
                return Err(PyError::type_error("unexpected end of pickle string data"));
            }
            let s = std::str::from_utf8(&data[*pos..*pos + len])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle string"))?;
            *pos += len;
            Ok(py_str(s))
        }
        b'P' => {
            // Class reference by name.
            let name_val = pickle_deserialize(data, pos, memo)?;
            let cname = name_val.str();
            if let Some(t) =
                PICKLE_CLASS_REGISTRY.with(|r| r.borrow().get(&cname).cloned())
            {
                return Ok(t);
            }
            // Fallback: resolve through live builtins/modules tables.
            match crate::object::with_vm_mut(|vm| {
                if let Some(b) = vm.builtins.get(&crate::interner::intern(&cname)) {
                    return Ok(b.clone());
                }
                for m in vm.modules.values() {
                    if let Ok(v) = crate::object::ObjectAccess::get_attribute(
                        &*m.borrow(),
                        &cname,
                    ) {
                        if matches!(&*v.borrow(), PyObject::Type { .. }) {
                            return Ok(v);
                        }
                    }
                }
                Err(PyError::type_error(format!(
                    "cannot unpickle class '{}'",
                    cname
                )))
            }) {
                Ok(v) => v,
                Err(e) => return Err(e),
            }
        }
        b'K' => {
            // User-class instance: 'K' <class-name-str> <attrs-dict>.
            // The instance is created and REGISTERED IN MEMO before its
            // attributes are read, mirroring the serializer's order -- that
            // is what makes self-referencing attributes resolve to the same
            // object instead of duplicating it.
            let name_val = pickle_deserialize(data, pos, memo)?;
            let cname = name_val.str();
            let typ = PICKLE_CLASS_REGISTRY
                .with(|r| r.borrow().get(&cname).cloned())
                .ok_or_else(|| {
                    PyError::type_error(format!(
                        "cannot unpickle class '{}' (not seen in this process)",
                        cname
                    ))
                })?;
            let inst = PyObjectRef::new(PyObject::Instance {
                typ,
                dict: crate::object::AttrMap::new(),
            });
            memo.push(inst.clone());
            let attrs = pickle_deserialize(data, pos, memo)?;
            if let PyObject::Dict(dd) = &*attrs.borrow() {
                for (k, v) in dd.items() {
                    if let PyObject::Instance { dict, .. } = &mut *inst.borrow_mut() {
                        dict.insert(k.str(), v.clone());
                    }
                }
            }
            Ok(inst)
        }

        b'B' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b':' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error(
                    "unterminated bytes length in pickle data",
                ));
            }
            let len_str = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle bytes length"))?;
            let len: usize = len_str
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid bytes length: {}", len_str)))?;
            *pos += 1; // skip ':'
            if *pos + len > data.len() {
                return Err(PyError::type_error("unexpected end of pickle bytes data"));
            }
            let bytes = data[*pos..*pos + len].to_vec();
            *pos += len;
            Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
        }
        b'[' => {
            let list_ref = py_list(Vec::new());
            memo.push(list_ref.clone());
            let mut items = Vec::new();
            while *pos < data.len() && data[*pos] != b']' {
                items.push(pickle_deserialize(data, pos, memo)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated list in pickle data"));
            }
            *pos += 1; // skip ']'
            if let PyObject::List(l) = &mut *list_ref.borrow_mut() {
                *l = items;
            }
            Ok(list_ref)
        }
        b'D' => {
            let deque_ref = py_deque(std::collections::VecDeque::new(), None);
            memo.push(deque_ref.clone());
            let maxlen = match data.get(*pos) {
                Some(b'M') => {
                    *pos += 1;
                    let start = *pos;
                    while *pos < data.len() && data[*pos] != b'\n' {
                        *pos += 1;
                    }
                    if *pos >= data.len() {
                        return Err(PyError::type_error("unterminated maxlen in pickle data"));
                    }
                    let s = std::str::from_utf8(&data[start..*pos])
                        .map_err(|_| PyError::type_error("invalid utf-8 in pickle maxlen"))?;
                    *pos += 1; // skip '\n'
                    Some(
                        s.parse::<usize>()
                            .map_err(|_| PyError::type_error(format!("invalid maxlen: {}", s)))?,
                    )
                }
                Some(b'N') => {
                    *pos += 1;
                    None
                }
                _ => return Err(PyError::type_error("malformed deque pickle data")),
            };
            if *pos >= data.len() || data[*pos] != b'[' {
                return Err(PyError::type_error("malformed deque pickle data"));
            }
            *pos += 1;
            let mut items = std::collections::VecDeque::new();
            while *pos < data.len() && data[*pos] != b']' {
                items.push_back(pickle_deserialize(data, pos, memo)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated deque in pickle data"));
            }
            *pos += 1; // skip ']'
            if let PyObject::Deque {
                data: d,
                maxlen: ml,
            } = &mut *deque_ref.borrow_mut()
            {
                *d = items;
                *ml = maxlen;
            }
            Ok(deque_ref)
        }
        b'q' => {
            let deque = pickle_deserialize(data, pos, memo)?;
            let index = pickle_deserialize(data, pos, memo)?;
            let start_len = pickle_deserialize(data, pos, memo)?;
            Ok(PyObjectRef::new(PyObject::DequeIter {
                deque,
                index: index.as_i64().unwrap_or(0) as usize,
                start_len: start_len.as_i64().unwrap_or(0) as usize,
            }))
        }
        b'@' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b'\n' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error(
                    "unterminated memo reference in pickle data",
                ));
            }
            let s = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle memo reference"))?;
            *pos += 1; // skip '\n'
            let id: usize = s
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid memo reference: {}", s)))?;
            memo.get(id).cloned().ok_or_else(|| {
                PyError::type_error(format!("pickle memo reference out of range: {}", id))
            })
        }
        b'E' => {
            // Function / builtin by reference (see the matching serializer arm).
            let module = pickle_deserialize(data, pos, memo)?;
            let name = pickle_deserialize(data, pos, memo)?;
            let module_str = module.str();
            let name_str = name.str();
            let func = crate::modules::get_module(&module_str)
                .and_then(|m| m.borrow().get_attribute(&name_str).ok())
                .or_else(|| {
                    crate::object::with_vm_mut(|vm| {
                        if let Some(mref) = vm.modules.get(&module_str) {
                            if let Ok(v) = mref.borrow().get_attribute(&name_str) {
                                return Some(v);
                            }
                        }
                        if module_str == "builtins" {
                            if let Some(b) =
                                vm.builtins.get(&crate::interner::intern(&name_str))
                            {
                                return Some(b.clone());
                            }
                        }
                        None
                    })
                    .ok()
                    .flatten()
                })
                .ok_or_else(|| {
                    PyError::type_error(format!(
                        "cannot find function {}.{} referenced by pickle data",
                        module_str, name_str
                    ))
                })?;
            Ok(func)
        }
        b'X' => {
            let typ = pickle_deserialize(data, pos, memo)?.str();
            // args tuple: '(' ... ')'
            if *pos >= data.len() || data[*pos] != b'(' {
                return Err(PyError::type_error(
                    "malformed exception pickle data (args)",
                ));
            }
            *pos += 1;
            let mut args: Vec<PyObjectRef> = Vec::new();
            while *pos < data.len() && data[*pos] != b')' {
                args.push(pickle_deserialize(data, pos, memo)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error(
                    "unterminated exception args in pickle data",
                ));
            }
            *pos += 1; // ')'
                       // extra dict or 'N'
            let mut extra = None;
            if let Some(marker) = data.get(*pos).copied() {
                *pos += 1;
                if marker == b'{' {
                    let mut m = std::collections::HashMap::new();
                    while *pos < data.len() && data[*pos] != b'}' {
                        let k = pickle_deserialize(data, pos, memo)?;
                        let v = pickle_deserialize(data, pos, memo)?;
                        m.insert(k.str(), v);
                    }
                    if *pos < data.len() {
                        *pos += 1; // '}'
                    }
                    if !m.is_empty() {
                        extra = Some(m);
                    }
                }
            }
            Ok(PyObjectRef::new(PyObject::Exception {
                typ,
                args,
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra,
            }))
        }
        b'C' => {
            let module = pickle_deserialize(data, pos, memo)?;
            let name = pickle_deserialize(data, pos, memo)?;
            let module_str = module.str();
            let class_name = name.str();
            // Resolve the class from the global class registry (every
            // user-defined class is registered at construction) — NOT
            // `sys.modules`/`vm.modules`, which are VM-relative and
            // unreliable here because the active VM pointer can be a
            // transient disposable one during `pickle.loads`.
            let typ = crate::object::find_class_by_qualified_name(&module_str, &class_name)
                .ok_or_else(|| {
                    PyError::type_error(format!(
                        "cannot find class {}.{} referenced by pickle data",
                        module_str, class_name
                    ))
                })?;
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: typ.clone(),
                dict: AttrMap::new(),
            });
            memo.push(instance.clone());
            let kind = data
                .get(*pos)
                .copied()
                .ok_or_else(|| PyError::type_error("malformed instance pickle data"))?;
            *pos += 1;
            let backing = match kind {
                b'D' => {
                    let maxlen = match data.get(*pos) {
                        Some(b'M') => {
                            *pos += 1;
                            let start = *pos;
                            while *pos < data.len() && data[*pos] != b'\n' {
                                *pos += 1;
                            }
                            if *pos >= data.len() {
                                return Err(PyError::type_error(
                                    "unterminated maxlen in pickle data",
                                ));
                            }
                            let s = std::str::from_utf8(&data[start..*pos]).map_err(|_| {
                                PyError::type_error("invalid utf-8 in pickle maxlen")
                            })?;
                            *pos += 1;
                            Some(s.parse::<usize>().map_err(|_| {
                                PyError::type_error(format!("invalid maxlen: {}", s))
                            })?)
                        }
                        Some(b'N') => {
                            *pos += 1;
                            None
                        }
                        _ => {
                            return Err(PyError::type_error("malformed deque-instance pickle data"))
                        }
                    };
                    if *pos >= data.len() || data[*pos] != b'[' {
                        return Err(PyError::type_error("malformed deque-instance pickle data"));
                    }
                    *pos += 1;
                    let mut items = std::collections::VecDeque::new();
                    while *pos < data.len() && data[*pos] != b']' {
                        items.push_back(pickle_deserialize(data, pos, memo)?);
                    }
                    if *pos >= data.len() {
                        return Err(PyError::type_error(
                            "unterminated deque-instance in pickle data",
                        ));
                    }
                    *pos += 1;
                    py_deque(items, maxlen)
                }
                b'L' => {
                    if *pos >= data.len() || data[*pos] != b'[' {
                        return Err(PyError::type_error("malformed list-instance pickle data"));
                    }
                    *pos += 1;
                    let mut items = Vec::new();
                    while *pos < data.len() && data[*pos] != b']' {
                        items.push(pickle_deserialize(data, pos, memo)?);
                    }
                    if *pos >= data.len() {
                        return Err(PyError::type_error(
                            "unterminated list-instance in pickle data",
                        ));
                    }
                    *pos += 1;
                    py_list(items)
                }
                b'Y' => {
                    if *pos >= data.len() || data[*pos] != b'{' {
                        return Err(PyError::type_error("malformed dict-instance pickle data"));
                    }
                    *pos += 1;
                    let mut dict = PyDict::new();
                    while *pos < data.len() && data[*pos] != b'}' {
                        let k = pickle_deserialize(data, pos, memo)?;
                        if *pos >= data.len() {
                            return Err(PyError::type_error(
                                "unterminated dict-instance in pickle data",
                            ));
                        }
                        let v = pickle_deserialize(data, pos, memo)?;
                        dict.set(k, v)?;
                    }
                    if *pos >= data.len() {
                        return Err(PyError::type_error(
                            "unterminated dict-instance in pickle data",
                        ));
                    }
                    *pos += 1;
                    PyObjectRef::new(PyObject::Dict(Box::new(dict)))
                }
                b'F' => {
                    // `fractions.Fraction`-style: no native backing, the
                    // instance dict (numerator/denominator) follows.
                    py_none()
                }
                _ => {
                    return Err(PyError::type_error(format!(
                        "unknown instance backing kind: {}",
                        kind as char
                    )))
                }
            };
            if *pos >= data.len() || data[*pos] != b'{' {
                return Err(PyError::type_error("malformed deque-instance pickle data"));
            }
            *pos += 1;
            let mut inst_dict = AttrMap::new();
            while *pos < data.len() && data[*pos] != b'}' {
                let k = pickle_deserialize(data, pos, memo)?;
                if *pos >= data.len() {
                    return Err(PyError::type_error(
                        "unterminated instance dict in pickle data",
                    ));
                }
                let v = pickle_deserialize(data, pos, memo)?;
                inst_dict.insert(k.str(), v);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error(
                    "unterminated instance dict in pickle data",
                ));
            }
            *pos += 1;
            if !matches!(&*backing.borrow(), PyObject::None) {
                inst_dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), backing);
            }
            if let PyObject::Instance { dict: d, .. } = &mut *instance.borrow_mut() {
                *d = inst_dict;
            }
            Ok(instance)
        }
        b'(' => {
            let mut items = Vec::new();
            while *pos < data.len() && data[*pos] != b')' {
                items.push(pickle_deserialize(data, pos, memo)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated tuple in pickle data"));
            }
            *pos += 1; // skip ')'
            Ok(py_tuple(items))
        }
        b'Y' => {
            // set: [elements]
            if *pos >= data.len() || data[*pos] != b'[' {
                return Err(PyError::type_error("malformed set pickle data"));
            }
            *pos += 1;
            let mut items = Vec::new();
            while *pos < data.len() && data[*pos] != b']' {
                items.push(pickle_deserialize(data, pos, memo)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated set in pickle data"));
            }
            *pos += 1;
            let s = crate::object::PySet::from_vec(items)
                .map_err(|e| PyError::type_error(format!("failed to create set: {}", e)))?;
            Ok(PyObjectRef::new(PyObject::Set(s)))
        }
        b'Z' => {
            // frozenset: [elements]
            if *pos >= data.len() || data[*pos] != b'[' {
                return Err(PyError::type_error("malformed frozenset pickle data"));
            }
            *pos += 1;
            let mut items = Vec::new();
            while *pos < data.len() && data[*pos] != b']' {
                items.push(pickle_deserialize(data, pos, memo)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated frozenset in pickle data"));
            }
            *pos += 1;
            let s = crate::object::PySet::from_vec(items)
                .map_err(|e| PyError::type_error(format!("failed to create frozenset: {}", e)))?;
            Ok(PyObjectRef::new(PyObject::FrozenSet(s)))
        }
        b'{' => {
            let dict_ref = PyObjectRef::new(PyObject::Dict(Box::new(crate::object::PyDict::new())));
            memo.push(dict_ref.clone());
            while *pos < data.len() && data[*pos] != b'}' {
                let key = pickle_deserialize(data, pos, memo)?;
                if *pos >= data.len() {
                    return Err(PyError::type_error("unterminated dict in pickle data"));
                }
                let value = pickle_deserialize(data, pos, memo)?;
                if let PyObject::Dict(d) = &mut *dict_ref.borrow_mut() {
                    d.set(key, value)?;
                }
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated dict in pickle data"));
            }
            *pos += 1; // skip '}'
            Ok(dict_ref)
        }
        b'R' => {
            let start = pickle_deserialize(data, pos, memo)?;
            let stop = pickle_deserialize(data, pos, memo)?;
            let step = pickle_deserialize(data, pos, memo)?;
            let s = crate::object::to_index(&start).unwrap_or_else(|_| num_bigint::BigInt::from(0));
            let e = crate::object::to_index(&stop).unwrap_or_else(|_| num_bigint::BigInt::from(0));
            let p = crate::object::to_index(&step).unwrap_or_else(|_| num_bigint::BigInt::from(1));
            Ok(PyObjectRef::imm(PyObject::Range {
                start: s,
                stop: e,
                step: p,
            }))
        }
        b's' => {
            let start = pickle_deserialize(data, pos, memo)?;
            let stop = pickle_deserialize(data, pos, memo)?;
            let step = pickle_deserialize(data, pos, memo)?;
            Ok(PyObjectRef::imm(PyObject::Slice { start, stop, step }))
        }
        b'i' => {
            let list = pickle_deserialize(data, pos, memo)?;
            let index = pickle_deserialize(data, pos, memo)?;
            let items = match &*list.borrow() {
                PyObject::List(items) => items.clone(),
                _ => return Err(PyError::type_error("invalid list_iterator pickle data")),
            };
            let idx = index.as_i64().unwrap_or(0) as usize;
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: items,
                index: idx,
            }))
        }
        b'g' => {
            let obj = pickle_deserialize(data, pos, memo)?;
            let index = pickle_deserialize(data, pos, memo)?;
            let idx = index.as_i64().unwrap_or(0);
            Ok(PyObjectRef::new(PyObject::GetItemIter { obj, index: idx }))
        }
        b'r' => {
            let current = pickle_deserialize(data, pos, memo)?;
            let stop = pickle_deserialize(data, pos, memo)?;
            let step = pickle_deserialize(data, pos, memo)?;
            let c =
                crate::object::to_index(&current).unwrap_or_else(|_| num_bigint::BigInt::from(0));
            let e = crate::object::to_index(&stop).unwrap_or_else(|_| num_bigint::BigInt::from(0));
            let p = crate::object::to_index(&step).unwrap_or_else(|_| num_bigint::BigInt::from(1));
            Ok(PyObjectRef::new(PyObject::RangeIter {
                current: c,
                stop: e,
                step: p,
            }))
        }
        b'c' => {
            // GLOBAL: module\nname\n
            let module = {
                let start = *pos;
                while *pos < data.len() && data[*pos] != b'\n' {
                    *pos += 1;
                }
                let s = std::str::from_utf8(&data[start..*pos])
                    .map_err(|_| PyError::type_error("invalid utf-8 in pickle GLOBAL"))?
                    .to_string();
                *pos += 1; // skip '\n'
                s
            };
            let name = {
                let start = *pos;
                while *pos < data.len() && data[*pos] != b'\n' {
                    *pos += 1;
                }
                let s = std::str::from_utf8(&data[start..*pos])
                    .map_err(|_| PyError::type_error("invalid utf-8 in pickle GLOBAL"))?
                    .to_string();
                *pos += 1; // skip '\n'
                s
            };
            // Resolve the global — for now, handle common cases
            match (module.as_str(), name.as_str()) {
                ("__builtin__" | "builtins", "iter") => {
                    // iter(...) will be handled by INST/REDUCE below
                    Ok(py_str("iter"))
                }
                ("__builtin__" | "builtins", "xrange" | "range") => {
                    // range(stop) or range(start, stop, step) — deserialized via REDUCE
                    Ok(py_str("range"))
                }
                _ => Err(PyError::type_error(format!(
                    "cannot resolve global {}.{} in pickle data",
                    module, name
                ))),
            }
        }
        _ => Err(PyError::type_error(format!(
            "unknown pickle marker byte: 0x{:02x}",
            marker
        ))),
    }
}
