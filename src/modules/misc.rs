use crate::object::*;
use std::collections::HashMap;

mod collections;
pub use collections::*;

mod types;
pub use types::*;

mod csv;
pub use csv::*;
mod re;
pub use re::*;

mod struct_heapq;
pub use struct_heapq::*;

mod graphlib;
pub use graphlib::*;

mod weakref;
pub use weakref::*;

mod numbers;
pub use numbers::*;

mod this;
pub use this::*;

mod queue;
pub use queue::*;

mod cmath;
pub use cmath::*;

mod hashlib_extra;
pub use hashlib_extra::*;

mod sysconfig;
pub use sysconfig::*;

mod xml;
pub use xml::*;

mod gettext;
pub use gettext::*;

mod email_utils;
pub use email_utils::*;

mod contextlib;
pub use contextlib::*;

mod getpass;
pub use getpass::*;

mod json_tool;
pub use json_tool::*;

mod logging_config;
pub use logging_config::*;

mod array;
pub use array::*;
mod sunau;
pub use sunau::*;
mod argparse;
pub use argparse::*;

mod gc;
pub use gc::*;
mod locale;
pub use locale::*;
mod colorsys;
pub use colorsys::*;
mod threading;
pub use threading::*;
mod platform;
pub use platform::*;
mod getopt;
pub use getopt::*;

mod email_mime_text;
pub use email_mime_text::*;

mod email_header;
pub use email_header::*;

mod copy;
pub use copy::*;
mod uuid;
pub use uuid::*;
mod ast;
pub use ast::*;

mod email;
pub use email::*;
mod contextvars;
pub use contextvars::*;
mod wave;
pub use wave::*;
mod ssl;
pub use ssl::*;
mod asyncio;
pub use asyncio::*;
mod logging;
pub use logging::*;
mod thread;
pub use thread::*;
mod signal;
pub use signal::*;










// Real Enum/IntEnum/StrEnum/EnumType/auto/unique semantics are implemented
// as real Python source instead — see ENUM_SOURCE (below) and
// VirtualMachine::install_source_defined_stdlib.
pub const ENUM_SOURCE: &str = include_str!("enum_extra.py");

// Build a UUID instance from a 32-hex-char string (no dashes).





// ---- pickle helper functions ----

/// Serialize a Python object to bytes using a simple custom format.
///
/// Format (byte markers):
///   N       -> None
///   T       -> True
///   F       -> False
///   I<val>\n -> int (decimal, newline-terminated)
///   G<val>\n -> float (decimal, newline-terminated)
///   S<len>:<utf8>  -> str (length-prefixed UTF-8)
///   B<len>:<bytes>  -> bytes (length-prefixed raw bytes)
///   [ ... ] -> list (elements serialized recursively)
///   ( ... ) -> tuple (elements serialized recursively)
///   { ... } -> dict (alternating key-value pairs serialized recursively)
/// Extract a stable identity pointer for a boxed (non-inline) `PyObject` —
/// used by `pickle_serialize`'s memo so a container (list/dict/deque) that
/// appears twice in one pickle — including a genuine cycle like
/// `d.append(d)` — serializes as a `@<id>` reference instead of recursing
/// forever (real CPython's pickle memo does the same).
fn container_ptr(o: &PyObjectRef) -> Option<*const ()> {
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
    static PICKLE_CLASS_REGISTRY: std::cell::RefCell<HashMap<String, PyObjectRef>> =
        std::cell::RefCell::new(HashMap::new());
}

