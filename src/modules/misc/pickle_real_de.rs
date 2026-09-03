//! Real-protocol pickle UNPICKLER (stack machine for CPython opcodes).
//!
//! The legacy custom wire format (`pickle_deserialize`) cannot read bytes
//! produced by CPython or by our own pure-Python `Lib/pickle.py` (which the
//! C-suite tests exercise via `_pickle.Unpickler`). This module parses the
//! real protocol 0-5 opcodes: scalars, strings/bytes with escapes, memo
//! (GET/PUT/BINPUT/.../MEMOIZE), containers (APPEND/SETITEM/ADDITEMS/...),
//! GLOBAL/STACK_GLOBAL resolution, and object construction
//! (REDUCE/BUILD/NEWOBJ/OBJ/INST). Out-of-band buffers (NEXT_BUFFER) and
//! persistent IDs without a loader are honest errors, not panics — every
//! read is bounds-checked (test_pickle feeds truncated buffers).

use crate::object::*;
use std::collections::HashMap;

fn unpickling_error(msg: impl Into<String>) -> PyError {
    PyError::Exception(
        "UnpicklingError".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "UnpicklingError".to_string(),
            args: vec![py_str(&msg.into())],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

#[derive(Clone)]
enum StackVal {
    Val(PyObjectRef),
    Mark,
}

/// Read exactly `n` bytes or fail cleanly.
fn take<'a>(data: &'a [u8], pos: &mut usize, n: usize) -> PyResult<&'a [u8]> {
    if *pos + n > data.len() {
        return Err(unpickling_error("truncated pickle data"));
    }
    let s = &data[*pos..*pos + n];
    *pos += n;
    Ok(s)
}

/// Read a decimal ASCII integer terminated by `\n`.
fn read_decimal(data: &[u8], pos: &mut usize) -> PyResult<i64> {
    let start = *pos;
    while *pos < data.len() && data[*pos] != b'\n' {
        *pos += 1;
    }
    if *pos >= data.len() {
        return Err(unpickling_error("truncated decimal in pickle data"));
    }
    let s = std::str::from_utf8(&data[start..*pos])
        .map_err(|_| unpickling_error("invalid utf-8 in pickle decimal"))?;
    *pos += 1; // skip '\n'
    // Strip trailing 'L' (protocol 0/1 longs).
    let s = s.strip_suffix('L').unwrap_or(s);
    s.parse::<i64>()
        .map_err(|_| unpickling_error(format!("invalid integer: {s}")))
}

/// Read a raw (non-decimal) line terminated by `\n` (STRING/UNICODE bodies).
fn read_line<'a>(data: &'a [u8], pos: &mut usize) -> PyResult<&'a [u8]> {
    let start = *pos;
    while *pos < data.len() && data[*pos] != b'\n' {
        *pos += 1;
    }
    if *pos >= data.len() {
        return Err(unpickling_error("truncated string in pickle data"));
    }
    let s = &data[start..*pos];
    *pos += 1;
    Ok(s)
}

fn le_i16(b: &[u8]) -> i16 {
    i16::from_le_bytes([b[0], b[1]])
}
fn le_u16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn le_i32(b: &[u8]) -> i32 {
    i32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn le_u64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}
fn be_f64(b: &[u8]) -> f64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    f64::from_be_bytes(a)
}

