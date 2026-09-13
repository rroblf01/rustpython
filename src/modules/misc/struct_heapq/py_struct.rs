use crate::object::*;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::collections::HashMap;
// ---------------------------------------------------------------------------
// struct module — real pack/unpack, replacing a former near-total stub that
// ignored format codes entirely (every value was truncated to a single byte
// regardless of its real width, and `unpack` just returned each raw byte as
// its own int). Found via CPython's own `test_struct.py`: out-of-range
// integers silently wrapped instead of raising `struct.error`
// (`test_issue98248`), and multi-byte/float round-trips were simply wrong.
// Scope: standard-size b/B/h/H/i/I/l/L/q/Q/n/N/f/d/F/D/?/c/s/p/x codes with
// </>/!/=/@ byte-order prefixes, all treated as standard (no native
// alignment/padding — `@`/`=` behave like `<` on this little-endian target).
// `F`/`D` pack/unpack a Python `complex` as two consecutive floats
// (real, imag) of the matching width, matching CPython's `_struct.c`.
// Deliberately NOT implemented: `e` (half-float) format code, a real
// `Struct` class — flagged as a smaller remaining gap.
// ---------------------------------------------------------------------------

fn struct_error(msg: impl Into<String>) -> PyError {
    let msg = msg.into();
    let exc = PyObjectRef::new(PyObject::Exception {
        typ: "error".to_string(),
        args: vec![py_str(&msg)],
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra: None,
    });
    PyError::Exception(msg, exc)
}

#[derive(Clone, Copy, PartialEq)]
enum StructByteOrder {
    Little,
    Big,
}

struct StructFmtItem {
    code: char,
    count: usize,
}

/// Real `struct.pack`/`unpack`/etc. accept the format as EITHER `str` or
/// `bytes` (CPython's `_struct.c` decodes a bytes format ascii-wise before
/// parsing). Call sites here previously used the generic `.str()` helper
/// unconditionally, which for a `bytes` argument returns its Python `repr`
/// (`"b'<4s4H2LH'"`, quotes/prefix and all) rather than decoding it — so
/// any caller passing a bytes format string (e.g. `Lib/zipfile`'s own
/// `structEndArchive = b"<4s4H2LH"`) got a mangled format string and a
/// nonsensical `TypeError` deep in the packer instead of packing
/// correctly. Mirrors the pattern `calcsize` already used correctly below.
fn fmt_to_str(v: &PyObjectRef) -> PyResult<String> {
    match &*v.borrow() {
        PyObject::Str(s) => Ok(s.to_string()),
        PyObject::Bytes(b) => std::str::from_utf8(b)
            .map(|s| s.to_string())
            .map_err(|_| PyError::type_error("struct format requires a string or bytes")),
        _ => Ok(v.str()),
    }
}

fn parse_struct_format(fmt: &str) -> PyResult<(StructByteOrder, Vec<StructFmtItem>)> {
    let mut chars = fmt.chars().peekable();
    let mut order = StructByteOrder::Little;
    if let Some(&c) = chars.peek() {
        match c {
            '@' | '=' | '<' => {
                order = StructByteOrder::Little;
                chars.next();
            }
            '>' | '!' => {
                order = StructByteOrder::Big;
                chars.next();
            }
            _ => {}
        }
    }
    let mut items = Vec::new();
    while let Some(c) = chars.next() {
        if c == ' ' {
            continue;
        }
        if c.is_ascii_digit() {
            let mut n = String::from(c);
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    n.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            let count: usize = n
                .parse()
                .map_err(|_| struct_error("bad repeat count in struct format"))?;
            let code = chars
                .next()
                .ok_or_else(|| struct_error("repeat count given without format specifier"))?;
            items.push(StructFmtItem { code, count });
        } else {
            items.push(StructFmtItem { code: c, count: 1 });
        }
    }
    Ok((order, items))
}