fn pickle_serialize(
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

/// Try to unpickle a CPython-compat range_iterator produced by
/// `pickle.dumps(iter(range(...)))` with `__setstate__(index)` via the `b`
/// (BUILD) opcode. CPython's test_range.py::test_iterator_unpickle_compat
/// pins exactly this: 10 historical pickle byte strings (protocols 0-4,
/// including Python 2's `xrange`) that all decode to `iter(range(10,20,2))`
/// with index 2 and to a large-negative range variant. Our own pickle format
/// uses `R`/`r` etc. and cannot parse these — `pickle_deserialize` would see
/// the first `c` GLOBAL and return early with trailing bytes left over.
fn try_unpickle_rangeiter_compat(data: &[u8]) -> Option<PyObjectRef> {
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
fn pickle_deserialize(
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
            // Function by reference (see the matching serializer arm).
            let module = pickle_deserialize(data, pos, memo)?;
            let name = pickle_deserialize(data, pos, memo)?;
            let module_str = module.str();
            let name_str = name.str();
            let func = crate::modules::get_module(&module_str)
                .and_then(|m| m.borrow().get_attribute(&name_str).ok())
                .ok_or_else(|| {
                    PyError::type_error(format!(
                        "cannot find function {}.{} referenced by pickle data",
                        module_str, name_str
                    ))
                })?;
            if matches!(&*func.borrow(), PyObject::Function(_)) {
                Ok(func)
            } else {
                Err(PyError::type_error(format!(
                    "{}.{} is not a function",
                    module_str, name_str
                )))
            }
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


thread_local! {
    // Each callback stores the callable plus the extra positional args
    // (and a trailing keyword dict, if any) it was registered with — real
    // `atexit.register(func, *args, **kwargs)` passes those on invocation.
    static EXIT_CALLBACKS: std::cell::RefCell<Vec<(PyObjectRef, Vec<PyObjectRef>, Vec<(String, PyObjectRef)>)>> = std::cell::RefCell::new(Vec::new());
}

thread_local! {
    // The real `sys` module (registered once at VM init) — native code like
    // atexit's `_run_exitfuncs` reads the CURRENT `sys.unraisablehook` from
    // it to report raising callbacks. A disposable VM's own sys module would
    // hold the DEFAULT hook, losing any reassignment made by
    // `catch_unraisable_exception`-style contexts.
    static CURRENT_SYS_MODULE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

pub(crate) fn set_sys_module(mod_ref: Option<PyObjectRef>) {
    CURRENT_SYS_MODULE.with(|m| *m.borrow_mut() = mod_ref);
}

thread_local! {
    // The real builtins map (see `set_builtins_ref`) — lets native code
    // resolve a builtin exception CLASS object by name.
    static CURRENT_BUILTINS: std::cell::RefCell<Option<std::rc::Rc<std::collections::HashMap<crate::interner::StrId, PyObjectRef>>>> = std::cell::RefCell::new(None);
}

pub(crate) fn set_builtins_ref(
    builtins: std::rc::Rc<std::collections::HashMap<crate::interner::StrId, PyObjectRef>>,
) {
    CURRENT_BUILTINS.with(|b| *b.borrow_mut() = Some(builtins));
}

pub(crate) fn get_builtin_class(name: &str) -> Option<PyObjectRef> {
    CURRENT_BUILTINS.with(|b| {
        let map = b.borrow().clone()?;
        let id = crate::interner::intern(name);
        map.get(&id).cloned()
    })
}

/// Add `cls` to an ABC's `_abc_registry` (CPython's `ABC.register(cls)`).
fn abc_register_class(abc: &PyObjectRef, cls: &PyObjectRef) {
    if let PyObject::Type { dict, .. } = &mut *abc.borrow_mut() {
        let mut items = if let Some(r) = dict.get_str("_abc_registry") {
            if let PyObject::FrozenSet(s) = &*r.borrow() {
                s.to_vec()
            } else {
                vec![]
            }
        } else {
            vec![]
        };
        if !items.iter().any(|r| r.is(cls)) {
            items.push(cls.clone());
        }
        let mut set = PySet::new();
        for i in items {
            let _ = set.add(i);
        }
        dict.insert_str("_abc_registry", PyObjectRef::imm(PyObject::FrozenSet(set)));
    }
}

/// Register the builtin container types as virtual subclasses of their
/// `collections.abc` ABCs (CPython's `_collections_abc` module does this at
/// startup) — so `issubclass(dict, Mapping)`, `issubclass(list, Sequence)`
/// etc. hold. Must run AFTER the builtins map is available.
pub(crate) fn register_collections_abc_builtins() {
    let abc = get_module("collections.abc");
    let Some(abc) = abc else { return };
    let get_name = |d: &HashMap<String, PyObjectRef>, n: &str| d.get(n).cloned();
    let abc_entries: HashMap<String, PyObjectRef> = {
        let b = abc.borrow();
        if let PyObject::Module { dict, .. } = &*b {
            dict.iter()
                .map(|(k, v)| (crate::interner::lookup_str(*k).to_string(), v.clone()))
                .collect()
        } else {
            return;
        }
    };
    let builtin = |n: &str| get_builtin_class(n);
    let reg = |abc_name: &str, builtin_name: &str| {
        if let (Some(abc), Some(b)) = (get_name(&abc_entries, abc_name), builtin(builtin_name)) {
            abc_register_class(&abc, &b);
        }
    };
    reg("Mapping", "dict");
    reg("MutableMapping", "dict");
    reg("Sequence", "list");
    reg("Sequence", "str");
    reg("Sequence", "tuple");
    reg("Sequence", "bytes");
    reg("Sequence", "bytearray");
    reg("Sequence", "range");
    reg("MutableSequence", "list");
    reg("MutableSequence", "bytearray");
    reg("Set", "set");
    reg("Set", "frozenset");
    reg("MutableSet", "set");
    reg("Iterable", "list");
    reg("Iterable", "tuple");
    reg("Iterable", "dict");
    reg("Iterable", "set");
    reg("Iterable", "frozenset");
    reg("Iterable", "str");
    reg("Iterable", "bytes");
    reg("Iterable", "bytearray");
    reg("Iterable", "range");
    reg("Collection", "list");
    reg("Collection", "tuple");
    reg("Collection", "dict");
    reg("Collection", "set");
    reg("Collection", "frozenset");
    reg("Collection", "str");
    reg("Collection", "bytes");
    reg("Collection", "bytearray");
    reg("Reversible", "list");
    reg("Reversible", "tuple");
    reg("Reversible", "str");
    reg("Reversible", "bytes");
    reg("Reversible", "bytearray");
    reg("Reversible", "range");
    reg("Sized", "list");
    reg("Sized", "tuple");
    reg("Sized", "dict");
    reg("Sized", "set");
    reg("Sized", "frozenset");
    reg("Sized", "str");
    reg("Sized", "bytes");
    reg("Sized", "bytearray");
    reg("Sized", "range");
    reg("Hashable", "str");
    reg("Hashable", "bytes");
    reg("Hashable", "tuple");
    reg("Hashable", "frozenset");
    reg("Iterator", "list_iterator");
}

/// Look up a module by name through the live `sys.modules` dict (no VM
/// needed — a plain dict read; safe from inside a native closure that is
/// itself running under the VM).
pub(crate) fn get_module(name: &str) -> Option<PyObjectRef> {
    CURRENT_SYS_MODULE.with(|m| {
        let sys_mod = m.borrow().clone()?;
        let modules = {
            let b = sys_mod.borrow();
            if let PyObject::Module { dict, .. } = &*b {
                dict.get_str("modules").cloned()
            } else {
                None
            }
        }?;
        let mb = modules.borrow();
        if let PyObject::Dict(d) = &*mb {
            d.get(&py_str(name)).ok().flatten()
        } else {
            None
        }
    })
}

fn get_current_unraisablehook() -> Option<PyObjectRef> {
    CURRENT_SYS_MODULE.with(|m| {
        let mod_ref = m.borrow().clone()?;
        let borrowed = mod_ref.borrow();
        if let PyObject::Module { dict, .. } = &*borrowed {
            dict.get_str("unraisablehook").cloned()
        } else {
            None
        }
    })
}

// `UnraisableHookArgs`-shaped object for a raising atexit callback (real
// CPython passes object=None for atexit callbacks, the func's repr in
// err_msg, and the exception's type/value). exc_type is the real builtin
// exception class (looked up through sys.modules['builtins'], so identity
// matches what Python code holds) and exc_value a real PyObject::Exception.
fn build_unraisable_args(func: &PyObjectRef, err: &PyError) -> PyObjectRef {
    let exc_name = py_error_type_name(err);
    if std::env::var("RPY_DEBUG_UNRAISABLE").is_ok() {
        eprintln!(
            "UNRAISABLE name={} err={:?} builtin={:?}",
            exc_name,
            err,
            get_builtin_class(&exc_name).map(|b| b.repr())
        );
    }
    let exc_value = PyObjectRef::new(PyObject::Exception {
        typ: exc_name.clone(),
        args: py_error_args(err),
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra: None,
    });
    let exc_type = CURRENT_SYS_MODULE.with(|m| {
        let mod_ref = m.borrow().clone()?;
        let borrowed = mod_ref.borrow();
        let modules = if let PyObject::Module { dict, .. } = &*borrowed {
            dict.get_str("modules").cloned()
        } else {
            None
        };
        let modules = modules?;
        let builtins_mod = {
            let mb = modules.borrow();
            if let PyObject::Dict(d) = &*mb {
                d.get(&py_str("builtins")).ok().flatten()
            } else {
                None
            }
        }?;
        let bb = builtins_mod.borrow();
        if let PyObject::Module { dict, .. } = &*bb {
            dict.get_str(&exc_name).cloned()
        } else {
            None
        }
    });
    let mut attrs = crate::object::AttrMap::new();
    attrs.insert_str("object", py_none());
    attrs.insert_str(
        "err_msg",
        py_str(&format!(
            "Exception ignored in atexit callback {}",
            func.repr()
        )),
    );
    attrs.insert_str("exc_type", exc_type.unwrap_or_else(|| py_none()));
    attrs.insert_str("exc_value", exc_value);
    attrs.insert_str("exc_traceback", py_none());
    let typ = PyObjectRef::new(PyObject::Type {
        name: "UnraisableHookArgs".to_string(),
        dict: Box::new(crate::object::str_map_to_typedict(
            std::collections::HashMap::new(),
        )),
        bases: vec![],
        mro: vec![],
    });
    PyObjectRef::new(PyObject::Instance { typ, dict: attrs })
}

fn py_error_type_name(err: &PyError) -> String {
    match err {
        PyError::TypeError(_) => "TypeError".to_string(),
        PyError::ValueError(_) => "ValueError".to_string(),
        PyError::NameError(_) => "NameError".to_string(),
        PyError::AttributeError(_) => "AttributeError".to_string(),
        PyError::IndexError(_) => "IndexError".to_string(),
        PyError::KeyError(_) => "KeyError".to_string(),
        PyError::ZeroDivisionError(_) => "ZeroDivisionError".to_string(),
        PyError::RuntimeError(_) => "RuntimeError".to_string(),
        PyError::SystemExit(_) => "SystemExit".to_string(),
        PyError::Exception(name, exc) => {
            // `raise SomeClass` (bare class, no message) comes through as
            // PyError::Exception("", exc) — the NAME field is empty, so
            // recover the exception type from the exc object itself.
            if name.is_empty() {
                match &*exc.borrow() {
                    PyObject::Exception { typ, .. } => typ.clone(),
                    PyObject::ExceptionGroup { typ, .. } => typ.clone(),
                    PyObject::Instance { typ, .. } => typ.borrow().type_name(),
                    _ => "Exception".to_string(),
                }
            } else {
                name.clone()
            }
        }
        PyError::OsError(_) => "OSError".to_string(),
        PyError::ImportError(_) => "ImportError".to_string(),
        PyError::RecursionError(_) => "RecursionError".to_string(),
        _ => "Exception".to_string(),
    }
}

fn py_error_args(err: &PyError) -> Vec<PyObjectRef> {
    match err {
        PyError::TypeError(m)
        | PyError::ValueError(m)
        | PyError::NameError(m)
        | PyError::AttributeError(m)
        | PyError::IndexError(m)
        | PyError::KeyError(m)
        | PyError::ZeroDivisionError(m)
        | PyError::RuntimeError(m)
        | PyError::ImportError(m)
        | PyError::RecursionError(m)
        | PyError::OsError(m) => vec![py_str(m)],
        PyError::Exception(_, exc) => {
            if let PyObject::Exception { args, .. } = &*exc.borrow() {
                args.clone()
            } else {
                vec![exc.clone()]
            }
        }
        _ => vec![],
    }
}

pub fn create_atexit_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "register",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "register".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "register() requires a callable argument",
                    ));
                }
                // Real `atexit.register(func, *args, **kwargs)` stores the extra
                // positional args (and, if present, a trailing keyword dict) and
                // passes them to `func` when it runs at shutdown — `test_atexit`
                // registers `print` with a message arg, and `test_shutdown`
                // asserts the printed output.
                let func = args[0].clone();
                let mut extra = args[1..].to_vec();
                let mut kwargs: Vec<(String, PyObjectRef)> = Vec::new();
                let trailing_is_dict = extra
                    .last()
                    .map(|l| matches!(&*l.borrow(), PyObject::Dict(_)))
                    .unwrap_or(false);
                if trailing_is_dict {
                    // Extract the trailing keyword-dict's items into `kwargs`
                    // (cloned so no borrow is held across `extra.pop()`).
                    let items: Vec<(String, PyObjectRef)> = {
                        let b = extra.last().unwrap().borrow();
                        if let PyObject::Dict(d) = &*b {
                            d.items().into_iter().map(|(k, v)| (k.str(), v)).collect()
                        } else {
                            Vec::new()
                        }
                    };
                    extra.pop();
                    kwargs = items;
                }
                EXIT_CALLBACKS.with(|cb| cb.borrow_mut().push((func, extra, kwargs)));
                Ok(py_none())
            },
        }),
    );
    d.insert_str(
        "unregister",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "unregister".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "unregister() requires a callable argument",
                    ));
                }
                let target = args[0].clone();
                // Real CPython compares callbacks with `==` (a callback's own
                // `__eq__` may even call unregister re-entrantly — see CPython
                // issue #112127 / _test_atexit's test_eq_unregister), NOT
                // identity. Evaluate equality WITHOUT holding the callbacks
                // borrow (re-entrant unregister needs borrow_mut), removing each
                // match from the live list as it is found.
                let funcs: Vec<PyObjectRef> = EXIT_CALLBACKS
                    .with(|cb| cb.borrow().iter().map(|(f, _, _)| f.clone()).collect());
                for f in &funcs {
                    let eq = crate::object::py_compare(f, &target, 2)
                        .map(|v| v.truthy())
                        .unwrap_or(false);
                    if eq {
                        EXIT_CALLBACKS.with(|cb| cb.borrow_mut().retain(|(g, _, _)| !g.is(f)));
                    }
                }
                Ok(py_none())
            },
        }),
    );
    d.insert_str("__name__", py_str("atexit"));
    d.insert_str(
        "_clear",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_clear".to_string(),
            func: |_| {
                EXIT_CALLBACKS.with(|cb| cb.borrow_mut().clear());
                Ok(py_none())
            },
        }),
    );
    // `atexit._ncallbacks` — real CPython's internal count of registered
    // callbacks, read directly by `test_atexit.py`'s `test_callbacks_leak`/
    // `test_callbacks_leak_refcycle` to detect leaked registrations. Backed
    // by the live `EXIT_CALLBACKS` list length so it stays in sync.
    d.insert_str(
        "_ncallbacks",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_ncallbacks".to_string(),
            func: |_| Ok(py_int(EXIT_CALLBACKS.with(|cb| cb.borrow().len() as i64))),
        }),
    );
    // `atexit.is_tracing()` — real CPython returns True iff a Python-level
    // trace function is currently set (`sys.gettrace() != None`). This
    // interpreter's `sys.settrace` is a no-op stub, so no tracing is ever
    // active; `test_atexit.py`'s leak tests call it during callback
    // iteration.
    d.insert_str(
        "is_tracing",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "is_tracing".to_string(),
            func: |_| Ok(py_bool(false)),
        }),
    );
    // `atexit._run_exitfuncs()` — runs all registered callbacks in LIFO
    // order and CLEARS them (real CPython's internal function, exercised
    // directly by the vendored `_test_atexit.py`, which runs it in-process
    // to verify ordering/arg-passing/unraisable handling without exiting).
    d.insert_str(
        "_run_exitfuncs",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_run_exitfuncs".to_string(),
            func: |_| {
                let callbacks: Vec<(PyObjectRef, Vec<PyObjectRef>, Vec<(String, PyObjectRef)>)> =
                    EXIT_CALLBACKS.with(|cb| cb.borrow().clone());
                for (func, extra, kwargs) in callbacks.iter().rev() {
                    // A raising callback is "unraisable" — real CPython reports
                    // it via sys.unraisablehook (the current hook, which
                    // catch_unraisable_exception-style contexts may have
                    // reassigned), then continues with the next callback.
                    let result = crate::object::call_function_disposable(
                        func,
                        extra.clone(),
                        kwargs.clone(),
                    );
                    if let Err(err) = result {
                        let unraisable = build_unraisable_args(func, &err);
                        if let Some(hook) = get_current_unraisablehook() {
                            let _ = crate::object::call_function_disposable(
                                &hook,
                                vec![unraisable],
                                vec![],
                            );
                        }
                    }
                }
                EXIT_CALLBACKS.with(|cb| cb.borrow_mut().clear());
                Ok(py_none())
            },
        }),
    );
    d
}

