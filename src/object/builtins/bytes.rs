use super::*;

pub fn builtin_bytes(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())))
    } else {
        if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
            if let Some(rc) = target.upgrade() {
                return builtin_bytes(&[PyObjectRef::Imm(rc)]);
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }
        {
            let is_instance = matches!(&*args[0].borrow(), PyObject::Instance { .. });
            if is_instance {
                let f = {
                    let b = args[0].borrow();
                    if let PyObject::Instance { typ, .. } = &*b {
                        lookup_dunder_via_mro(typ, "__bytes__")
                    } else { None }
                };
                if let Some(f) = f {
                    return call_bound_method(f, args[0].clone(), vec![]);
                }
            }
        }
        // Buffer protocol: try memoryview first (e.g. bytes(MyBuffer()) where MyBuffer defines __buffer__)
        {
            let obj = args[0].clone();
            if let Ok(mv) = crate::object::builtin_memoryview(&[obj.clone()]) {
                if let Ok(bytes) = crate::object::mv_tobytes(&mv) {
                    return Ok(PyObjectRef::imm(PyObject::Bytes(bytes)));
                }
            }
        }
        // PickleBuffer and memoryview are bytes-like via buffer protocol
        {
            let b = args[0].borrow();
            if let PyObject::Instance { typ, dict } = &*b {
                let is_pb = if let PyObject::Type { name, .. } = &*typ.borrow() {
                    name == "PickleBuffer"
                } else {
                    false
                };
                if is_pb {
                    let released = dict
                        .get("_released")
                        .map(|v| v.truthy())
                        .unwrap_or(false);
                    if released {
                        return Err(PyError::value_error(
                            "operation forbidden on released PickleBuffer object",
                        ));
                    }
                    let underlying = dict.get("_obj").cloned().unwrap_or_else(py_none);
                    drop(b);
                    return builtin_bytes(&[underlying]);
                }
            }
            if let PyObject::MemoryView { released, .. } = &*b {
                if *released {
                    return Err(PyError::value_error(
                        "operation forbidden on released memoryview object",
                    ));
                }
                // fall through to dedicated memoryview handling below after drop
            }
        }
        // memoryview -> bytes is a direct tobytes copy
        if matches!(&*args[0].borrow(), PyObject::MemoryView { .. }) {
            let bytes = crate::object::mv_tobytes(&args[0])?;
            return Ok(PyObjectRef::imm(PyObject::Bytes(bytes)));
        }
        let obj = args[0].borrow();
        match &*obj {
            // Same fix as `bytearray(n)` above: `bytes(n)` zero-fills a
            // buffer of length `n`, it doesn't wrap `n` as a single byte
            // value.
            PyObject::Int(i) => {
                let n = i
                    .to_i64()
                    .ok_or_else(|| PyError::value_error("bytes() argument must be non-negative"))?;
                if n < 0 {
                    return Err(PyError::value_error(
                        "bytes() argument must be non-negative",
                    ));
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(vec![0u8; n as usize])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::imm(PyObject::Bytes(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Set(items) | PyObject::FrozenSet(items) => {
                let mut result = Vec::new();
                for item in items.to_vec() {
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            _ => {
                drop(obj);
                // Same fix as `builtin_list`'s matching site: propagate
                // `builtin_iter`'s error as-is rather than replacing it with
                // a generic message (swallowing a real exception raised
                // from inside a custom `__iter__`).
                let it = builtin_iter(&[args[0].clone()])?;
                let mut result = Vec::new();
                loop {
                    let item = match builtin_next(&[it.clone()]) {
                        Ok(val) => val,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    };
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
        }
    }
}

/// bytes.fromhex(string) -> bytes
///
/// Create a bytes object from a string of hexadecimal digits.
pub fn builtin_bytes_fromhex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "bytes.fromhex() takes exactly 1 argument (0 given)",
        ));
    }
    let s = args[0].str();
    // Remove spaces (CPython allows spaces in the hex string)
    let s = s.replace(' ', "");
    if s.len() % 2 != 0 {
        return Err(PyError::value_error("hex string must be of even length"));
    }
    let mut result = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hex_pair = std::str::from_utf8(chunk)
            .map_err(|_| PyError::value_error("non-hexadecimal number found"))?;
        let byte = u8::from_str_radix(hex_pair, 16).map_err(|_| {
            PyError::value_error(format!(
                "non-hexadecimal number found in fromhex() arg at position {}",
                s.find(hex_pair).unwrap_or(0)
            ))
        })?;
        result.push(byte);
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
}

pub fn builtin_bytearray(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::new(PyObject::ByteArray(Vec::new())))
    } else {
        let obj = args[0].borrow();
        match &*obj {
            // Real `bytearray(n)` (a single non-negative int argument)
            // creates a zero-filled buffer of length `n` — NOT a
            // single-element buffer holding the byte value `n` (that's
            // `bytes([n])`, a completely different construction). This
            // matched the length-1 anti-pattern instead, silently breaking
            // the extremely common "pre-allocate an I/O buffer"
            // idiom — found via `struct.pack_into`'s own doctest-style
            // idiom `bytearray(10)`.
            PyObject::Int(i) => {
                let n = i.to_i64().ok_or_else(|| {
                    PyError::value_error("bytearray() argument must be non-negative")
                })?;
                if n < 0 {
                    return Err(PyError::value_error(
                        "bytearray() argument must be non-negative",
                    ));
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(vec![0u8; n as usize])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::new(PyObject::ByteArray(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytearray() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytearray() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytearray() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytearray() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytearray() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytearray() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Range { .. } => {
                // Any iterable of ints (range, generators, custom __iter__)
                // is valid (test_pprint: bytearray(range(5))).
                drop(obj);
                let it = builtin_iter(&[args[0].clone()])?;
                let mut result = Vec::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(item) => {
                            let n = item.as_i64().ok_or_else(|| {
                                PyError::value_error("bytearray() requires int in range 0-255")
                            })?;
                            if n < 0 || n > 255 {
                                return Err(PyError::value_error(
                                    "bytearray() requires int in range 0-255",
                                ));
                            }
                            result.push(n as u8);
                        }
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            _ => Err(PyError::type_error(format!(
                "cannot convert '{}' to bytearray",
                obj.type_name()
            ))),
        }
    }
}