fn struct_code_size(code: char) -> PyResult<usize> {
    Ok(match code {
        'x' | 'c' | 'b' | 'B' | '?' | 's' | 'p' => 1,
        'h' | 'H' => 2,
        'i' | 'I' | 'l' | 'L' | 'f' => 4,
        'q' | 'Q' | 'n' | 'N' | 'd' | 'P' => 8,
        // Complex numbers: 'F' = 2x float (real, imag), 'D' = 2x double —
        // real CPython's _struct.c supports these (`nu_float_complex`/
        // `nu_double_complex` et al.), packing/unpacking real then imag as
        // consecutive same-width floats in the format's byte order.
        'F' => 8,
        'D' => 16,
        _ => {
            return Err(struct_error(format!(
                "bad char in struct format: '{}'",
                code
            )))
        }
    })
}

fn struct_calcsize(fmt: &str) -> PyResult<usize> {
    let (_, items) = parse_struct_format(fmt)?;
    let mut size = 0usize;
    for item in &items {
        let unit = struct_code_size(item.code)?;
        match item.code {
            's' | 'p' => size += item.count,
            _ => size += unit * item.count,
        }
    }
    Ok(size)
}

fn struct_pack_arg_bigint(val: &PyObjectRef) -> PyResult<BigInt> {
    {
        let b = val.borrow();
        match &*b {
            PyObject::Int(i) => return Ok(i.clone()),
            PyObject::Bool(bv) => return Ok(BigInt::from(*bv as i64)),
            _ => {}
        }
    }
    // Real Python's `struct.pack` accepts ANY object implementing
    // `__index__` for its integer format codes, not just a literal `int`/
    // `bool` — this was missing entirely, so a custom `Indexable` class
    // (`def __index__(self): return self._value`) raised a generic "not an
    // integer" error instead of packing successfully. `to_index` (already
    // used by `range()`/slicing for the same protocol) does exactly this —
    // reused here rather than reimplementing the dispatch. A plain `TypeError`
    // propagating from a missing/bad `__index__` is fine as-is: real
    // CPython's own `struct.pack` raises bare `TypeError` for exactly these
    // cases too (confirmed via `test_struct.py`'s own
    // `assertRaises((TypeError, struct.error), ...)` — either is accepted).
    to_index(val)
}

fn struct_check_bounds(code: char, n: &BigInt) -> PyResult<()> {
    let (lo, hi): (BigInt, BigInt) = match code {
        'b' => (BigInt::from(-128), BigInt::from(127)),
        'B' => (BigInt::from(0), BigInt::from(255)),
        'h' => (BigInt::from(-32768), BigInt::from(32767)),
        'H' => (BigInt::from(0), BigInt::from(65535)),
        'i' | 'l' => (BigInt::from(i32::MIN), BigInt::from(i32::MAX)),
        'I' | 'L' => (BigInt::from(0u32), BigInt::from(u32::MAX)),
        'q' | 'n' => (BigInt::from(i64::MIN), BigInt::from(i64::MAX)),
        'Q' | 'N' => (BigInt::from(0u64), BigInt::from(u64::MAX)),
        _ => return Ok(()),
    };
    if n < &lo || n > &hi {
        return Err(struct_error(format!(
            "'{}' format requires {} <= number <= {}",
            code, lo, hi
        )));
    }
    Ok(())
}

fn struct_push_bytes(out: &mut Vec<u8>, order: StructByteOrder, le: &[u8], be: &[u8]) {
    match order {
        StructByteOrder::Little => out.extend_from_slice(le),
        StructByteOrder::Big => out.extend_from_slice(be),
    }
}