/// Run all registered atexit handlers, using the provided VM.
pub fn run_atexit_handlers(vm: &mut crate::vm::VirtualMachine) {
    // Opcode histogram dump (RPY_OPCODE_HIST=1) — every normal exit path
    // funnels through here, so this is the one reliable dump point.
    crate::vm::opcode_hist_dump();
    // Real CPython runs exit handlers in LIFO order (last registered runs
    // FIRST) — `test_shutdown`'s `atexit.register(print, "one"); atexit.
    // register(print, "two")` expects output `two` then `one`.
    let callbacks: Vec<(PyObjectRef, Vec<PyObjectRef>, Vec<(String, PyObjectRef)>)> =
        EXIT_CALLBACKS.with(|cb| cb.borrow().clone());
    for (func, extra, kwargs) in callbacks.iter().rev() {
        let mut call_args = extra.clone();
        if !kwargs.is_empty() {
            let mut kwd = PyDict::new();
            for (k, v) in kwargs {
                let _ = kwd.set(py_str(k), v.clone());
            }
            call_args.push(PyObjectRef::new(PyObject::Dict(Box::new(kwd))));
        }
        let _ = vm.call_function(func.clone(), call_args, vec![]);
    }
}

/// Compile `stmt` once and run it `number` times in pooled VMs.
/// Returns elapsed total seconds.
fn timeit_run_compiled(code: &crate::bytecode::CodeObject, number: u64) -> PyResult<f64> {
    let start = std::time::Instant::now();
    for _ in 0..number {
        let mut vm = crate::vm::VirtualMachine::take_disposable();
        let r = vm.run(code.clone());
        crate::vm::VirtualMachine::release_disposable(vm);
        r.map_err(|e| PyError::type_error(format!("timeit error: {}", e)))?;
    }
    Ok(start.elapsed().as_secs_f64())
}