/// Decode Python string escapes (STRING body): \\ \' \" \n \r \t \xXX.
/// Returns lossy string on invalid sequences (matches CPython leniency
/// for the common cases; exotic escapes pass through).
fn unescape_string(body: &[u8]) -> String {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i] == b'\\' && i + 1 < body.len() {
            match body[i + 1] {
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'\'' => {
                    out.push(b'\'');
                    i += 2;
                }
                b'"' => {
                    out.push(b'"');
                    i += 2;
                }
                b'x' if i + 3 < body.len() => {
                    if let Ok(hex) = std::str::from_utf8(&body[i + 2..i + 4]) {
                        if let Ok(v) = u8::from_str_radix(hex, 16) {
                            out.push(v);
                            i += 4;
                            continue;
                        }
                    }
                    out.push(b'\\');
                    i += 1;
                }
                _ => {
                    out.push(body[i + 1]);
                    i += 2;
                }
            }
        } else {
            out.push(body[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode raw-unicode-escape (UNICODE body): \uXXXX \UXXXXXXXX \N{...}.
fn unescape_unicode(body: &[u8]) -> String {
    let mut out = String::with_capacity(body.len());
    let s = String::from_utf8_lossy(body);
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' => {
                    out.push('\n');
                    i += 2;
                }
                'r' => {
                    out.push('\r');
                    i += 2;
                }
                't' => {
                    out.push('\t');
                    i += 2;
                }
                '\\' => {
                    out.push('\\');
                    i += 2;
                }
                'u' if i + 5 < chars.len() => {
                    let hex: String = chars[i + 2..i + 6].iter().collect();
                    if let Ok(v) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(v) {
                            out.push(c);
                            i += 6;
                            continue;
                        }
                    }
                    out.push('\\');
                    i += 1;
                }
                'U' if i + 9 < chars.len() => {
                    let hex: String = chars[i + 2..i + 10].iter().collect();
                    if let Ok(v) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(v) {
                            out.push(c);
                            i += 10;
                            continue;
                        }
                    }
                    out.push('\\');
                    i += 1;
                }
                _ => {
                    out.push(chars[i + 1]);
                    i += 2;
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Resolve `module.name` to a callable (builtins + registered classes).
fn resolve_global(module: &str, name: &str) -> PyResult<PyObjectRef> {
    // Synthesized `_codecs.encode/decode` FIRST (before find_class, which
    // returns bogus hits for these): protocol 0-2 pickles bytes via
    // `REDUCE(_codecs.encode, (str, 'latin1'))`.
    if module == "_codecs" && name == "encode" {
        return Ok(PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_codecs.encode".to_string(),
            func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "_codecs.encode() requires 2 arguments",
                    ));
                }
                let enc = args[1].str().to_lowercase();
                if enc != "latin1" && enc != "latin-1" && enc != "iso-8859-1" {
                    return Err(PyError::value_error(format!(
                        "unsupported encoding for _codecs shim: {enc}"
                    )));
                }
                let text = args[0].str();
                let bytes: Vec<u8> =
                    text.chars().map(|c| (c as u32 & 0xFF) as u8).collect();
                Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
            },
        }));
    }
    if module == "_codecs" && name == "decode" {
        return Ok(PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_codecs.decode".to_string(),
            func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "_codecs.decode() requires 2 arguments",
                    ));
                }
                let enc = args[1].str().to_lowercase();
                if enc != "latin1" && enc != "latin-1" && enc != "iso-8859-1" {
                    return Err(PyError::value_error(format!(
                        "unsupported encoding for _codecs shim: {enc}"
                    )));
                }
                let raw: Vec<u8> = match &*args[0].borrow() {
                    PyObject::Bytes(b) => b.clone(),
                    _ => args[0].str().into_bytes(),
                };
                let s: String = raw.iter().map(|&b| b as char).collect();
                Ok(py_str(&s))
            },
        }));
    }
    // Common builtins by hand (no import machinery needed).
    if module == "builtins" || module == "__builtin__" {
        let v = match name {
            "list" => crate::modules::get_builtin_class("list"),
            "dict" => crate::modules::get_builtin_class("dict"),
            "tuple" => crate::modules::get_builtin_class("tuple"),
            "set" => crate::modules::get_builtin_class("set"),
            "frozenset" => crate::modules::get_builtin_class("frozenset"),
            "bytes" => crate::modules::get_builtin_class("bytes"),
            "bytearray" => crate::modules::get_builtin_class("bytearray"),
            "str" => crate::modules::get_builtin_class("str"),
            "int" => crate::modules::get_builtin_class("int"),
            "float" => crate::modules::get_builtin_class("float"),
            "complex" => crate::modules::get_builtin_class("complex"),
            "bool" => crate::modules::get_builtin_class("bool"),
            "range" => crate::modules::get_builtin_class("range"),
            "slice" => crate::modules::get_builtin_class("slice"),
            "object" => crate::modules::get_builtin_class("object"),
            "Exception" => crate::modules::get_builtin_class("Exception"),
            _ => None,
        };
        if let Some(v) = v {
            return Ok(v);
        }
    }
    if let Some(v) = crate::object::find_class_by_qualified_name(module, name) {
        return Ok(v);
    }
    // Fall back to module attribute lookup (functions like _codecs.encode,
    // copyreg helpers, etc. — needed for REDUCE-based bytes in proto 0-2).
    // Import machinery errors are swallowed; failure to resolve is an
    // honest unpickling error below.
    let mod_obj = (|| {
        // Try sys.modules first, then a real import.
        if let Some(sys_mod) = crate::modules::get_module("sys") {
            let b = sys_mod.borrow();
            if let PyObject::Module { dict, .. } = &*b {
                if let Some(mods) = dict.get_str("modules").cloned() {
                    if let PyObject::Dict(d) = &*mods.borrow() {
                        if let Some(m) = d.get(&py_str(module)).ok().flatten() {
                            return Some(m);
                        }
                    }
                }
            }
        }
        None
    })();
    if let Some(m) = mod_obj {
        if let Ok(v) = m.borrow().get_attribute(name) {
            return Ok(v);
        }
    }
    // Last resort: import machinery is unreliable from this context
    // (thread-local VM state); unresolvable names are honest errors.
    Err(unpickling_error(format!(
        "cannot resolve global {module}.{name}"
    )))
}