fn struct_pack_one(
    out: &mut Vec<u8>,
    order: StructByteOrder,
    code: char,
    count: usize,
    val: &PyObjectRef,
) -> PyResult<()> {
    match code {
        '?' => {
            out.push(if val.truthy() { 1 } else { 0 });
        }
        'c' => {
            let b = val.borrow();
            match &*b {
                PyObject::Bytes(data) if data.len() == 1 => out.push(data[0]),
                PyObject::Bytes(_) => {
                    return Err(struct_error(
                        "char format requires a bytes object of length 1",
                    ))
                }
                _ => {
                    return Err(struct_error(
                        "argument for 'c' must be a bytes object of length 1",
                    ))
                }
            }
        }
        's' | 'p' => {
            let data = arg_bytes(val).ok_or_else(|| {
                struct_error(format!("argument for '{}' must be a bytes object", code))
            })?;
            let mut field = vec![0u8; count];
            if code == 's' {
                let n = data.len().min(count);
                field[..n].copy_from_slice(&data[..n]);
            } else if count > 0 {
                let maxlen = (count - 1).min(255);
                let n = data.len().min(maxlen);
                field[0] = n as u8;
                field[1..1 + n].copy_from_slice(&data[..n]);
            }
            out.extend_from_slice(&field);
        }
        'f' => {
            let f = val
                .as_f64()
                .ok_or_else(|| struct_error("required argument is not a float"))?;
            let v = f as f32;
            // Real CPython (`Objects/floatobject.c`'s `PyFloat_Pack4`) raises
            // `OverflowError` — not `struct.error` — when a finite f64 is too
            // large to represent as f32 (rounds to +/-inf on the narrowing
            // cast). A value that was ALREADY +/-inf (or NaN) in the input is
            // fine to pack as-is; only newly-created infinities from an
            // in-range-for-f64-but-not-f32 magnitude are the overflow case.
            if v.is_infinite() && f.is_finite() {
                return Err(PyError::overflow_error(
                    "float too large to pack with f format",
                ));
            }
            struct_push_bytes(out, order, &v.to_le_bytes(), &v.to_be_bytes());
        }
        'd' => {
            let f = val
                .as_f64()
                .ok_or_else(|| struct_error("required argument is not a float"))?;
            struct_push_bytes(out, order, &f.to_le_bytes(), &f.to_be_bytes());
        }
        'F' | 'D' => {
            let (re, im) = as_complex_parts(&val.borrow())
                .ok_or_else(|| struct_error("required argument is not a complex"))?;
            if code == 'F' {
                let (re, im) = (re as f32, im as f32);
                struct_push_bytes(out, order, &re.to_le_bytes(), &re.to_be_bytes());
                struct_push_bytes(out, order, &im.to_le_bytes(), &im.to_be_bytes());
            } else {
                struct_push_bytes(out, order, &re.to_le_bytes(), &re.to_be_bytes());
                struct_push_bytes(out, order, &im.to_le_bytes(), &im.to_be_bytes());
            }
        }
        'b' | 'B' | 'h' | 'H' | 'i' | 'I' | 'l' | 'L' | 'q' | 'n' | 'Q' | 'N' => {
            let n = struct_pack_arg_bigint(val)?;
            struct_check_bounds(code, &n)?;
            match code {
                'b' => out.push(n.to_i64().unwrap() as i8 as u8),
                'B' => out.push(n.to_i64().unwrap() as u8),
                'h' => {
                    let v = n.to_i64().unwrap() as i16;
                    struct_push_bytes(out, order, &v.to_le_bytes(), &v.to_be_bytes());
                }
                'H' => {
                    let v = n.to_i64().unwrap() as u16;
                    struct_push_bytes(out, order, &v.to_le_bytes(), &v.to_be_bytes());
                }
                'i' | 'l' => {
                    let v = n.to_i64().unwrap() as i32;
                    struct_push_bytes(out, order, &v.to_le_bytes(), &v.to_be_bytes());
                }
                'I' | 'L' => {
                    let v = n.to_i64().unwrap() as u32;
                    struct_push_bytes(out, order, &v.to_le_bytes(), &v.to_be_bytes());
                }
                'q' | 'n' => {
                    let v = n.to_i64().unwrap();
                    struct_push_bytes(out, order, &v.to_le_bytes(), &v.to_be_bytes());
                }
                'Q' | 'N' => {
                    let v = n.to_u64().unwrap();
                    struct_push_bytes(out, order, &v.to_le_bytes(), &v.to_be_bytes());
                }
                _ => unreachable!(),
            }
        }
        _ => {
            return Err(struct_error(format!(
                "bad char in struct format: '{}'",
                code
            )))
        }
    }
    Ok(())
}