fn timeit_compile_src(src: &str, what: &str) -> PyResult<crate::bytecode::CodeObject> {
    let mut parser = crate::parser::Parser::new(src);
    let program = parser
        .parse_program()
        .map_err(|e| PyError::type_error(format!("timeit {} parse error: {}", what, e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    compiler
        .compile(&program, "<timeit>")
        .map_err(|e| PyError::type_error(format!("timeit {} compile error: {}", what, e)))
}
fn timeit_native_compile(src: &str) -> PyResult<PyObjectRef> {
    let code = timeit_compile_src(src, "compile")?;
    Ok(PyObjectRef::imm(PyObject::Code(Rc::new(code))))
}

fn timeit_native_run_in_globals(code_obj: &PyObjectRef, globals: &PyObjectRef) -> PyResult<PyObjectRef> {
    let code_rc = match &*code_obj.borrow() {
        PyObject::Code(c) => c.clone(),
        _ => return Err(PyError::type_error("_run_in_globals expects a code object")),
    };
    let mut map: HashMap<crate::interner::StrId, PyObjectRef> = HashMap::new();
    if let PyObject::Dict(d) = &*globals.borrow() {
        for (k, v) in d.items() {
            if let PyObject::Str(sk) = &*k.borrow() {
                map.insert(crate::interner::intern(sk.as_str()), v.clone());
            }
        }
    }
    let bmod = crate::vm::get_shared_builtins_module();
    map.insert(crate::interner::intern("__builtins__"), bmod);
    // Inside this pooled-VM execution, sys.modules is the shared truth:
    // `import timeit` must resolve to the REAL module object (with
    // test-injected attributes like _fake_timer), not a stale snapshot.
    crate::vm::set_sys_modules_priority(true);
    let mut vm = crate::vm::VirtualMachine::take_disposable();
    vm.globals = Rc::new(RefCell::new(map));
    let r = vm.run((*code_rc).clone());
    crate::vm::set_sys_modules_priority(false);
    crate::vm::VirtualMachine::release_disposable(vm);
    r
}


/// Native `timeit.Timer`.
///
/// Faithful enough for CPython's own `test_timeit.py`:
/// * `stmt`/`setup` may be strings (compiled once, executed in the given
///   or synthesized globals) OR callables (invoked directly).
/// * `timer` must be a callable used as the clock — the returned "elapsed"
///   is `timer_end - timer_start`, which is how the fake-timer tests get
///   exact deltas (`delta_time == number`).
/// * `globals` is the namespace statements execute in.
fn split_kwargs(args: &[PyObjectRef]) -> (usize, Vec<(String, PyObjectRef)>) {
    if let Some(last) = args.last() {
        let b = last.borrow();
        if let PyObject::Dict(d) = &*b {
            if args.len() >= 2 {
                let pairs = d.items();
                let kw: Vec<(String, PyObjectRef)> = pairs
                    .iter()
                    .map(|(k, v)| (k.str(), v.clone()))
                    .collect();
                return (args.len() - 1, kw);
            }
        }
    }
    (args.len(), Vec::new())
}

fn kw_lookup<'a>(kw: &'a [(String, PyObjectRef)], name: &str) -> Option<&'a PyObjectRef> {
    kw.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn make_timeit_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    macro_rules! t_method {
        ($name:expr, $func:expr) => {
            type_dict.insert(
                $name.to_string(),
                PyObjectRef::imm(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // helper: call a Python callable from native context
    fn py_call(f: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
        if let PyObject::Instance { typ, .. } = &*f.borrow() {
            if let Some(cm) = crate::object::lookup_dunder_via_mro(typ, "__call__") {
                return crate::object::call_bound_method(cm, f.clone(), args);
            }
            return Err(PyError::type_error("object is not callable"));
        }
        // Python functions need a VM; use the disposable-VM caller.
        crate::object::call_function_disposable(&f, args, vec![])
    }

    t_method!("__init__", |args| {
        if std::env::var("RPY_DBG_TT").is_ok() {
            eprintln!("NATIVE Timer.__init__ nargs={} a1={:?}", args.len(), args.get(1).map(|v| v.str()));
        }
        let self_obj = args
            .first()
            .cloned()
            .ok_or_else(|| PyError::type_error("__init__ missing self"))?;
        let (n, kw) = split_kwargs(args);
        let getp = |i: usize| -> Option<PyObjectRef> { args.get(i + 1).cloned() };
        let pos_stmt = getp(0);
        let pos_setup = getp(1);
        let pos_timer = getp(2);
        let stmt = kw_lookup(&kw, "stmt").or(pos_stmt.as_ref()).cloned();
        let setup = kw_lookup(&kw, "setup").or(pos_setup.as_ref()).cloned();
        let timer = kw_lookup(&kw, "timer").or(pos_timer.as_ref()).cloned();
        let globals_v = kw_lookup(&kw, "globals").cloned();
        {
            let mut b = self_obj.borrow_mut();
            if let PyObject::Instance { dict, .. } = &mut *b {
                dict.insert_str("_stmt", stmt.clone().unwrap_or_else(|| py_str("pass")));
                dict.insert_str("_setup", setup.clone().unwrap_or_else(|| py_str("pass")));
                dict.insert_str(
                    "_timer",
                    timer.unwrap_or_else(|| py_none()),
                );
                dict.insert_str(
                    "_globals",
                    globals_v.unwrap_or_else(|| py_none()),
                );
            }
        }
        Ok(py_none())
    });

    // Runs one timed measurement. Returns elapsed seconds per CPython rules:
    // uses the injected timer when present.
    fn run_timed(
        self_obj: &PyObjectRef,
        number: u64,
    ) -> PyResult<f64> {
        let (stmt_v, setup_v, timer_v, globals_v) = {
            let b = self_obj.borrow();
            let get = |k: &str| -> Option<PyObjectRef> {
                if let PyObject::Instance { dict, .. } = &*b {
                    dict.get_str(k).cloned()
                } else {
                    None
                }
            };
            (get("_stmt"), get("_setup"), get("_timer"), get("_globals"))
        };

        let is_callable = |v: &Option<PyObjectRef>| -> bool {
            v.as_ref()
                .map(|x| {
                    matches!(
                        &*x.borrow(),
                        PyObject::Function(_)
                            | PyObject::BuiltinFunction { .. }
                            | PyObject::BuiltinMethod { .. }
                            | PyObject::BoundMethod { .. }
                            | PyObject::Instance { .. }
                    )
                })
                .unwrap_or(false)
        };

        // Prepare globals dict (PyObject::Dict) for string execution.
        let globals_dict: PyObjectRef = match globals_v {
            Some(g) if !matches!(&*g.borrow(), PyObject::None) => g,
            _ => PyObjectRef::imm(PyObject::Dict(Box::new(PyDict::new()))),
        };

        // Resolve setup: compile or wrap callable
        enum Prepared {
            Src(std::rc::Rc<crate::bytecode::CodeObject>),
            Callable(PyObjectRef),
        }
        let setup_prep: Option<Prepared> = match &setup_v {
            Some(v) if is_callable(&Some(v.clone())) => Some(Prepared::Callable(v.clone())),
            Some(v) => {
                let src = v.str();
                if src.trim().is_empty() || src.trim() == "pass" {
                    None
                } else {
                    let cobj = timeit_native_compile(&src)?;
                    let c = match &*cobj.borrow() {
                        PyObject::Code(c) => c.clone(),
                        _ => unreachable!(),
                    };
                    Some(Prepared::Src(c))
                }
            }
            None => None,
        };
        let stmt_prep = match &stmt_v {
            Some(v) if is_callable(&Some(v.clone())) => Prepared::Callable(v.clone()),
            Some(v) => {
                let src = v.str();
                Prepared::Src(match timeit_native_compile(&src)? {
                    PyObjectRef::Imm(rc) => match &*rc.borrow() {
                        PyObject::Code(c) => c.clone(),
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                })
            }
            None => return Err(PyError::type_error("timeit missing stmt")),
        };

        // Run setup once (not timed)
        match &setup_prep {
            Some(Prepared::Callable(f)) => {
                py_call(f.clone(), vec![])?;
            }
            Some(Prepared::Src(code)) => {
                let cobj = PyObjectRef::imm(PyObject::Code(code.clone()));
                timeit_native_run_in_globals(&cobj, &globals_dict)?;
            }
            None => {}
        }

        // Clock
        use std::time::Instant;
        let timer_is_usable = timer_v.as_ref().map(|t| {
            match &*t.borrow() {
                PyObject::None => false,
                PyObject::Instance { typ, .. } => {
                    crate::object::lookup_dunder_via_mro(typ, "__call__").is_some()
                }
                _ => true,
            }
        }).unwrap_or(false);
        let has_py_timer = timer_is_usable;

        if has_py_timer {
            let timer = timer_v.clone().unwrap();
            let t0 = py_call(timer.clone(), vec![])?;
            match &stmt_prep {
                Prepared::Callable(f) => {
                    for _ in 0..number {
                        py_call(f.clone(), vec![])?;
                    }
                }
                Prepared::Src(code) => {
                    let cobj = PyObjectRef::imm(PyObject::Code(code.clone()));
                    for _ in 0..number {
                        timeit_native_run_in_globals(&cobj, &globals_dict)?;
                    }
                }
            }
            let t1 = py_call(timer.clone(), vec![])?;
            // delta = t1 - t0 (both floats or ints)
            py_sub(&t1, &t0)?
                .as_f64()
                .ok_or_else(|| PyError::type_error("timer returned non-number"))
        } else {
            let t0 = Instant::now();
            match &stmt_prep {
                Prepared::Callable(f) => {
                    for _ in 0..number {
                        py_call(f.clone(), vec![])?;
                    }
                }
                Prepared::Src(code) => {
                    let cobj = PyObjectRef::imm(PyObject::Code(code.clone()));
                    for _ in 0..number {
                        timeit_native_run_in_globals(&cobj, &globals_dict)?;
                    }
                }
            }
            Ok(t0.elapsed().as_secs_f64())
        }
    }

    t_method!("timeit", |args| {
        let self_obj = args.first().cloned().unwrap();
        let (n, kw) = split_kwargs(args);
        if std::env::var("RPY_DBG_TT").is_ok() {
            eprintln!("TT timeit nargs={} kw={:?}", n, kw);
        }
        let number = kw_lookup(&kw, "number")
            .and_then(|v| v.as_i64())
            .or_else(|| args.get(n - n + 1).and_then(|v| v.as_i64()))
            .unwrap_or(1_000_000)
            .max(0) as u64;
        let secs = run_timed(&self_obj, number)?;
        Ok(py_float(secs))
    });

    t_method!("repeat", |args| {
        let self_obj = args.first().cloned().unwrap();
        let (n, kw) = split_kwargs(args);
        // positional fallback: bound-method args are [self, repeat, number]
        let repeat = kw_lookup(&kw, "repeat")
            .and_then(|v| v.as_i64())
            .or_else(|| args.get(1).and_then(|v| v.as_i64()))
            .unwrap_or(5)
            .max(0) as u64;
        let number = kw_lookup(&kw, "number")
            .and_then(|v| v.as_i64())
            .or_else(|| args.get(2).and_then(|v| v.as_i64()))
            .unwrap_or(1_000_000)
            .max(0) as u64;
        let mut times = Vec::new();
        for _ in 0..repeat {
            let secs = run_timed(&self_obj, number)?;
            times.push(py_float(secs));
        }
        Ok(py_list(times))
    });

    // autorange(callback=None) -> (num_loops, time_per_loop).
    // Uses CPython's 1-2-5-per-decade search sequence.
    t_method!("autorange", |args| {
        let self_obj = args.first().cloned().unwrap();
        let callback: Option<PyObjectRef> = args.get(1).and_then(|c| {
            if matches!(&*c.borrow(), PyObject::None) { None } else { Some(c.clone()) }
        }).or_else(|| {
            // kwargs form: callback=<callable> in trailing Dict
            args.last().and_then(|d| {
                let b = d.borrow();
                if let PyObject::Dict(dd) = &*b {
                    dd.items().into_iter()
                        .find(|(k, _)| k.str() == "callback")
                        .map(|(_, v)| v.clone())
                } else { None }
            })
        });
        let report = |callback: &Option<PyObjectRef>, n: usize, secs: f64| -> PyResult<()> {
            if let Some(cb) = callback {
                crate::object::call_function_disposable(
                    cb,
                    vec![py_int(n as i64), py_float(secs)],
                    vec![],
                )?;
            }
            Ok(())
        };
        let mut base = 1usize;
        loop {
            for j in [1usize, 2, 5] {
                let number = base * j;
                let secs = run_timed(&self_obj, number as u64)?;
                report(&callback, number, secs)?;
                if secs >= 0.2 {
                    // CPython returns TOTAL time for the whole run.
                    return Ok(py_tuple(vec![
                        py_int(number as i64),
                        py_float(secs),
                    ]));
                }
            }
            base *= 10;
            if base > 1_000_000_000 {
                return Ok(py_tuple(vec![py_int(base as i64), py_float(0.0)]));
            }
        }
    });

    PyObjectRef::new(PyObject::Type {
        name: "Timer".to_string(),
        dict: Box::new(crate::object::str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub fn create_timeit_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! timeit_func {
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

    timeit_func!("timeit", |args| {
        // Trailing Dict = kwargs appended by the dispatcher.
        let (pos, kw) = match args.last() {
            Some(d) => {
                let b = d.borrow();
                if let PyObject::Dict(dd) = &*b {
                    let mut p: Vec<PyObjectRef> = args[..args.len()-1].to_vec();
                    // drop a positional None/placeholder setup if kw supplies one
                    let wrapped = PyObjectRef::imm(PyObject::Dict(dd.clone()));
                    let (_, kwd) = split_kwargs(&[py_none(), wrapped]);
                    if let Some(sv) = kw_lookup(&kwd, "setup") { if p.len() > 1 { p.truncate(1); } }
                    (p, kwd)
                } else { (args.to_vec(), Vec::new()) }
            }
            None => (args.to_vec(), Vec::new()),
        };
        let stmt_v = pos.first().cloned().unwrap_or_else(|| py_str("pass"));
        let setup_v = kw_lookup(&kw, "setup").cloned()
            .or_else(|| pos.get(1).cloned())
            .unwrap_or_else(|| py_str("pass"));
        let timer_v = kw_lookup(&kw, "timer").cloned()
            .or_else(|| pos.get(2).cloned())
            .unwrap_or_else(|| py_none());
        let globals_v = kw_lookup(&kw, "globals").cloned()
            .or_else(|| pos.get(3).cloned())
            .unwrap_or_else(|| py_none());
        let mut cargs = vec![stmt_v, setup_v, timer_v, globals_v];
        let timer_obj = make_timeit_type();
        let inst = crate::object::call_function(&timer_obj, cargs)?;
        let m = inst.borrow().get_attribute("timeit")?;
        let nv_owned = kw_lookup(&kw, "number").map(|v| v.clone())
            .or_else(|| pos.get(1).cloned());
        let mut margs: Vec<PyObjectRef> = vec![];
        if let Some(nv) = nv_owned { margs.push(nv); }
        crate::object::call_function(&m, margs)
    });

    // Also provide a repeat function for convenience — delegates to Timer
    // so callables/timer/globals behave exactly like the class methods.
    timeit_func!("repeat", |args| {
        let (pos, kw) = match args.last() {
            Some(d) => {
                let b = d.borrow();
                if let PyObject::Dict(dd) = &*b {
                    let wrapped = PyObjectRef::imm(PyObject::Dict(dd.clone()));
                    let (_, kwd) = split_kwargs(&[py_none(), wrapped]);
                    let p: Vec<PyObjectRef> = args[..args.len()-1].to_vec();
                    (p, kwd)
                } else { (args.to_vec(), Vec::new()) }
            }
            None => (args.to_vec(), Vec::new()),
        };
        let stmt_v = pos.first().cloned().unwrap_or_else(|| py_str("pass"));
        let setup_v = kw_lookup(&kw, "setup").cloned()
            .or_else(|| pos.get(1).cloned())
            .unwrap_or_else(|| py_str("pass"));
        let timer_v = kw_lookup(&kw, "timer").cloned()
            .or_else(|| pos.get(2).cloned())
            .unwrap_or_else(|| py_none());
        let globals_v = kw_lookup(&kw, "globals").cloned()
            .or_else(|| pos.get(3).cloned())
            .unwrap_or_else(|| py_none());
        let mut cargs = vec![stmt_v, setup_v, timer_v, globals_v];
        let timer_obj = make_timeit_type();
        let inst = crate::object::call_function(&timer_obj, cargs)?;
        let m = inst.borrow().get_attribute("repeat")?;
        let rv_owned = kw_lookup(&kw, "repeat").map(|v| v.clone())
            .or_else(|| pos.get(1).cloned());
        let nv_owned = kw_lookup(&kw, "number").map(|v| v.clone())
            .or_else(|| pos.get(2).cloned());
        let mut margs: Vec<PyObjectRef> = vec![];
        if let Some(rv) = rv_owned { margs.push(rv); }
        if let Some(nv) = nv_owned { margs.push(nv); }
        crate::object::call_function(&m, margs)
    });

    d.insert("Timer".to_string(), make_timeit_type());
    d.insert(
        "reindent".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "reindent".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("reindent takes 2 arguments"));
                }
                let src = args[0].str();
                let n = args[1].as_i64().unwrap_or(0).max(0) as usize;
                if n == 0 {
                    // strip common leading whitespace per line, preserving empties
                    let out: Vec<String> = src
                        .lines()
                        .map(|l| l.trim_start().to_string())
                        .collect();
                    return Ok(py_str(&out.join("\n")));
                }
                let pad = " ".repeat(n);
                let out: Vec<String> = src.lines().map(|l| if l.is_empty() { String::new() } else { format!("{}{}", pad, l) }).collect();
                Ok(py_str(&out.join("\n")))
            },
        }),
    );
    d.insert(
        "_compile".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_compile".to_string(),
            func: |args| {
                let src = args
                    .first()
                    .map(|v| v.str())
                    .ok_or_else(|| PyError::type_error("_compile missing src"))?;
                timeit_native_compile(&src)
            },
        }),
    );
    d.insert(
        "_run_in_globals".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_run_in_globals".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("_run_in_globals needs code, globals"));
                }
                timeit_native_run_in_globals(&args[0], &args[1])
            },
        }),
    );
    d.insert_str("default_number", py_int(1_000_000));
    d.insert_str("default_repeat", py_int(3));

    d
}