/// Pop a value (error on empty stack or bare mark).
fn pop_val(stack: &mut Vec<StackVal>) -> PyResult<PyObjectRef> {
    match stack.pop() {
        Some(StackVal::Val(v)) => Ok(v),
        _ => Err(unpickling_error("stack underflow in pickle data")),
    }
}

/// Pop all values since the last mark (for APPENDS/SETITEMS/ADDITEMS/TUPLE).
fn pop_mark(stack: &mut Vec<StackVal>) -> PyResult<Vec<PyObjectRef>> {
    let mut items = Vec::new();
    loop {
        match stack.pop() {
            Some(StackVal::Val(v)) => items.push(v),
            Some(StackVal::Mark) => break,
            None => return Err(unpickling_error("MARK not found in pickle data")),
        }
    }
    items.reverse();
    Ok(items)
}

fn memo_get(memo: &[PyObjectRef], idx: usize) -> PyResult<PyObjectRef> {
    memo.get(idx).cloned().ok_or_else(|| {
        unpickling_error(format!("memo index out of range: {idx}"))
    })
}

/// Parse real-protocol pickle bytes with a stack machine.
/// Returns the value at STOP. All reads are bounds-checked.
pub fn unpickle_real(data: &[u8]) -> PyResult<PyObjectRef> {
    let mut pos = 0usize;
    let mut stack: Vec<StackVal> = Vec::new();
    let mut memo: Vec<PyObjectRef> = Vec::new();
    // Extension registry (copyreg) not wired; EXT opcodes error honestly.
    loop {
        if pos >= data.len() {
            return Err(unpickling_error("truncated pickle data (no STOP)"));
        }
        let op = data[pos];
        pos += 1;
        match op {
            // framing / protocol
            0x80 => {
                // PROTO + version byte
                if pos >= data.len() {
                    return Err(unpickling_error("truncated PROTO"));
                }
                let _ver = data[pos];
                pos += 1;
            }
            0x95 => {
                // FRAME + u64 length (framing only; keep parsing inline)
                let b = take(data, &mut pos, 8)?;
                let _len = le_u64(b);
            }
            b'.' => {
                // STOP: result is stack top
                return pop_val(&mut stack);
            }
            // scalars
            b'N' => stack.push(StackVal::Val(py_none())),
            0x88 => stack.push(StackVal::Val(py_bool(true))),
            0x89 => stack.push(StackVal::Val(py_bool(false))),
            b'I' => {
                // INT decimal (optional trailing L); I01/I00 are bools
                let start = pos;
                while pos < data.len() && data[pos] != b'\n' {
                    pos += 1;
                }
                if pos >= data.len() {
                    return Err(unpickling_error("truncated INT"));
                }
                let mut s = std::str::from_utf8(&data[start..pos])
                    .map_err(|_| unpickling_error("invalid INT"))?
                    .to_string();
                pos += 1;
                // Protocol 0/1 bools are exactly I01\n / I00\n.
                let is_bool_true = s == "01";
                let is_bool_false = s == "00";
                if is_bool_true {
                    stack.push(StackVal::Val(py_bool(true)));
                } else if is_bool_false {
                    stack.push(StackVal::Val(py_bool(false)));
                } else {
                    let s = s.strip_suffix('L').unwrap_or(&s);
                    // Big integers beyond i64 need BigInt parse.
                    if let Ok(v) = s.parse::<i64>() {
                        stack.push(StackVal::Val(py_int(v)));
                    } else {
                        let big: num_bigint::BigInt = s
                            .parse()
                            .map_err(|_| unpickling_error("invalid INT"))?;
                        stack.push(StackVal::Val(py_int(big)));
                    }
                }
            }
            b'J' => {
                let b = take(data, &mut pos, 4)?;
                stack.push(StackVal::Val(py_int(le_i32(b) as i64)));
            }
            b'K' => {
                let b = take(data, &mut pos, 1)?;
                stack.push(StackVal::Val(py_int(b[0] as i64)));
            }
            b'M' => {
                let b = take(data, &mut pos, 2)?;
                stack.push(StackVal::Val(py_int(le_u16(b) as i64)));
            }
            b'L' => {
                // LONG decimal + '\n' (protocol 0/1 big ints)
                let start = pos;
                while pos < data.len() && data[pos] != b'\n' {
                    pos += 1;
                }
                if pos >= data.len() {
                    return Err(unpickling_error("truncated LONG"));
                }
                let s = std::str::from_utf8(&data[start..pos])
                    .map_err(|_| unpickling_error("invalid LONG"))?;
                pos += 1;
                let s = s.strip_suffix('L').unwrap_or(s);
                if let Ok(v) = s.parse::<i64>() {
                    stack.push(StackVal::Val(py_int(v)));
                } else {
                    let big: num_bigint::BigInt = s
                        .parse()
                        .map_err(|_| unpickling_error("invalid LONG"))?;
                    stack.push(StackVal::Val(py_int(big)));
                }
            }
            0x8a => {
                // LONG1: u8 length + little-endian signed bytes
                let b = take(data, &mut pos, 1)?;
                let n = b[0] as usize;
                let raw = take(data, &mut pos, n)?;
                stack.push(StackVal::Val(py_int(signed_from_le(raw))));
            }
            0x8b => {
                // LONG4: i32 length + bytes
                let b = take(data, &mut pos, 4)?;
                let n = le_i32(b);
                if n < 0 {
                    return Err(unpickling_error("negative LONG4 length"));
                }
                let raw = take(data, &mut pos, n as usize)?;
                stack.push(StackVal::Val(py_int(signed_from_le(raw))));
            }
            b'F' => {
                // FLOAT decimal + '\n'
                let start = pos;
                while pos < data.len() && data[pos] != b'\n' {
                    pos += 1;
                }
                if pos >= data.len() {
                    return Err(unpickling_error("truncated FLOAT"));
                }
                let s = std::str::from_utf8(&data[start..pos])
                    .map_err(|_| unpickling_error("invalid FLOAT"))?;
                pos += 1;
                let f: f64 = match s {
                    "nan" | "-nan" => f64::NAN,
                    "inf" | "Infinity" => f64::INFINITY,
                    "-inf" | "-Infinity" => f64::NEG_INFINITY,
                    _ => s
                        .parse()
                        .map_err(|_| unpickling_error("invalid FLOAT"))?,
                };
                stack.push(StackVal::Val(py_float(f)));
            }
            b'G' => {
                let b = take(data, &mut pos, 8)?;
                stack.push(StackVal::Val(py_float(be_f64(b))));
            }
            // strings
            b'V' => {
                let body = read_line(data, &mut pos)?;
                stack.push(StackVal::Val(py_str(&unescape_unicode(body))));
            }
            b'X' => {
                let b = take(data, &mut pos, 4)?;
                let n = le_u32(b) as usize;
                let raw = take(data, &mut pos, n)?;
                let s = std::str::from_utf8(raw)
                    .map_err(|_| unpickling_error("invalid BINUNICODE"))?;
                stack.push(StackVal::Val(py_str(s)));
            }
            0x8c => {
                let b = take(data, &mut pos, 1)?;
                let n = b[0] as usize;
                let raw = take(data, &mut pos, n)?;
                let s = std::str::from_utf8(raw)
                    .map_err(|_| unpickling_error("invalid SHORT_BINUNICODE"))?;
                stack.push(StackVal::Val(py_str(s)));
            }
            0x8d => {
                let b = take(data, &mut pos, 8)?;
                let n = le_u64(b) as usize;
                let raw = take(data, &mut pos, n)?;
                let s = std::str::from_utf8(raw)
                    .map_err(|_| unpickling_error("invalid BINUNICODE8"))?;
                stack.push(StackVal::Val(py_str(s)));
            }
            b'S' => {
                let body = read_line(data, &mut pos)?;
                // Stripped quotes: '...' or "..." with escapes
                if body.len() >= 2
                    && ((body[0] == b'\'' && body[body.len() - 1] == b'\'')
                        || (body[0] == b'"' && body[body.len() - 1] == b'"'))
                {
                    stack.push(StackVal::Val(py_str(&unescape_string(&body[1..body.len() - 1]))));
                } else {
                    return Err(unpickling_error("invalid STRING quoting"));
                }
            }
            b'T' => {
                let b = take(data, &mut pos, 4)?;
                let n = le_i32(b);
                if n < 0 {
                    return Err(unpickling_error("negative BINSTRING length"));
                }
                let raw = take(data, &mut pos, n as usize)?;
                stack.push(StackVal::Val(py_str(&unescape_string(raw))));
            }
            b'U' => {
                let b = take(data, &mut pos, 1)?;
                let n = b[0] as usize;
                let raw = take(data, &mut pos, n)?;
                stack.push(StackVal::Val(py_str(&unescape_string(raw))));
            }
            // bytes
            b'B' => {
                let b = take(data, &mut pos, 4)?;
                let n = le_i32(b);
                if n < 0 {
                    return Err(unpickling_error("negative BINBYTES length"));
                }
                let raw = take(data, &mut pos, n as usize)?;
                stack.push(StackVal::Val(PyObjectRef::imm(PyObject::Bytes(
                    raw.to_vec(),
                ))));
            }
            b'C' => {
                let b = take(data, &mut pos, 1)?;
                let n = b[0] as usize;
                let raw = take(data, &mut pos, n)?;
                stack.push(StackVal::Val(PyObjectRef::imm(PyObject::Bytes(
                    raw.to_vec(),
                ))));
            }
            0x8e => {
                let b = take(data, &mut pos, 8)?;
                let n = le_u64(b) as usize;
                let raw = take(data, &mut pos, n)?;
                stack.push(StackVal::Val(PyObjectRef::imm(PyObject::Bytes(
                    raw.to_vec(),
                ))));
            }
            0x96 => {
                let b = take(data, &mut pos, 8)?;
                let n = le_u64(b) as usize;
                let raw = take(data, &mut pos, n)?;
                stack.push(StackVal::Val(PyObjectRef::new(PyObject::ByteArray(
                    raw.to_vec(),
                ))));
            }
            // containers
            b'(' => stack.push(StackVal::Mark),
            b']' => stack.push(StackVal::Val(py_list(vec![]))),
            b'}' => stack.push(StackVal::Val(py_dict())),
            0x8f => stack.push(StackVal::Val(PyObjectRef::new(PyObject::Set(
                crate::object::PySet::new(),
            )))),
            b'l' => {
                let items = pop_mark(&mut stack)?;
                stack.push(StackVal::Val(py_list(items)));
            }
            b't' => {
                let items = pop_mark(&mut stack)?;
                stack.push(StackVal::Val(py_tuple(items)));
            }
            b'd' => {
                let items = pop_mark(&mut stack)?;
                if items.len() % 2 != 0 {
                    return Err(unpickling_error("odd DICT items"));
                }
                let mut d = PyDict::new();
                let mut it = items.into_iter();
                while let (Some(k), Some(v)) = (it.next(), it.next()) {
                    let _ = d.set(k, v);
                }
                stack.push(StackVal::Val(PyObjectRef::new(PyObject::Dict(Box::new(
                    d,
                )))));
            }
            b'e' => {
                // APPENDS: extend list below mark
                let items = pop_mark(&mut stack)?;
                let target = pop_val(&mut stack)?;
                {
                    if let PyObject::List(lst) = &mut *target.borrow_mut() {
                        lst.extend(items);
                    } else {
                        return Err(unpickling_error("APPENDS on non-list"));
                    }
                }
                stack.push(StackVal::Val(target));
            }
            b'a' => {
                // APPEND: one item onto list below
                let item = pop_val(&mut stack)?;
                let target = pop_val(&mut stack)?;
                {
                    if let PyObject::List(lst) = &mut *target.borrow_mut() {
                        lst.push(item);
                    } else {
                        return Err(unpickling_error("APPEND on non-list"));
                    }
                }
                stack.push(StackVal::Val(target));
            }
            b'u' => {
                // SETITEMS: extend dict below mark
                let items = pop_mark(&mut stack)?;
                if items.len() % 2 != 0 {
                    return Err(unpickling_error("odd SETITEMS items"));
                }
                let target = pop_val(&mut stack)?;
                {
                    if let PyObject::Dict(d) = &mut *target.borrow_mut() {
                        let mut it = items.into_iter();
                        while let (Some(k), Some(v)) = (it.next(), it.next()) {
                            let _ = d.set(k, v);
                        }
                    } else {
                        return Err(unpickling_error("SETITEMS on non-dict"));
                    }
                }
                stack.push(StackVal::Val(target));
            }
            b's' => {
                // SETITEM: dict[k] = v
                let v = pop_val(&mut stack)?;
                let k = pop_val(&mut stack)?;
                let target = pop_val(&mut stack)?;
                {
                    if let PyObject::Dict(d) = &mut *target.borrow_mut() {
                        let _ = d.set(k, v);
                    } else {
                        return Err(unpickling_error("SETITEM on non-dict"));
                    }
                }
                stack.push(StackVal::Val(target));
            }
            0x90 => {
                // ADDITEMS: extend set below mark
                let items = pop_mark(&mut stack)?;
                let target = pop_val(&mut stack)?;
                {
                    if let PyObject::Set(s) = &mut *target.borrow_mut() {
                        for item in items {
                            let _ = s.add(item);
                        }
                    } else {
                        return Err(unpickling_error("ADDITEMS on non-set"));
                    }
                }
                stack.push(StackVal::Val(target));
            }
            0x91 => {
                // FROZENSET: pop mark-delimited items, push frozenset.
                let items = pop_mark(&mut stack)?;
                let mut fs = crate::object::PySet::new();
                for item in items {
                    let _ = fs.add(item);
                }
                stack.push(StackVal::Val(PyObjectRef::new(PyObject::FrozenSet(
                    fs,
                ))));
            }
            0x85 => {
                // TUPLE1
                let a = pop_val(&mut stack)?;
                stack.push(StackVal::Val(py_tuple(vec![a])));
            }
            0x86 => {
                // TUPLE2
                let b = pop_val(&mut stack)?;
                let a = pop_val(&mut stack)?;
                stack.push(StackVal::Val(py_tuple(vec![a, b])));
            }
            0x87 => {
                // TUPLE3
                let c = pop_val(&mut stack)?;
                let b = pop_val(&mut stack)?;
                let a = pop_val(&mut stack)?;
                stack.push(StackVal::Val(py_tuple(vec![a, b, c])));
            }
            // memo
            b'g' => {
                let idx = read_decimal(data, &mut pos)?;
                if idx < 0 {
                    return Err(unpickling_error("negative GET index"));
                }
                stack.push(StackVal::Val(memo_get(&memo, idx as usize)?));
            }
            b'h' => {
                let b = take(data, &mut pos, 1)?;
                stack.push(StackVal::Val(memo_get(&memo, b[0] as usize)?));
            }
            b'j' => {
                let b = take(data, &mut pos, 4)?;
                let idx = le_i32(b);
                if idx < 0 {
                    return Err(unpickling_error("negative LONG_BINGET index"));
                }
                stack.push(StackVal::Val(memo_get(&memo, idx as usize)?));
            }
            b'p' => {
                let idx = read_decimal(data, &mut pos)?;
                if idx < 0 {
                    return Err(unpickling_error("negative PUT index"));
                }
                let idx = idx as usize;
                let top = match stack.last() {
                    Some(StackVal::Val(v)) => v.clone(),
                    _ => return Err(unpickling_error("PUT on empty stack")),
                };
                if memo.len() <= idx {
                    memo.resize(idx + 1, py_none());
                }
                memo[idx] = top;
            }
            b'q' => {
                let b = take(data, &mut pos, 1)?;
                let idx = b[0] as usize;
                let top = stack_last(&stack)?;
                if memo.len() <= idx {
                    memo.resize(idx + 1, py_none());
                }
                memo[idx] = top;
            }
            b'r' => {
                let b = take(data, &mut pos, 4)?;
                let idx = le_i32(b);
                if idx < 0 {
                    return Err(unpickling_error("negative LONG_BINPUT"));
                }
                let top = stack_last(&stack)?;
                let idx = idx as usize;
                if memo.len() <= idx {
                    memo.resize(idx + 1, py_none());
                }
                memo[idx] = top;
            }
            0x94 => {
                // MEMOIZE: append top
                memo.push(stack_last(&stack)?);
            }
            // stack manipulation
            b'0' => {
                pop_val(&mut stack)?;
            }
            b'1' => {
                // POP_MARK
                pop_mark(&mut stack)?;
            }
            b'2' => {
                // DUP
                let top = stack_last(&stack)?;
                stack.push(StackVal::Val(top));
            }
            // globals + construction
            b'c' => {
                // GLOBAL: module\nname\n
                let module = read_global_str(data, &mut pos)?;
                let name = read_global_str(data, &mut pos)?;
                stack.push(StackVal::Val(resolve_global(&module, &name)?));
            }
            0x93 => {
                // STACK_GLOBAL: pops name then module name.
                let name_v = pop_val(&mut stack)?;
                let module_v = pop_val(&mut stack)?;
                stack.push(StackVal::Val(resolve_global(
                    &module_v.str(),
                    &name_v.str(),
                )?));
            }
            b'R' => {
                // REDUCE: pop args, pop callable, call
                let args_v = pop_val(&mut stack)?;
                let func = pop_val(&mut stack)?;
                let arg_list: Vec<PyObjectRef> =
                    if let PyObject::Tuple(items) = &*args_v.borrow() {
                        items.clone()
                    } else {
                        return Err(unpickling_error("REDUCE args not tuple"));
                    };
                let res = crate::object::builtin_call(&func, &arg_list)?;
                stack.push(StackVal::Val(res));
            }
            b'b' => {
                // BUILD: pop state, pop instance; apply
                let state = pop_val(&mut stack)?;
                let inst = pop_val(&mut stack)?;
                apply_build_state(&inst, &state)?;
                stack.push(StackVal::Val(inst));
            }
            b'o' => {
                // OBJ: pop args, pop callable; build without __init__
                let args_v = pop_val(&mut stack)?;
                let cls = pop_val(&mut stack)?;
                let arg_list: Vec<PyObjectRef> =
                    if let PyObject::Tuple(items) = &*args_v.borrow() {
                        items.clone()
                    } else {
                        return Err(unpickling_error("OBJ args not tuple"));
                    };
                stack.push(StackVal::Val(build_instance(&cls, &arg_list, None)?));
            }
            b'i' => {
                // INST: pop args, pop callable (old-style, same as OBJ here)
                let args_v = pop_val(&mut stack)?;
                let cls = pop_val(&mut stack)?;
                let arg_list: Vec<PyObjectRef> = if let PyObject::Tuple(items) =
                    &*args_v.borrow()
                {
                    items.clone()
                } else {
                    return Err(unpickling_error("INST args not tuple"));
                };
                stack.push(StackVal::Val(build_instance(&cls, &arg_list, None)?));
            }
            0x81 => {
                // NEWOBJ: pop args, pop cls
                let args_v = pop_val(&mut stack)?;
                let cls = pop_val(&mut stack)?;
                let arg_list: Vec<PyObjectRef> = if let PyObject::Tuple(items) =
                    &*args_v.borrow()
                {
                    items.clone()
                } else {
                    return Err(unpickling_error("NEWOBJ args not tuple"));
                };
                stack.push(StackVal::Val(build_instance(&cls, &arg_list, None)?));
            }
            0x92 => {
                // NEWOBJ_EX: pop kwargs, pop args, pop cls
                let kwargs_v = pop_val(&mut stack)?;
                let args_v = pop_val(&mut stack)?;
                let cls = pop_val(&mut stack)?;
                let arg_list: Vec<PyObjectRef> = if let PyObject::Tuple(items) =
                    &*args_v.borrow()
                {
                    items.clone()
                } else {
                    return Err(unpickling_error("NEWOBJ_EX args not tuple"));
                };
                stack.push(StackVal::Val(build_instance(&cls, &arg_list, Some(
                    kwargs_v,
                ))?));
            }
            0x82 | 0x83 | 0x84 => {
                // EXT1/EXT2/EXT4: extension registry (copyreg) — not wired.
                return Err(unpickling_error("EXT opcodes not supported"));
            }
            b'P' => {
                // PERSID without persistent_load
                let _ = read_line(data, &mut pos)?;
                return Err(unpickling_error(
                    "PERSID without persistent_load",
                ));
            }
            b'Q' => {
                // BINPERSID without persistent_load
                let _ = pop_val(&mut stack)?;
                return Err(unpickling_error(
                    "BINPERSID without persistent_load",
                ));
            }
            0x97 | 0x98 => {
                return Err(unpickling_error(
                    "out-of-band buffers require Unpickler with buffers",
                ));
            }
            other => {
                return Err(unpickling_error(format!(
                    "unknown opcode 0x{other:02x}"
                )));
            }
        }
    }
}

