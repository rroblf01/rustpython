use crate::object::*;
use std::collections::HashMap;

/// Decode the bytes payload of `array.__reduce_ex__`'s pickle tuple back into
/// f64-per-element storage (`PyArray` stores every element as `f64`
/// regardless of typecode — see `array_typecode_is_float`). `mformat_code`
/// is CPython's machine-format enum (`Modules/arraymodule.c`) describing the
/// on-disk width/endianness/signedness actually used when pickling, which is
/// independent of (but for every typecode this array module accepts,
/// consistent with) the array's own typecode.
fn decode_reconstructed_bytes(mformat_code: i64, bytes: &[u8]) -> Result<Vec<f64>, PyError> {
    let mut data = Vec::new();
    macro_rules! decode {
        ($ty:ty, $from_bytes:ident, $width:expr) => {{
            if bytes.len() % $width != 0 {
                return Err(PyError::value_error(
                    "bytes length not a multiple of item size".to_string(),
                ));
            }
            for chunk in bytes.chunks_exact($width) {
                let arr: [u8; $width] = chunk.try_into().unwrap();
                data.push(<$ty>::$from_bytes(arr) as f64);
            }
        }};
    }
    match mformat_code {
        0 => decode!(u8, from_le_bytes, 1),
        1 => decode!(i8, from_le_bytes, 1),
        2 => decode!(u16, from_le_bytes, 2),
        3 => decode!(u16, from_be_bytes, 2),
        4 => decode!(i16, from_le_bytes, 2),
        5 => decode!(i16, from_be_bytes, 2),
        6 => decode!(u32, from_le_bytes, 4),
        7 => decode!(u32, from_be_bytes, 4),
        8 => decode!(i32, from_le_bytes, 4),
        9 => decode!(i32, from_be_bytes, 4),
        10 => decode!(u64, from_le_bytes, 8),
        11 => decode!(u64, from_be_bytes, 8),
        12 => decode!(i64, from_le_bytes, 8),
        13 => decode!(i64, from_be_bytes, 8),
        14 => decode!(f32, from_le_bytes, 4),
        15 => decode!(f32, from_be_bytes, 4),
        16 => decode!(f64, from_le_bytes, 8),
        17 => decode!(f64, from_be_bytes, 8),
        18 => decode!(u16, from_le_bytes, 2),
        19 => decode!(u16, from_be_bytes, 2),
        20 => decode!(u32, from_le_bytes, 4),
        21 => decode!(u32, from_be_bytes, 4),
        _ => {
            return Err(PyError::value_error(format!(
                "invalid machine format code {mformat_code}"
            )));
        }
    }
    Ok(data)
}