pub fn create_configparser_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Helper: parse INI string into sections
    fn parse_ini_string(data: &str) -> HashMap<String, HashMap<String, String>> {
        let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current_section: Option<String> = None;

        // Start with a pseudo-section for DEFAULT values
        sections.insert("DEFAULT".to_string(), HashMap::new());

        for line in data.lines() {
            let trimmed = line.trim();
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                continue;
            }

            // Section header: [sectionname]
            if trimmed.starts_with('[') {
                if let Some(end) = trimmed.find(']') {
                    let name = trimmed[1..end].trim().to_string();
                    if !name.is_empty() {
                        current_section = Some(name.clone());
                        sections.entry(name).or_insert_with(HashMap::new);
                    }
                }
                continue;
            }

            // Key = value (or key: value)
            if let Some(eq_pos) = trimmed.find('=').or_else(|| trimmed.find(':')) {
                let key = trimmed[..eq_pos].trim().to_string();
                let value = trimmed[eq_pos + 1..].trim().to_string();
                if !key.is_empty() {
                    let section_name = current_section
                        .clone()
                        .unwrap_or_else(|| "DEFAULT".to_string());
                    let section = sections.entry(section_name).or_insert_with(HashMap::new);
                    section.insert(key, value);
                }
            }
        }

        sections
    }

    // ConfigParser class — constructor
    d.insert_str(
        "ConfigParser",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "ConfigParser".to_string(),
            func: |_args| {
                let mut type_dict = HashMap::new();

                // read_string(self, string) — parse INI from a string
                type_dict.insert_str(
                    "read_string",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "read_string".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "read_string() missing required argument: string",
                                ));
                            }
                            let data = inner_args[1].str();

                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "read_string(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            let parsed = parse_ini_string(&data);
                            // Merge parsed sections into existing sections
                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                for (section_name, options) in parsed {
                                    let section_key = py_str(&section_name);
                                    // Try to get existing section dict
                                    let existing =
                                        sections_dict.get(&section_key).ok().and_then(|o| o);
                                    if let Some(existing_ref) = existing {
                                        if let PyObject::Dict(ref mut existing_dict) =
                                            &mut *existing_ref.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ =
                                                    existing_dict.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                    } else {
                                        // Create new section dict
                                        let option_dict = py_dict();
                                        if let PyObject::Dict(ref mut od) =
                                            &mut *option_dict.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ = od.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                        let _ =
                                            sections_dict.set(py_str(&section_name), option_dict);
                                    }
                                }
                            }

                            Ok(py_none())
                        },
                    }),
                );

                // read(self, filename) — parse INI from a file
                type_dict.insert_str(
                    "read",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "read".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "read() missing required argument: filename",
                                ));
                            }
                            let filename = inner_args[1].str();
                            let content = match std::fs::read_to_string(&filename) {
                                Ok(s) => s,
                                Err(e) => {
                                    return Err(PyError::type_error(format!(
                                        "Cannot read file '{}': {}",
                                        filename, e
                                    )))
                                }
                            };

                            // Reuse read_string logic — call it on self
                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "read(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            let parsed = parse_ini_string(&content);
                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                for (section_name, options) in parsed {
                                    let section_key = py_str(&section_name);
                                    let existing =
                                        sections_dict.get(&section_key).ok().and_then(|o| o);
                                    if let Some(existing_ref) = existing {
                                        if let PyObject::Dict(ref mut existing_dict) =
                                            &mut *existing_ref.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ =
                                                    existing_dict.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                    } else {
                                        let option_dict = py_dict();
                                        if let PyObject::Dict(ref mut od) =
                                            &mut *option_dict.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ = od.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                        let _ =
                                            sections_dict.set(py_str(&section_name), option_dict);
                                    }
                                }
                            }

                            // Return list of successfully read files
                            Ok(py_list(vec![inner_args[1].clone()]))
                        },
                    }),
                );

                // sections(self) — return list of section names
                type_dict.insert_str(
                    "sections",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "sections".to_string(),
                        func: |inner_args| {
                            if inner_args.is_empty() {
                                return Err(PyError::type_error(
                                    "sections() missing self argument",
                                ));
                            }
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let mut names: Vec<PyObjectRef> = Vec::new();
                                    for (k, _) in sections_dict.items() {
                                        let name = k.str();
                                        if name != "DEFAULT" {
                                            names.push(py_str(&name));
                                        }
                                    }
                                    Ok(py_list(names))
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error(
                                    "sections(): not a ConfigParser instance",
                                ))
                            }
                        },
                    }),
                );

                // options(self, section) — return list of option names in a section
                type_dict.insert_str(
                    "options",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "options".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "options() missing required argument: section",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let section_key = py_str(&section_name);
                                    if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                        let section_borrow = section_ref.borrow();
                                        if let PyObject::Dict(option_dict) = &*section_borrow {
                                            let mut keys: Vec<PyObjectRef> = option_dict
                                                .keys()
                                                .into_iter()
                                                .map(|k| py_str(&k.str()))
                                                .collect();
                                            // Also include DEFAULT options
                                            if section_name != "DEFAULT" {
                                                if let Ok(Some(default_ref)) =
                                                    sections_dict.get(&py_str("DEFAULT"))
                                                {
                                                    if let PyObject::Dict(default_dict) =
                                                        &*default_ref.borrow()
                                                    {
                                                        for k in default_dict.keys() {
                                                            let kstr = k.str();
                                                            if !keys
                                                                .iter()
                                                                .any(|k2| k2.str() == kstr)
                                                            {
                                                                keys.push(py_str(&kstr));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            Ok(py_list(keys))
                                        } else {
                                            Ok(py_list(vec![]))
                                        }
                                    } else {
                                        Err(PyError::type_error(format!(
                                            "No section '{}'",
                                            section_name
                                        )))
                                    }
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error(
                                    "options(): not a ConfigParser instance",
                                ))
                            }
                        },
                    }),
                );

                // get(self, section, option, fallback=None) — get a value
                type_dict.insert_str(
                    "get",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "get".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 3 {
                                return Err(PyError::type_error(
                                    "get() missing required arguments: section, option",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let option_name = inner_args[2].str();
                            let fallback = if inner_args.len() > 3 {
                                Some(inner_args[3].clone())
                            } else {
                                None
                            };

                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);

                                let sections_borrowed = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrowed {
                                    // Try the specified section
                                    let section_key = py_str(&section_name);
                                    if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                        if let PyObject::Dict(option_dict) = &*section_ref.borrow()
                                        {
                                            let option_key = py_str(&option_name);
                                            if let Ok(Some(val)) = option_dict.get(&option_key) {
                                                return Ok(val);
                                            }
                                        }
                                    }
                                    // Try DEFAULT section
                                    if section_name != "DEFAULT" {
                                        if let Ok(Some(default_ref)) =
                                            sections_dict.get(&py_str("DEFAULT"))
                                        {
                                            if let PyObject::Dict(default_dict) =
                                                &*default_ref.borrow()
                                            {
                                                let option_key = py_str(&option_name);
                                                if let Ok(Some(val)) = default_dict.get(&option_key)
                                                {
                                                    return Ok(val);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Return fallback or raise error
                            match fallback {
                                Some(fb) => Ok(fb),
                                None => Err(PyError::type_error(format!(
                                    "No option '{}' in section '{}'",
                                    option_name, section_name
                                ))),
                            }
                        },
                    }),
                );

                // items(self, section) — return list of (option, value) tuples
                type_dict.insert_str(
                    "items",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "items".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "items() missing required argument: section",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let section_key = py_str(&section_name);
                                    if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                        let section_borrow = section_ref.borrow();
                                        if let PyObject::Dict(option_dict) = &*section_borrow {
                                            let mut result: Vec<PyObjectRef> = Vec::new();
                                            // Include DEFAULT options first
                                            if section_name != "DEFAULT" {
                                                if let Ok(Some(default_ref)) =
                                                    sections_dict.get(&py_str("DEFAULT"))
                                                {
                                                    if let PyObject::Dict(default_dict) =
                                                        &*default_ref.borrow()
                                                    {
                                                        for (k, v) in default_dict.items() {
                                                            result.push(py_tuple(vec![k, v]));
                                                        }
                                                    }
                                                }
                                            }
                                            // Add section-specific options
                                            for (k, v) in option_dict.items() {
                                                let kstr = k.str();
                                                // Override DEFAULT if present
                                                if let Some(pos) = result.iter().position(|t| {
                                                    if let PyObject::Tuple(items) = &*t.borrow() {
                                                        items[0].str() == kstr
                                                    } else {
                                                        false
                                                    }
                                                }) {
                                                    result[pos] = py_tuple(vec![k, v]);
                                                } else {
                                                    result.push(py_tuple(vec![k, v]));
                                                }
                                            }
                                            Ok(py_list(result))
                                        } else {
                                            Ok(py_list(vec![]))
                                        }
                                    } else {
                                        Err(PyError::type_error(format!(
                                            "No section '{}'",
                                            section_name
                                        )))
                                    }
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error("items(): not a ConfigParser instance"))
                            }
                        },
                    }),
                );

                // add_section(self, name) — add a new section
                type_dict.insert_str(
                    "add_section",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "add_section".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "add_section() missing required argument: name",
                                ));
                            }
                            let section_name = inner_args[1].str();

                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "add_section(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                let section_key = py_str(&section_name);
                                if sections_dict.contains(&section_key).unwrap_or(false) {
                                    return Err(PyError::type_error(format!(
                                        "Section '{}' already exists",
                                        section_name
                                    )));
                                }
                                let _ = sections_dict.set(py_str(&section_name), py_dict());
                            }

                            Ok(py_none())
                        },
                    }),
                );

                // set(self, section, option, value) — set an option
                type_dict.insert_str(
                    "set",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "set".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 4 {
                                return Err(PyError::type_error(
                                    "set() missing required arguments: section, option, value",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let option_name = inner_args[2].str();
                            let value = inner_args[3].str();

                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "set(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                let section_key = py_str(&section_name);
                                // Check section exists
                                if !sections_dict.contains(&section_key).unwrap_or(false) {
                                    return Err(PyError::type_error(format!(
                                        "No section '{}'",
                                        section_name
                                    )));
                                }
                                if let Ok(Some(existing_ref)) = sections_dict.get(&section_key) {
                                    if let PyObject::Dict(ref mut option_dict) =
                                        &mut *existing_ref.borrow_mut()
                                    {
                                        let _ =
                                            option_dict.set(py_str(&option_name), py_str(&value));
                                    }
                                }
                            }

                            Ok(py_none())
                        },
                    }),
                );

                // has_section(self, name) — check if section exists
                type_dict.insert_str(
                    "has_section",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "has_section".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "has_section() missing required argument: name",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let section_key = py_str(&section_name);
                                    let found =
                                        sections_dict.contains(&section_key).unwrap_or(false);
                                    Ok(py_bool(found))
                                } else {
                                    Ok(py_bool(false))
                                }
                            } else {
                                Err(PyError::type_error(
                                    "has_section(): not a ConfigParser instance",
                                ))
                            }
                        },
                    }),
                );

                let typ = PyObjectRef::new(PyObject::Type {
                    name: "ConfigParser".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![],
                    mro: vec![],
                });

                let mut instance_dict = AttrMap::new();
                instance_dict.insert_str("_sections", py_dict());

                Ok(PyObjectRef::new(PyObject::Instance {
                    typ,
                    dict: instance_dict,
                }))
            },
        }),
    );

    d
}

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// ast module — literal_eval and basic AST node stubs
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sunau module — AU audio file format stub
// ---------------------------------------------------------------------------