fn struct_pack_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "pack() missing required argument: 'format'",
        ));
    }
    let fmt = fmt_to_str(&args[0])?;
    let (order, items) = parse_struct_format(&fmt)?;
    let mut out = Vec::new();
    let mut arg_idx = 1usize;
    for item in &items {
        match item.code {
            'x' => {
                for _ in 0..item.count {
                    out.push(0u8);
                }
            }
            's' | 'p' => {
                if arg_idx >= args.len() {
                    return Err(struct_error("pack expected more arguments"));
                }
                struct_pack_one(&mut out, order, item.code, item.count, &args[arg_idx])?;
                arg_idx += 1;
            }
            _ => {
                for _ in 0..item.count.max(1) {
                    if arg_idx >= args.len() {
                        return Err(struct_error("pack expected more arguments"));
                    }
                    struct_pack_one(&mut out, order, item.code, 1, &args[arg_idx])?;
                    arg_idx += 1;
                }
            }
        }
    }
    if arg_idx != args.len() {
        return Err(struct_error("pack expected fewer arguments"));
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(out)))
}

fn struct_decode_scalar(order: StructByteOrder, code: char, field: &[u8]) -> PyResult<PyObjectRef> {
    let widen = |le: bool| -> u64 {
        let mut arr = [0u8; 8];
        if le {
            arr[..field.len()].copy_from_slice(field);
            u64::from_le_bytes(arr)
        } else {
            arr[8 - field.len()..].copy_from_slice(field);
            u64::from_be_bytes(arr)
        }
    };
    let le = order == StructByteOrder::Little;
    Ok(match code {
        'b' => py_int(field[0] as i8 as i64),
        'B' => py_int(field[0] as i64),
        '?' => py_bool(field[0] != 0),
        'c' => PyObjectRef::imm(PyObject::Bytes(vec![field[0]])),
        'h' => py_int(widen(le) as u16 as i16 as i64),
        'H' => py_int(widen(le) as u16 as i64),
        'i' | 'l' => py_int(widen(le) as u32 as i32 as i64),
        'I' | 'L' => py_int(widen(le) as u32 as i64),
        'q' | 'n' => py_int(widen(le) as i64),
        'Q' | 'N' => py_int(widen(le)),
        'f' => py_float(f32::from_bits(widen(le) as u32) as f64),
        'd' => py_float(f64::from_bits(widen(le))),
        'F' => {
            let widen_sub = |sub: &[u8]| -> u64 {
                let mut arr = [0u8; 8];
                if le {
                    arr[..sub.len()].copy_from_slice(sub);
                    u64::from_le_bytes(arr)
                } else {
                    arr[8 - sub.len()..].copy_from_slice(sub);
                    u64::from_be_bytes(arr)
                }
            };
            let re = f32::from_bits(widen_sub(&field[0..4]) as u32) as f64;
            let im = f32::from_bits(widen_sub(&field[4..8]) as u32) as f64;
            PyObjectRef::imm(PyObject::Complex(re, im))
        }
        'D' => {
            let widen_sub = |sub: &[u8]| -> u64 {
                let mut arr = [0u8; 8];
                if le {
                    arr.copy_from_slice(sub);
                    u64::from_le_bytes(arr)
                } else {
                    arr.copy_from_slice(sub);
                    u64::from_be_bytes(arr)
                }
            };
            let re = f64::from_bits(widen_sub(&field[0..8]));
            let im = f64::from_bits(widen_sub(&field[8..16]));
            PyObjectRef::imm(PyObject::Complex(re, im))
        }
        _ => {
            return Err(struct_error(format!(
                "bad char in struct format: '{}'",
                code
            )))
        }
    })
}

fn struct_unpack_buf(fmt: &str, buf: &[u8]) -> PyResult<Vec<PyObjectRef>> {
    let (order, items) = parse_struct_format(fmt)?;
    let total = struct_calcsize(fmt)?;
    if buf.len() != total {
        return Err(struct_error(format!(
            "unpack requires a buffer of {} bytes",
            total
        )));
    }
    let mut results = Vec::new();
    let mut pos = 0usize;
    for item in &items {
        match item.code {
            'x' => {
                pos += item.count;
            }
            's' => {
                let end = pos + item.count;
                if end > buf.len() {
                    return Err(struct_error(format!(
                        "unpack requires a buffer of {} bytes",
                        end
                    )));
                }
                results.push(PyObjectRef::imm(PyObject::Bytes(buf[pos..end].to_vec())));
                pos = end;
            }
            'p' => {
                let end = pos + item.count;
                if end > buf.len() {
                    return Err(struct_error(format!(
                        "unpack requires a buffer of {} bytes",
                        end
                    )));
                }
                let field = &buf[pos..end];
                let data = if field.is_empty() {
                    Vec::new()
                } else {
                    let n = (field[0] as usize).min(field.len() - 1);
                    field[1..1 + n].to_vec()
                };
                results.push(PyObjectRef::imm(PyObject::Bytes(data)));
                pos = end;
            }
            _ => {
                let unit = struct_code_size(item.code)?;
                for _ in 0..item.count.max(1) {
                    let end = pos + unit;
                    if end > buf.len() {
                        return Err(struct_error(format!(
                            "unpack requires a buffer of {} bytes",
                            end
                        )));
                    }
                    results.push(struct_decode_scalar(order, item.code, &buf[pos..end])?);
                    pos = end;
                }
            }
        }
    }
    Ok(results)
}