pub fn create_array_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    d.insert_str(
        "_array_reconstructor",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_array_reconstructor".to_string(),
            func: |args| {
                if args.len() < 4 {
                    return Err(PyError::type_error(
                        "_array_reconstructor requires 4 arguments",
                    ));
                }
                let typecode_str = args[1].str();
                let typecode = typecode_str.chars().next().unwrap_or('b');
                let mformat_code = args[2].as_i64().ok_or_else(|| {
                    PyError::type_error("_array_reconstructor: mformat_code must be an int")
                })?;
                let bytes_borrowed = args[3].borrow();
                let bytes = match &*bytes_borrowed {
                    PyObject::Bytes(b) => b.clone(),
                    _ => {
                        return Err(PyError::type_error(
                            "_array_reconstructor: fourth argument must be bytes",
                        ));
                    }
                };
                drop(bytes_borrowed);
                let data = decode_reconstructed_bytes(mformat_code, &bytes)?;
                Ok(PyObjectRef::new(PyObject::Array(PyArray { typecode, data })))
            },
        }),
    );

    // Array type as a factory function
    d.insert_str(
        "array",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "array".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "array() requires at least 1 argument (typecode)",
                    ));
                }
                let typecode_str = args[0].str();
                if typecode_str.is_empty() {
                    return Err(PyError::value_error("empty typecode".to_string()));
                }
                let typecode = typecode_str.chars().next().unwrap();
                // Real Python's `array` module accepts all of `bBuhHiIlLqQfd` —
                // this previously only recognized 'i'/'f'/'d', raising
                // `ValueError: bad typecode` for anything else (e.g. `array
                // .array('B', ...)`, an extremely common "typed byte buffer"
                // idiom used throughout CPython's own test suite as setup/helper
                // code, not something specific to `array` itself). `PyArray`
                // stores every element as `f64` regardless of typecode (a
                // simplification — real per-typecode overflow/wraparound
                // semantics and `itemsize` aren't modeled), but that's already
                // true for the 'i' case this accepted before; broadening which
                // typecodes are ACCEPTED (and read back as `int` vs `float` per
                // `array_typecode_is_float` below) fixes the far more common
                // "construction rejected outright" failure mode.
                if !"bBuhHiIlLqQfdwu".contains(typecode) {
                    return Err(PyError::value_error(format!("bad typecode '{}'", typecode)));
                }
                let is_float = array_typecode_is_float(typecode);
                let is_unicode = typecode == 'w' || typecode == 'u';
                let mut data: Vec<f64> = Vec::new();
                if args.len() > 1 {
                    let init = &args[1];
                    // Only List/Tuple/Str(unicode) are handled by directly
                    // borrowing `init`'s backing storage; anything else falls
                    // through to the generic-iterator path below, which calls
                    // `builtin_next` on `init` itself (e.g. when `init` is a
                    // `list_iterator`). That reentrantly needs `init`'s own
                    // RefCell via `borrow_mut()`, so the initial `.borrow()`
                    // here MUST be dropped before falling through — holding
                    // it across the whole `match` (as this used to) panicked
                    // with "RefCell already borrowed" for any iterator/
                    // generator `initializer` argument.
                    enum Kind {
                        List,
                        Tuple,
                        StrUnicode,
                        Other,
                    }
                    let kind = {
                        let b = init.borrow();
                        match &*b {
                            PyObject::List(_) => Kind::List,
                            PyObject::Tuple(_) => Kind::Tuple,
                            PyObject::Str(_) if is_unicode => Kind::StrUnicode,
                            _ => Kind::Other,
                        }
                    };
                    match kind {
                        Kind::List => {
                            let b = init.borrow();
                            if let PyObject::List(items) = &*b {
                                for item in items {
                                    if is_float {
                                        data.push(item.as_f64().unwrap_or(0.0));
                                    } else if is_unicode {
                                        let s = item.str();
                                        let ch = s.chars().next().unwrap_or('\0') as u32 as f64;
                                        data.push(ch);
                                    } else {
                                        data.push(item.as_i64().unwrap_or(0) as f64);
                                    }
                                }
                            }
                        }
                        Kind::Tuple => {
                            let b = init.borrow();
                            if let PyObject::Tuple(items) = &*b {
                                for item in items {
                                    if is_float {
                                        data.push(item.as_f64().unwrap_or(0.0));
                                    } else if is_unicode {
                                        let s = item.str();
                                        let ch = s.chars().next().unwrap_or('\0') as u32 as f64;
                                        data.push(ch);
                                    } else {
                                        data.push(item.as_i64().unwrap_or(0) as f64);
                                    }
                                }
                            }
                        }
                        Kind::StrUnicode => {
                            let b = init.borrow();
                            if let PyObject::Str(s) = &*b {
                                for ch in s.chars() {
                                    data.push(ch as u32 as f64);
                                }
                            }
                        }
                        Kind::Other => {
                            let iter_obj = builtin_iter(&[init.clone()])?;
                            loop {
                                match builtin_next(&[iter_obj.clone()]) {
                                    Ok(item) => {
                                        if is_float {
                                            data.push(item.as_f64().unwrap_or(0.0));
                                        } else if is_unicode {
                                            let s = item.str();
                                            let ch = s.chars().next().unwrap_or('\0') as u32 as f64;
                                            data.push(ch);
                                        } else {
                                            data.push(item.as_i64().unwrap_or(0) as f64);
                                        }
                                    }
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                }
                Ok(PyObjectRef::new(PyObject::Array(PyArray {
                    typecode,
                    data,
                })))
            },
        }),
    );

    d
}