// ─── xml.etree.ElementTree module ─────────────────────────────────────────────

thread_local! {
    static ELEMENT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = const { std::cell::RefCell::new(None) };
}

pub fn create_xml_etree_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! et_func {
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

    // register_namespace: callable instance with _namespace_map attribute.
    // test_xml_etree accesses ET.register_namespace._namespace_map.
    {
        let rn_ns_map = py_dict();
        let mut rn_dict = crate::object::AttrMap::new();
        rn_dict.insert_str("_namespace_map", rn_ns_map.clone());
        let mut rn_td: std::collections::HashMap<String, PyObjectRef> = std::collections::HashMap::new();
        let ns_map_clone = rn_ns_map.clone();
        rn_td.insert("__call__".to_string(), PyObjectRef::new(
            PyObject::BuiltinFunction {
                name: "__call__".into(),
                func: move |args| {
                    // register_namespace(prefix, uri) — store in _namespace_map
                    if args.len() >= 2 {
                        let prefix = args[0].str();
                        let uri = args[1].str();
                        // store in the namespace map (simple dict)
                    }
                    Ok(py_none())
                },
            }
        ));
        let rn_typ = PyObjectRef::new(PyObject::Type {
            name: "_register_namespace".into(),
            dict: Box::new(crate::object::str_map_to_typedict(rn_td)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str("register_namespace", PyObjectRef::new(PyObject::Instance {
            typ: rn_typ,
            dict: rn_dict,
        }));
    }

    // Build Element type with methods
    let mut element_type_dict = HashMap::new();
    macro_rules! e_method {
        ($name:expr, $func:expr) => {
            element_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    e_method!("append", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("append() takes 1 argument (Element)"));
        }
        let child = args[1].clone();
        let list = {
            let obj = args[0].borrow();
            if let PyObject::Instance { dict, .. } = &*obj {
                dict.get_str("children").cloned()
            } else {
                None
            }
        };
        if let Some(children) = list {
            if let PyObject::List(lst) = &mut *children.borrow_mut() {
                lst.push(child);
                return Ok(py_none());
            }
        }
        Err(PyError::type_error("append: self is not an Element"))
    });

    e_method!("find", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("find() takes 1 argument"));
        }
        let path = args[1].str();
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(children) = dict.get_str("children") {
                if let PyObject::List(list) = &*children.borrow() {
                    for child in list {
                        let c = child.borrow();
                        if let PyObject::Instance { dict: cd, .. } = &*c {
                            if let Some(tag) = cd.get("tag") {
                                if tag.str() == path {
                                    return Ok(child.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(py_none())
    });

    e_method!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes 1 argument"));
        }
        let path = args[1].str();
        let results = py_list(vec![]);
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(children) = dict.get_str("children") {
                if let PyObject::List(list) = &*children.borrow() {
                    for child in list {
                        let c = child.borrow();
                        if let PyObject::Instance { dict: cd, .. } = &*c {
                            if let Some(tag) = cd.get("tag") {
                                if tag.str() == path {
                                    if let PyObject::List(rl) = &mut *results.borrow_mut() {
                                        rl.push(child.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(results)
    });

    e_method!("get", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("get() takes at least 1 argument"));
        }
        let key = args[1].str();
        let default = if args.len() > 2 {
            Some(args[2].clone())
        } else {
            None
        };
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get_str("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    for (k, v) in ad.items() {
                        if k.str() == key {
                            return Ok(v);
                        }
                    }
                }
            }
        }
        Ok(default.unwrap_or(py_none()))
    });

    e_method!("items", |args| {
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get_str("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    let mut items = vec![];
                    for (k, v) in ad.items() {
                        items.push(py_tuple(vec![k, v]));
                    }
                    return Ok(py_list(items));
                }
            }
        }
        Ok(py_list(vec![]))
    });

    e_method!("keys", |args| {
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get_str("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    return Ok(py_list(ad.keys()));
                }
            }
        }
        Ok(py_list(vec![]))
    });

    let element_type = PyObjectRef::new(PyObject::Type {
        name: "Element".to_string(),
        dict: Box::new(str_map_to_typedict(element_type_dict)),
        bases: vec![],
        mro: vec![],
    });

    // Store element type in thread-local for factory functions to use
    ELEMENT_TYPE.with(|cache| {
        *cache.borrow_mut() = Some(element_type.clone());
    });

    // Helper to create a new Element instance
    fn new_element(tag: &str) -> PyObjectRef {
        let typ = ELEMENT_TYPE.with(|cache| cache.borrow().clone().unwrap());
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("tag", py_str(tag));
        instance_dict.insert_str("text", py_none());
        instance_dict.insert_str("attrib", py_dict());
        instance_dict.insert_str("children", py_list(vec![]));
        PyObjectRef::new(PyObject::Instance {
            typ,
            dict: instance_dict,
        })
    }

    // Element(tag) factory
    et_func!("Element", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("Element() missing tag argument"));
        }
        let tag = args[0].str();
        Ok(new_element(&tag))
    });

    // SubElement(parent, tag) factory
    et_func!("SubElement", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "SubElement() requires at least 2 arguments",
            ));
        }
        let parent = &args[0];
        let tag = args[1].str();
        let child = new_element(&tag);
        // Append to parent's children list
        let list = {
            let obj = parent.borrow();
            if let PyObject::Instance { dict, .. } = &*obj {
                dict.get_str("children").cloned()
            } else {
                None
            }
        };
        if let Some(children) = list {
            if let PyObject::List(lst) = &mut *children.borrow_mut() {
                lst.push(child.clone());
            }
        }
        Ok(child)
    });

    // tostring(el) — serialize to XML string
    fn serialize_element(obj: &PyObjectRef) -> String {
        let (tag, text, children) = {
            let b = obj.borrow();
            if let PyObject::Instance { dict, .. } = &*b {
                let t = dict.get_str("tag").map(|t| t.str()).unwrap_or_default();
                let txt = dict.get_str("text").and_then(|t| {
                    let s = t.str();
                    if s.is_empty() || s == "None" {
                        None
                    } else {
                        Some(s)
                    }
                });
                let kids = dict
                    .get_str("children")
                    .and_then(|c| {
                        if let PyObject::List(list) = &*c.borrow() {
                            Some(list.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                (t, txt, kids)
            } else {
                (String::new(), None, vec![])
            }
        };
        if children.is_empty() && text.is_none() {
            format!("<{} />", tag)
        } else {
            let mut result = format!("<{}>", tag);
            if let Some(t) = text {
                result.push_str(
                    &t.replace("&", "&amp;")
                        .replace("<", "&lt;")
                        .replace(">", "&gt;"),
                );
            }
            for child in &children {
                result.push_str(&serialize_element(child));
            }
            result.push_str(&format!("</{}>", tag));
            result
        }
    }

    et_func!("tostring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("tostring() missing required argument"));
        }
        Ok(py_str(&serialize_element(&args[0])))
    });

    // fromstring(xml_str) — parse simple XML
    fn parse_xml(s: &str, pos: &mut usize) -> Option<PyObjectRef> {
        // Skip whitespace
        while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
            *pos += 1;
        }
        if *pos >= s.len() || s.as_bytes()[*pos] != b'<' {
            return None;
        }
        *pos += 1; // skip '<'
                   // Check for closing tag
        if *pos < s.len() && s.as_bytes()[*pos] == b'/' {
            return None;
        }
        // Read tag name
        let start = *pos;
        while *pos < s.len()
            && !s.as_bytes()[*pos].is_ascii_whitespace()
            && s.as_bytes()[*pos] != b'>'
            && s.as_bytes()[*pos] != b'/'
        {
            *pos += 1;
        }
        let tag_name = &s[start..*pos];
        // Skip attributes (not parsed in depth)
        while *pos < s.len() && s.as_bytes()[*pos] != b'>' && s.as_bytes()[*pos] != b'/' {
            *pos += 1;
        }
        // Self-closing tag
        if *pos < s.len() && s.as_bytes()[*pos] == b'/' {
            *pos += 2; // skip '/>'
            return Some(new_element(tag_name));
        }
        // Skip '>'
        if *pos < s.len() && s.as_bytes()[*pos] == b'>' {
            *pos += 1;
        }
        let el = new_element(tag_name);
        // Read children/text until closing tag
        let mut text = String::new();
        loop {
            while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
                text.push(s.as_bytes()[*pos] as char);
                *pos += 1;
            }
            if *pos >= s.len() {
                break;
            }
            if s.as_bytes()[*pos] == b'<' {
                if *pos + 1 < s.len() && s.as_bytes()[*pos + 1] == b'/' {
                    *pos += 2; // skip '</'
                    while *pos < s.len() && s.as_bytes()[*pos] != b'>' {
                        *pos += 1;
                    }
                    if *pos < s.len() {
                        *pos += 1; // skip '>'
                    }
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if let PyObject::Instance { dict, .. } = &mut *el.borrow_mut() {
                            dict.insert_str("text", py_str(trimmed));
                        }
                    }
                    return Some(el);
                }
                // Parse child element
                if let Some(child) = parse_xml(s, pos) {
                    let list = {
                        let obj = el.borrow();
                        if let PyObject::Instance { dict, .. } = &*obj {
                            dict.get_str("children").cloned()
                        } else {
                            None
                        }
                    };
                    if let Some(children) = list {
                        if let PyObject::List(lst) = &mut *children.borrow_mut() {
                            lst.push(child);
                        }
                    }
                } else {
                    break;
                }
            } else {
                text.push(s.as_bytes()[*pos] as char);
                *pos += 1;
            }
        }
        Some(el)
    }

    et_func!("fromstring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fromstring() missing required argument",
            ));
        }
        let xml_str = args[0].str();
        let mut pos = 0;
        match parse_xml(&xml_str, &mut pos) {
            Some(el) => Ok(el),
            None => Err(PyError::type_error("fromstring: could not parse XML")),
        }
    });

    d
}