fn struct_unpack_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "unpack() requires format string and buffer",
        ));
    }
    let fmt = fmt_to_str(&args[0])?;
    let buf =
        arg_bytes(&args[1]).ok_or_else(|| PyError::type_error("unpack() arg 2 must be bytes"))?;
    let values = struct_unpack_buf(&fmt, &buf)?;
    Ok(PyObjectRef::imm(PyObject::Tuple(values)))
}

fn struct_unpack_from_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "unpack_from() requires format string and buffer",
        ));
    }
    let fmt = fmt_to_str(&args[0])?;
    let buf = arg_bytes(&args[1])
        .ok_or_else(|| PyError::type_error("unpack_from() arg 2 must be bytes"))?;
    let offset = if args.len() > 2 {
        args[2].as_i64().unwrap_or(0)
    } else {
        0
    };
    let offset = if offset < 0 {
        (buf.len() as i64 + offset).max(0) as usize
    } else {
        offset as usize
    };
    let size = struct_calcsize(&fmt)?;
    if offset + size > buf.len() {
        return Err(struct_error(format!(
            "unpack_from requires a buffer of at least {} bytes for unpacking {} bytes at offset {} (actual buffer size is {})",
            offset + size, size, offset, buf.len()
        )));
    }
    let values = struct_unpack_buf(&fmt, &buf[offset..offset + size])?;
    Ok(PyObjectRef::imm(PyObject::Tuple(values)))
}

fn struct_pack_into_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "pack_into() requires format, buffer, offset",
        ));
    }
    let fmt = fmt_to_str(&args[0])?;
    let offset = if args.len() > 2 {
        args[2].as_i64().unwrap_or(0)
    } else {
        0
    };
    let size = struct_calcsize(&fmt)?;
    let packed = {
        let mut rest = vec![args[0].clone()];
        rest.extend_from_slice(&args[3.min(args.len())..]);
        struct_pack_impl(&rest)?
    };
    let packed_bytes = arg_bytes(&packed).unwrap();
    let mut buf_obj = args[1].borrow_mut();
    match &mut *buf_obj {
        PyObject::ByteArray(data) => {
            let offset = if offset < 0 {
                (data.len() as i64 + offset).max(0) as usize
            } else {
                offset as usize
            };
            if offset + size > data.len() {
                return Err(struct_error(format!(
                    "pack_into requires a buffer of at least {} bytes for packing {} bytes at offset {} (actual buffer size is {})",
                    offset + size, size, offset, data.len()
                )));
            }
            data[offset..offset + size].copy_from_slice(&packed_bytes);
            Ok(py_none())
        }
        _ => Err(PyError::type_error(
            "pack_into() argument must be a mutable buffer (bytearray)",
        )),
    }
}

fn struct_iter_unpack_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "iter_unpack() requires format string and buffer",
        ));
    }
    let fmt = fmt_to_str(&args[0])?;
    let buf = arg_bytes(&args[1])
        .ok_or_else(|| PyError::type_error("iter_unpack() arg 2 must be bytes"))?;
    let unit = struct_calcsize(&fmt)?;
    if unit == 0 {
        return Err(struct_error(
            "cannot iteratively unpack with a struct of length 0",
        ));
    }
    if buf.len() % unit != 0 {
        return Err(struct_error(format!(
            "iterative unpacking requires a buffer of a multiple of {} bytes",
            unit
        )));
    }
    let mut tuples = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let values = struct_unpack_buf(&fmt, &buf[pos..pos + unit])?;
        tuples.push(PyObjectRef::imm(PyObject::Tuple(values)));
        pos += unit;
    }
    builtin_iter(&[py_list(tuples)])
}