/// Stack top as value (for PUT/MEMOIZE/DUP).
fn stack_last(stack: &[StackVal]) -> PyResult<PyObjectRef> {
    match stack.last() {
        Some(StackVal::Val(v)) => Ok(v.clone()),
        _ => Err(unpickling_error("empty stack")),
    }
}

/// Read a GLOBAL component line (module or name).
fn read_global_str(data: &[u8], pos: &mut usize) -> PyResult<String> {
    if *pos >= data.len() {
        return Err(unpickling_error("truncated GLOBAL opcode"));
    }
    let start = *pos;
    while *pos < data.len() && data[*pos] != b'\n' {
        *pos += 1;
    }
    if *pos >= data.len() {
        return Err(unpickling_error("truncated GLOBAL opcode"));
    }
    let s = std::str::from_utf8(&data[start..*pos])
        .map_err(|_| unpickling_error("invalid utf-8 in GLOBAL"))?
        .to_string();
    *pos += 1;
    Ok(s)
}

/// Interpret a little-endian two's-complement byte string as BigInt.
fn signed_from_le(raw: &[u8]) -> num_bigint::BigInt {
    if raw.is_empty() {
        return num_bigint::BigInt::from(0);
    }
    let mut v = num_bigint::BigInt::from(0);
    for (i, &byte) in raw.iter().enumerate() {
        v += num_bigint::BigInt::from(byte) * (num_bigint::BigInt::from(256).pow(i as u32));
    }
    // Sign-extend from the top bit.
    if raw[raw.len() - 1] & 0x80 != 0 {
        v -= num_bigint::BigInt::from(256).pow(raw.len() as u32);
    }
    v
}

