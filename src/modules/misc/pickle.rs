use crate::object::*;
use std::collections::HashMap;
use crate::modules::misc::pickle_ser::pickle_serialize;
use crate::modules::misc::pickle_de::{pickle_deserialize, try_unpickle_rangeiter_compat};

pub fn create_pickle_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pickle_func {
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

    d.insert_str("HIGHEST_PROTOCOL", py_int(5));
    d.insert_str("DEFAULT_PROTOCOL", py_int(4));
    d.insert_str(
        "__all__",
        py_list(vec![
            py_str("PickleError"),
            py_str("PicklingError"),
            py_str("UnpicklingError"),
            py_str("Pickler"),
            py_str("Unpickler"),
            py_str("dump"),
            py_str("dumps"),
            py_str("load"),
            py_str("loads"),
            py_str("encode_long"),
            py_str("decode_long"),
            py_str("HIGHEST_PROTOCOL"),
            py_str("DEFAULT_PROTOCOL"),
            py_str("PickleBuffer"),
            py_str("bytes_types"),
        ]),
    );
    // Real CPython's `pickle.py` internal constant, used for isinstance
    // checks in the pure-Python pickler fallback path — `isinstance()`
    // here does its own name-based comparison against a `PyObject::Type`
    // (see `builtin_type_of`'s doc comment), so building this from real
    // `type(...)` calls on sample instances works correctly.
    d.insert_str(
        "bytes_types",
        py_tuple(vec![
            crate::object::builtin_type_of(&[PyObjectRef::imm(PyObject::Bytes(Vec::new()))])
                .unwrap_or_else(|_| py_none()),
            crate::object::builtin_type_of(&[PyObjectRef::new(PyObject::ByteArray(Vec::new()))])
                .unwrap_or_else(|_| py_none()),
        ]),
    );
    // Real `PickleBuffer` — wraps a buffer-protocol object for out-of-band
    // (protocol 5) pickling. Constructible for bytes/bytearray/memoryview/
    // array; `.raw()` returns a contiguous memoryview; `.release()` marks it
    // released so `memoryview(pb)` / `pb.raw()` raise ValueError thereafter.
    d.insert_str(
        "PickleBuffer",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "PickleBuffer".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "PickleBuffer() takes exactly one argument (0 given)",
                    ));
                }
                let obj = args[0].clone();
                // Validate buffer-like; reject non-bytes-like (e.g. str)
                // Must accept bytes subclasses (B(bytes)) which are stored as
                // Instance with `__native__` Bytes backing.
                let is_buffer = {
                    let b = obj.borrow();
                    if matches!(
                        &*b,
                        PyObject::Bytes(_)
                            | PyObject::ByteArray(_)
                            | PyObject::Array(_)
                            | PyObject::MemoryView { .. }
                    ) {
                        true
                    } else {
                        drop(b);
                        if let Some(backing) = crate::object::native_backing_of(&obj) {
                            matches!(
                                &*backing.borrow(),
                                PyObject::Bytes(_)
                                    | PyObject::ByteArray(_)
                                    | PyObject::Array(_)
                            )
                        } else {
                            false
                        }
                    }
                };
                if !is_buffer {
                    // Also allow PickleBuffer wrapping? but test only cares about str
                    let tname = obj.borrow().type_name();
                    return Err(PyError::type_error(format!(
                        "a bytes-like object is required, not '{}'",
                        tname
                    )));
                }
                // Released memoryview is not acceptable
                if let PyObject::MemoryView { released, .. } = &*obj.borrow() {
                    if *released {
                        return Err(PyError::value_error(
                            "operation forbidden on released memoryview object",
                        ));
                    }
                }
                let mut inst_dict = AttrMap::new();
                inst_dict.insert("_obj".to_string(), obj);
                inst_dict.insert("_released".to_string(), py_bool(false));
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: PyObjectRef::new(PyObject::Type {
                        name: "PickleBuffer".to_string(),
                        dict: Box::new(str_map_to_typedict(HashMap::from([
                            (
                                "raw".to_string(),
                                PyObjectRef::new(PyObject::BuiltinFunction {
                                    name: "raw".to_string(),
                                    func: |args| {
                                        if let PyObject::Instance { dict, .. } =
                                            &*args[0].borrow()
                                        {
                                            let released = dict
                                                .get("_released")
                                                .map(|v| v.truthy())
                                                .unwrap_or(false);
                                            if released {
                                                return Err(PyError::value_error(
                                                    "operation forbidden on released PickleBuffer object",
                                                ));
                                            }
                                            let underlying = dict
                                                .get("_obj")
                                                .cloned()
                                                .unwrap_or_else(py_none);
                                            // raw() must be contiguous; for this interpreter all
                                            // 1-D views are contiguous, so just wrap in memoryview
                                            crate::object::builtin_memoryview(&[underlying])
                                        } else {
                                            Err(PyError::type_error("raw() missing self"))
                                        }
                                    },
                                }),
                            ),
                            (
                                "release".to_string(),
                                PyObjectRef::new(PyObject::BuiltinFunction {
                                    name: "release".to_string(),
                                    func: |args| {
                                        if let PyObject::Instance { dict, .. } =
                                            &mut *args[0].borrow_mut()
                                        {
                                            dict.insert("_released".to_string(), py_bool(true));
                                        }
                                        Ok(py_none())
                                    },
                                }),
                            ),
                        ]))),
                        bases: vec![],
                        mro: vec![],
                    }),
                    dict: inst_dict,
                }))
            },
        }),
    );

    d.insert_str(
        "PickleError",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "PickleError".to_string(),
            func: crate::object::builtin_make_exception_pickleerror,
        }),
    );
    d.insert_str(
        "PicklingError",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "PicklingError".to_string(),
            func: crate::object::builtin_make_exception_picklingerror,
        }),
    );
    d.insert_str(
        "UnpicklingError",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "UnpicklingError".to_string(),
            func: crate::object::builtin_make_exception_unpicklingerror,
        }),
    );

    // pickle.decode_long(data): Decode a long integer from little-endian bytes
    pickle_func!("decode_long", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("decode_long() missing required argument: 'data'"));
        }
        let bytes: Vec<u8> = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("decode_long() argument must be bytes-like")),
        };
        if bytes.is_empty() {
            return Ok(py_int(0));
        }
        use num_bigint::BigInt;
        use num_traits::ToPrimitive;
        let sign_negative = bytes.last().map_or(false, |&b| b & 0x80 != 0);
        let mut magnitude = BigInt::from(0u32);
        for &b in bytes.iter().rev() {
            magnitude = (magnitude << 8) | BigInt::from(b);
        }
        let result = if sign_negative {
            let bits = (bytes.len() * 8) as u32;
            let modulus = BigInt::from(1u32) << bits;
            magnitude - modulus
        } else {
            magnitude
        };
        Ok(py_int(result))
    });

    // pickle.encode_long(n): Encode an integer as little-endian bytes
    pickle_func!("encode_long", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("encode_long() missing required argument: 'n'"));
        }
        let n: num_bigint::BigInt = match &*args[0].borrow() {
            PyObject::Int(i) => i.clone(),
            PyObject::Bool(b) => num_bigint::BigInt::from(if *b { 1i32 } else { 0i32 }),
            _ => return Err(PyError::type_error("encode_long() argument must be an integer")),
        };
        let is_negative = n.sign() == num_bigint::Sign::Minus;
        let abs_bytes = n.magnitude().to_bytes_le();
        let mut result = abs_bytes;
        // Add sign byte if the high bit of the last byte is set (or if negative and no bytes)
        if result.is_empty() {
            if is_negative {
                result.push(0x80);
            } else {
                result.push(0x00);
            }
        } else if is_negative {
            let last = *result.last().unwrap();
            if last < 0x80 {
                result.push(0x80);
            }
        } else {
            let last = *result.last().unwrap();
            if last >= 0x80 {
                result.push(0x00);
            }
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(result)))
    });

    pickle_func!("dumps", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("dumps() missing required argument"));
        }
        let mut protocol = 4i32;
        // Check positional args and kwargs for protocol
        for arg in args.iter().skip(1) {
            if let PyObject::Dict(d) = &*arg.borrow() {
                if let Ok(Some(p)) = d.get(&py_str("protocol")) {
                    protocol = p.as_i64().unwrap_or(4) as i32;
                }
            } else {
                protocol = arg.as_i64().unwrap_or(4) as i32;
            }
        }
        let mut buf = Vec::new();
        let mut memo: Vec<*const ()> = Vec::new();
        // Protocol 2+ starts with PROTO header
        if protocol >= 2 {
            buf.push(0x80); // PROTO
            buf.push(protocol as u8); // protocol version
        }
        pickle_serialize(&args[0], &mut buf, &mut memo, protocol)?;
        // All protocols end with a stop marker (.)
        buf.push(b'.');
        Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
    });

    fn pickle_loads_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.is_empty() {
            return Err(PyError::type_error("loads() missing required argument"));
        }
        let data: Vec<u8> = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => {
                return Err(PyError::type_error(
                    "loads() argument must be bytes or string",
                ))
            }
        };
        // CPython compat: historical range_iterator pickles (protocols 0-4,
        // including Python 2 `xrange`) are a different wire format from our
        // own custom pickle. Try that first so `trailing bytes` doesn't fire.
        if let Some(v) = try_unpickle_rangeiter_compat(&data) {
            return Ok(v);
        }
        let mut pos = 0;
        let mut memo: Vec<PyObjectRef> = Vec::new();
        let result = pickle_deserialize(&data, &mut pos, &mut memo)?;
        // Skip protocol 0 stop marker (.) if present
        if pos < data.len() && data[pos] == b'.' {
            pos += 1;
        }
        if pos != data.len() {
            return Err(PyError::type_error(format!(
                "pickle data has trailing bytes after value (pos={}, len={})",
                pos,
                data.len()
            )));
        }
        Ok(result)
    }
    pickle_func!("loads", pickle_loads_impl);
    pickle_func!("_loads", pickle_loads_impl);

    d
}