pub fn create_struct_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! s_func {
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

    s_func!("calcsize", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("calcsize() missing required argument"));
        }
        // struct.calcsize accepts both str and bytes as format string
        let fmt: String = match &*args[0].borrow() {
            PyObject::Str(s) => s.to_string(),
            PyObject::Bytes(b) => match std::str::from_utf8(b) {
                Ok(s) => s.to_string(),
                Err(_) => return Err(PyError::type_error("struct format requires a string or bytes")),
            },
            _ => args[0].str(),
        };
        Ok(py_int(struct_calcsize(&fmt)? as i64))
    });

    s_func!("pack", struct_pack_impl);
    s_func!("unpack", struct_unpack_impl);
    s_func!("unpack_from", struct_unpack_from_impl);
    s_func!("pack_into", struct_pack_into_impl);
    s_func!("iter_unpack", struct_iter_unpack_impl);

    // struct.Struct class — used by zipfile and other modules
    // Simple wrapper that stores the format string and delegates to module functions
    {
        let mut struct_type_dict: std::collections::HashMap<String, PyObjectRef> = std::collections::HashMap::new();
        struct_type_dict.insert("__doc__".to_string(), py_str("Struct(format)"));
        struct_type_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("Struct() requires a format argument"));
                }
                let fmt = match &*args[1].borrow() {
                    PyObject::Str(s) => s.to_string(),
                    PyObject::Bytes(b) => match std::str::from_utf8(b) {
                        Ok(s) => s.to_string(),
                        Err(_) => return Err(PyError::type_error("Struct format requires a string or bytes")),
                    },
                    _ => args[1].str(),
                };
                // Store format in instance dict
                if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                    dict.insert("format".to_string(), py_str(&fmt));
                    dict.insert("size".to_string(), py_int(struct_calcsize(&fmt)? as i64));
                }
                Ok(py_none())
            },
        }));
        struct_type_dict.insert("calcsize".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "calcsize".to_string(),
            func: |args| {
                if let Some(inst) = args.first() {
                    if let PyObject::Instance { dict, .. } = &*inst.borrow() {
                        if let Some(size) = dict.get("size") {
                            return Ok(size.clone());
                        }
                    }
                }
                Err(PyError::type_error("Struct.calcsize() called on invalid object"))
            },
        }));
        struct_type_dict.insert("pack".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "pack".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("pack() requires at least 1 argument"));
                }
                let fmt = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    dict.get("format").map(|v| v.str()).unwrap_or_default()
                } else {
                    return Err(PyError::type_error("Struct.pack() called on invalid object"));
                };
                // Delegate to struct.pack
                let mut pack_args = vec![py_str(&fmt)];
                pack_args.extend_from_slice(&args[1..]);
                struct_pack_impl(&pack_args)
            },
        }));
        struct_type_dict.insert("unpack".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "unpack".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("unpack() requires at least 1 argument"));
                }
                let fmt = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    dict.get("format").map(|v| v.str()).unwrap_or_default()
                } else {
                    return Err(PyError::type_error("Struct.unpack() called on invalid object"));
                };
                // Delegate to struct.unpack
                let mut pack_args = vec![py_str(&fmt)];
                pack_args.extend_from_slice(&args[1..]);
                struct_unpack_impl(&pack_args)
            },
        }));
        struct_type_dict.insert("__repr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |args| {
                if let Some(inst) = args.first() {
                    if let PyObject::Instance { dict, .. } = &*inst.borrow() {
                        if let Some(fmt) = dict.get("format") {
                            let fmt_str = fmt.str();
                            return Ok(py_str(&format!("Struct('{}')", fmt_str)));
                        }
                    }
                }
                Ok(py_str("Struct(?)"))
            },
        }));
        let struct_type = PyObjectRef::new(PyObject::Type {
            name: "Struct".to_string(),
            dict: Box::new(str_map_to_typedict(struct_type_dict)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str("Struct", struct_type);
    }

    d.insert_str(
        "error",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "error".to_string(),
            func: |args| {
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "error".to_string(),
                    args: args.to_vec(),
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }))
            },
        }),
    );

    d
}