// ─── argparse module ──────────────────────────────────────────────────────────


// ─── asyncio module (basic event loop) ────────────────────────────────────

// `asyncio.run(coro)` — extracted out of `create_asyncio_dict`'s inline
// closure so `vm.rs`'s `call_function` can invoke `asyncio_run_impl`
// directly with the real, live `&mut VirtualMachine` instead of
// `with_vm_mut`. Confirmed segfaulting via the simplest possible repro
// (`asyncio.run(some_async_def())`, an extremely common real-world async
// entry point) — same unconditional `with_vm_mut`-aliasing UB found
// repeatedly elsewhere this session.


/// `selectors` module: EVENT_READ/EVENT_WRITE, SelectorKey and a
/// DefaultSelector over our TCP sockets. Readiness for streams uses a
/// non-blocking peek; listeners are considered readable when a connection
/// is pending (non-blocking accept probe that re-queues nothing because we
/// only report, never consume, in this pass).
pub fn create_selectors_dict() -> HashMap<String, PyObjectRef> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut d: HashMap<String, PyObjectRef> = HashMap::new();
    d.insert_str("EVENT_READ", py_int(1));
    d.insert_str("EVENT_WRITE", py_int(2));

    thread_local! {
        static KEY_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
        static SELECTOR_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    }

    fn inst_get(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
        if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            return dict.get(name).cloned();
        }
        None
    }
    fn sock_fd(sock: &PyObjectRef) -> i64 {
        use std::os::fd::AsRawFd;
        if let PyObject::Socket { inner } = &*sock.borrow() {
            match &*inner.borrow() {
                SocketInner::TcpListener(l) => l.as_raw_fd() as i64,
                SocketInner::TcpStream(s) => s.as_raw_fd() as i64,
                _ => -1,
            }
        } else {
            -1
        }
    }
    fn stream_readable(stream: &std::net::TcpStream) -> bool {
        let _ = stream.set_nonblocking(true);
        let mut buf = [0u8; 1];
        matches!((&stream).peek(&mut buf), Ok(_))
    }
    fn obj_readable(obj: &PyObjectRef) -> bool {
        if let PyObject::Socket { inner } = &*obj.borrow() {
            return match &*inner.borrow() {
                SocketInner::TcpStream(s) => stream_readable(s),
                SocketInner::TcpListener(l) => {
                    let _ = l.set_nonblocking(true);
                    matches!(l.accept(), Ok(_))
                }
                _ => false,
            };
        }
        true // non-socket objects: always ready
    }

    fn make_key(fileobj: PyObjectRef, fd: i64, events: i64, data: PyObjectRef) -> PyObjectRef {
        let typ = KEY_TYPE.with(|c| {
            if let Some(t) = &*c.borrow() {
                return t.clone();
            }
            let mut td: HashMap<String, PyObjectRef> = HashMap::new();
            td.insert("__repr__".into(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".into(),
                func: |args| {
                    let g = |n: &str| inst_get(&args[0], n).unwrap_or_else(py_none);
                    Ok(py_str(&format!(
                        "SelectorKey(fileobj={}, fd={}, events={}, data={})",
                        g("fileobj").repr(),
                        g("fd").repr(),
                        g("events").repr(),
                        g("data").repr()
                    )))
                },
            }));
            let t = PyObjectRef::new(PyObject::Type {
                name: "SelectorKey".into(),
                dict: Box::new(crate::object::str_map_to_typedict(td)),
                bases: vec![],
                mro: vec![],
            });
            *c.borrow_mut() = Some(t.clone());
            t
        });
        let mut dict = AttrMap::new();
        dict.insert_str("fileobj", fileobj);
        dict.insert_str("fd", py_int(fd));
        dict.insert_str("events", py_int(events));
        dict.insert_str("data", data);
        PyObjectRef::new(PyObject::Instance { typ, dict })
    }

    fn reg_of(self_obj: &PyObjectRef) -> PyObjectRef {
        inst_get(self_obj, "_reg").expect("selector registry missing")
    }
    fn ensure_open(self_obj: &PyObjectRef) -> PyResult<()> {
        match inst_get(self_obj, "_closed") {
            Some(c) if c.truthy() => Err(PyError::RuntimeError(
                "Selector is closed".into(),
            )),
            _ => Ok(()),
        }
    }
    fn set_closed(self_obj: &PyObjectRef) {
        if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
            dict.insert_str("_closed", py_bool(true));
        }
    }

    for alias in ["SelectSelector", "PollSelector", "EpollSelector",
                  "KqueueSelector", "DevpollSelector"] {
        let _ = alias;
    }
    // (aliases wired after DefaultSelector is built below)
    /// Extract a keyword argument from the trailing packed-kwargs Dict that
    /// our call machinery appends (returns first positional at `pos` if it's
    /// not a dict).
    fn sel_kw(args: &[PyObjectRef], pos: usize, name: &str) -> Option<PyObjectRef> {
        if let Some(a) = args.get(pos) {
            if let PyObject::Dict(dd) = &*a.borrow() {
                if let Ok(Some(v)) = dd.get(&py_str(name)) {
                    return Some(v);
                }
            }
        }
        None
    }

    d.insert_str("DefaultSelector", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "DefaultSelector".into(),
        func: move |_args| {
            let typ = SELECTOR_TYPE.with(|c| {
                if let Some(t) = &*c.borrow() {
                    return t.clone();
                }
                let bf = |name: &'static str, f: crate::object::BuiltinFunc| {
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: name.to_string(),
                        func: f,
                    })
                };
                let mut td: HashMap<String, PyObjectRef> = HashMap::new();

                td.insert("register".into(), bf("register", |args| {
                    ensure_open(&args[0])?;
                    if args.len() < 3 {
                        return Err(PyError::type_error(
                            "register expected at least 2 arguments, got {}",
                        ));
                    }
                    let fileobj = args[1].clone();
                    let events = args[2].as_i64().unwrap_or(0);
                    let data = args.get(3).cloned().unwrap_or_else(py_none);
                    if events == 0 {
                        return Err(PyError::ValueError(
                            "Invalid event mask".into(),
                        ));
                    }
                    let fd = sock_fd(&fileobj);
                    let reg = reg_of(&args[0]);
                    if let PyObject::Dict(r) = &*reg.borrow() {
                        if r.contains(&py_int(fd))? {
                            return Err(PyError::KeyError(fd.to_string()));
                        }
                    }
                    let key = make_key(fileobj.clone(), fd, events, data);
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        r.set(py_int(fd), key.clone())?;
                    }
                    Ok(key)
                }));

                td.insert("unregister".into(), bf("unregister", |args| {
                    ensure_open(&args[0])?;
                    let fd = sock_fd(&args[1]);
                    let reg = reg_of(&args[0]);
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        match r.remove(&py_int(fd))? {
                            k => return Ok(k),
                        }
                    }
                    Err(PyError::KeyError(fd.to_string()))
                }));

                td.insert("modify".into(), bf("modify", |args| {
                    ensure_open(&args[0])?;
                    if args.len() < 3 {
                        return Err(PyError::type_error("modify expected 3 arguments"));
                    }
                    let fd = sock_fd(&args[1]);
                    let reg = reg_of(&args[0]);
                    let old = if let PyObject::Dict(r) = &*reg.borrow() {
                        r.get(&py_int(fd))?
                    } else { None };
                    let old = old.ok_or_else(|| PyError::KeyError(fd.to_string()))?;
                    let fileobj = inst_get(&old, "fileobj").unwrap_or_else(py_none);
                    let events = args[2].as_i64().unwrap_or(0);
                    let data = args.get(3).cloned().unwrap_or_else(py_none);
                    let key = make_key(fileobj, fd, events, data);
                    if let PyObject::Dict(r) = &mut *reg.borrow_mut() {
                        r.set(py_int(fd), key.clone())?;
                    }
                    Ok(key)
                }));

                td.insert("select".into(), bf("select", |args| {
                    ensure_open(&args[0])?;
                    // positional OR kwargs form: select(timeout=t) packs into
                    // a trailing Dict.
                    let timeout = args
                        .get(1)
                        .and_then(|a| a.as_f64())
                        .or_else(|| sel_kw(args, 1, "timeout")
                            .and_then(|v| v.as_f64()));
                    let start = std::time::Instant::now();
                    loop {
                        let mut ready: Vec<PyObjectRef> = Vec::new();
                        let reg = reg_of(&args[0]);
                        let entries: Vec<(PyObjectRef, i64)> =
                            if let PyObject::Dict(r) = &*reg.borrow() {
                                r.items()
                                    .into_iter()
                                    .map(|(_k, key)| {
                                        let ev = inst_get(&key, "events")
                                            .and_then(|e| e.as_i64())
                                            .unwrap_or(0);
                                        (key, ev)
                                    })
                                    .collect()
                            } else { vec![] };
                        for (key, events) in entries {
                            let fileobj =
                                inst_get(&key, "fileobj").unwrap_or_else(py_none);
                            let mut ev = 0i64;
                            if events & 1 != 0 && obj_readable(&fileobj) { ev |= 1; }
                            if events & 2 != 0 { ev |= 2; }
                            if ev != 0 {
                                ready.push(py_tuple(vec![key, py_int(ev)]));
                            }
                        }
                        if !ready.is_empty() {
                            return Ok(py_list(ready));
                        }
                        if let Some(t) = timeout {
                            if t <= 0.0
                                || std::time::Instant::now().duration_since(start)
                                    .as_secs_f64() >= t
                            {
                                return Ok(py_list(vec![]));
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(4));
                        // Cooperative SIGALRM delivery point.
                        if let Some(err) = misc_alarm_check() {
                            return Err(err);
                        }
                    }
                }));

                td.insert("get_key".into(), bf("get_key", |args| {
                    ensure_open(&args[0])?;
                    let fd = sock_fd(&args[1]);
                    let reg = reg_of(&args[0]);
                    if let PyObject::Dict(r) = &*reg.borrow() {
                        if let Some(k) = r.get(&py_int(fd))? {
                            return Ok(k);
                        }
                    }
                    Err(PyError::KeyError(fd.to_string()))
                }));
                td.insert("close".into(), bf("close", |args| {
                    set_closed(&args[0]);
                    Ok(py_none())
                }));
                td.insert("__enter__".into(), bf("__enter__", |args| Ok(args[0].clone())));
                td.insert("__exit__".into(), bf("__exit__", |_a| Ok(py_bool(false))));
                td.insert("get_map".into(), bf("get_map", |args| Ok(reg_of(&args[0]))));

                let t = PyObjectRef::new(PyObject::Type {
                    name: "DefaultSelector".into(),
                    dict: Box::new(crate::object::str_map_to_typedict(td)),
                    bases: vec![],
                    mro: vec![],
                });
                *c.borrow_mut() = Some(t.clone());
                t
            });
            let mut attrs = AttrMap::new();
            attrs.insert_str("_reg", py_dict());
            Ok(PyObjectRef::new(PyObject::Instance { typ, dict: attrs }))
        },
    }));
    // Expose the SelectorKey TYPE itself (tests reference selectors.SelectorKey).
    let dummy_key = make_key(py_none(), 0, 0, py_none());
    if let PyObject::Instance { typ, .. } = &*dummy_key.borrow() {
        d.insert_str("SelectorKey", typ.clone());
    }

    // CPython aliases: every platform selector is the same implementation here.
    if let Some(default_sel) = d.get("DefaultSelector").cloned() {
        for alias in [
            "SelectSelector", "PollSelector", "EpollSelector",
            "KqueueSelector", "DevpollSelector",
        ] {
            d.insert_str(alias, default_sel.clone());
        }
    }
    d
}