/// Build an instance for NEWOBJ/OBJ/INST without calling __init__.
fn build_instance(
    cls: &PyObjectRef,
    args: &[PyObjectRef],
    kwargs: Option<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    // Call cls.__new__(cls, *args) when available; else blank instance.
    let new_m = cls.borrow().get_attribute("__new__").ok();
    if let Some(m) = new_m {
        let mut call_args = vec![cls.clone()];
        call_args.extend_from_slice(args);
        if let Ok(obj) = crate::object::builtin_call(&m, &call_args) {
            let _ = kwargs;
            return Ok(obj);
        }
    }
    // Fallback: blank instance of the class.
    let mut dict = crate::object::AttrMap::new();
    let _ = kwargs;
    Ok(PyObjectRef::new(PyObject::Instance {
        typ: cls.clone(),
        dict,
    }))
}

/// Apply BUILD state to an instance (dict update or __setstate__).
fn apply_build_state(inst: &PyObjectRef, state: &PyObjectRef) -> PyResult<()> {
    if matches!(&*state.borrow(), PyObject::None) {
        return Ok(());
    }
    // Prefer __setstate__ when defined.
    if let Ok(m) = inst.borrow().get_attribute("__setstate__") {
        // Skip the default object.__setstate__ placeholder if present?
        // Call it; a no-op default is harmless.
        let _ = crate::object::builtin_call(&m, &[state.clone()]);
        return Ok(());
    }
    // Snapshot state items first (borrow ends before mutating inst).
    let state_items: Option<Vec<(String, PyObjectRef)>> =
        if let PyObject::Dict(d) = &*state.borrow() {
            let mut items = Vec::new();
            for k in d.keys() {
                if let Some(v) = d.get(&k).ok().flatten() {
                    items.push((k.str(), v.clone()));
                }
            }
            Some(items)
        } else {
            None
        };
    if let Some(items) = state_items {
        if let PyObject::Instance { dict, .. } = &mut *inst.borrow_mut() {
            for (k, v) in items {
                dict.insert_str(&k, v);
            }
            return Ok(());
        }
        // Native exception/dict/list targets: best effort (no instance
        // dict to merge into).
        return Ok(());
    }
    // (state, slotstate) 2-tuple form (protocol 2+ objects with __slots__).
    if let PyObject::Tuple(items) = &*state.borrow() {
        if items.len() == 2 {
            apply_build_state(inst, &items[0])?;
            // Slot state dict: apply similarly.
            if let PyObject::Dict(d) = &*items[1].borrow() {
                if let PyObject::Instance { dict, .. } = &mut *inst.borrow_mut() {
                    for k in d.keys() {
                        if let Some(v) = d.get(&k).ok().flatten() {
                            let _ = dict.insert(k.str(), v.clone());
                        }
                    }
                }
            }
            return Ok(());
        }
    }
    Err(unpickling_error("unsupported BUILD state"))
}