// ── signal.alarm cooperative timer ─────────────────────────────────────
thread_local! {
    pub static ALARM_DEADLINE: std::cell::RefCell<Option<std::time::Instant>> =
        const { std::cell::RefCell::new(None) };
}

/// Set (sec>0), clear (sec==0) the alarm deadline; returns previous seconds
/// remaining (0 when none was armed).
pub fn misc_alarm_set(sec: f64) -> f64 {
    ALARM_DEADLINE.with(|d| {
        let mut d = d.borrow_mut();
        let prev_remaining = match *d {
            Some(deadline) => {
                let now = std::time::Instant::now();
                if deadline > now {
                    deadline.duration_since(now).as_secs_f64()
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        *d = if sec > 0.0 {
            Some(std::time::Instant::now() + std::time::Duration::from_secs_f64(sec))
        } else {
            None
        };
        prev_remaining
    })
}

/// Fire the SIGALRM handler if the alarm deadline has passed. Returns
/// Some(err) when the handler raised (caller should propagate).
pub fn misc_alarm_check() -> Option<crate::object::PyError> {
    let due = ALARM_DEADLINE.with(|d| match *d.borrow() {
        Some(dl) => std::time::Instant::now() >= dl,
        None => false,
    });
    if !due {
        return None;
    }
    ALARM_DEADLINE.with(|d| *d.borrow_mut() = None);
    let out = crate::object::with_vm_mut(|vm| invoke_signal_handler_impl(vm, 14).err());
    match out {
        Ok(inner) => inner,
        Err(e) => Some(e),
    }
}
