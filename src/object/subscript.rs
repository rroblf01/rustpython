// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds subscript
// access (`__getitem__`/`__setitem__`/`__delitem__` dispatch across
// list/dict/str/bytes/tuple/user-defined classes).
use super::*;

// ---- Subscript access ----

pub fn to_index(obj: &PyObjectRef) -> PyResult<BigInt> {
    let type_name = obj.get_type_name();
    let is_instance = matches!(&*obj.borrow(), PyObject::Instance { .. });
    if is_instance {
        let f = {
            let o = obj.borrow();
            match &*o {
                PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__index__"),
                _ => None,
            }
        };
        if let Some(f) = f {
            let result = call_bound_method(f, obj.clone(), vec![])?;
            let r = result.borrow();
            match &*r {
                PyObject::Int(i) => Ok(i.clone()),
                // `bool` is a genuine `int` subclass in real Python, so a
                // `__index__` returning `True`/`False` is valid (if
                // deprecated in modern CPython) — matches the native-`bool`
                // arm added just below for the same reason.
                PyObject::Bool(b) => Ok(BigInt::from(*b as i64)),
                _ => Err(PyError::type_error("__index__ must return int")),
            }
        } else {
            Err(PyError::type_error(format!("'{}' object cannot be interpreted as an integer", type_name)))
        }
    } else {
        let o = obj.borrow();
        match &*o {
            PyObject::Int(i) => Ok(i.clone()),
            // `bool` is a subtype of `int` in real Python (`range(True) ==
            // range(1)`, `[10, 20][False]`, etc.) — found via `range()`'s
            // own `__index__`-protocol fix above surfacing this same gap.
            PyObject::Bool(b) => Ok(BigInt::from(*b as i64)),
            _ => Err(PyError::type_error(format!("'{}' object cannot be interpreted as an integer", type_name))),
        }
    }
}

/// Plain-value equivalent of `to_index` for the many sequence-indexing sites
/// below (`list`/`tuple`/`str`/`bytes`/`bytearray`/`array`/`range`) that
/// already have `index` borrowed as a `PyObject` and just need "is this an
/// int (or bool, a genuine int subtype) at all" without the `__index__`-via-
/// mro dispatch `to_index` also does (those sites fall back to a `Slice`
/// arm too, which `to_index` doesn't know about). Found via `list[True]`
/// (and the tuple/str/bytes/bytearray/array/range equivalents) all raising
/// `TypeError: ... indices must be integers or slices` despite `bool` being
/// a valid index in real Python — same root gap as `range()`'s own
/// `__index__`/bool fix just above, just for indexing instead of construction.
fn sequence_index_int(idx: &PyObject) -> Option<BigInt> {
    match idx {
        PyObject::Int(i) => Some(i.clone()),
        PyObject::Bool(b) => Some(BigInt::from(*b as i64)),
        _ => None,
    }
}

/// Real Python slice-index normalization for a sequence of length `len` —
/// mirrors CPython's own `PySlice_GetIndicesEx`. Converts a possibly-
/// negative, possibly-omitted (`None`) start/stop pair into concrete,
/// in-bounds `isize` values a caller can safely loop
/// `while i (< or >) stop { ...; i += step }` over and cast to `usize`
/// without ever going negative.
///
/// Was NOT applied consistently anywhere in this file before this fix:
/// `List`/`Tuple` read-slicing did `start_val.max(0).min(len)` — clamping a
/// negative value straight to 0 instead of first adding `len` (so
/// `[1,2,3,4,5][-3:]`, meaning "last 3 elements", silently returned the
/// WHOLE list instead — a silent wrong-answer bug, not a crash). `Str`/
/// `Bytes`/`ByteArray` read-slicing did no clamping at all, so a negative
/// start/stop was cast straight from a negative `isize` to `usize`,
/// wrapping around to an astronomical value and panicking on the first
/// array access (confirmed via the simplest possible repro: `"hello"[-3:]`
/// crashed the whole process). Negative slice bounds are one of the most
/// common idioms in all of Python (`seq[:-1]`, `seq[-n:]`) — this was a
/// severe, high-blast-radius bug hiding in plain sight.
pub(crate) fn normalize_slice_bounds(start: Option<isize>, stop: Option<isize>, step: isize, len: usize) -> (isize, isize) {
    let len = len as isize;
    if step > 0 {
        let start = match start {
            None => 0,
            Some(v) if v < 0 => (len + v).max(0),
            Some(v) => v.min(len),
        };
        let stop = match stop {
            None => len,
            Some(v) if v < 0 => (len + v).max(0),
            Some(v) => v.min(len),
        };
        (start, stop)
    } else {
        let start = match start {
            None => len - 1,
            Some(v) if v < 0 => (len + v).max(-1),
            Some(v) => v.min(len - 1),
        };
        let stop = match stop {
            None => -1,
            Some(v) if v < 0 => (len + v).max(-1),
            Some(v) => v.min(len - 1),
        };
        (start, stop)
    }
}

/// Extracts `(start, stop, step)` as `Option<isize>`/`isize` from a
/// `PyObject::Slice`'s three borrowed fields, ready to hand to
/// `normalize_slice_bounds`.
///
/// Rejects a literal `step=0` with a real `ValueError` — this interpreter's
/// `slice()`/`BUILD_SLICE` construction does NOT reject it up front (unlike
/// what an earlier version of this comment assumed), so `some_list[::0]`
/// previously reached the iteration loops below with `step_val = 0` and
/// hung the whole process forever (`i += 0` never advances past `stop_n`,
/// an infinite loop — confirmed via the simplest repro, `[1,2,3][::0]`).
/// Real CPython raises `ValueError: slice step cannot be zero` at the point
/// a zero-step slice is actually USED for indexing, matched here.
pub(crate) fn extract_slice_fields(start: &PyObjectRef, stop: &PyObjectRef, step: &PyObjectRef) -> PyResult<(Option<isize>, Option<isize>, isize)> {
    let step_val = if let PyObject::Int(i) = &*step.borrow() { i.to_isize().unwrap_or(1) } else { 1 };
    if step_val == 0 {
        return Err(PyError::value_error("slice step cannot be zero"));
    }
    let start_val = if let PyObject::Int(i) = &*start.borrow() { i.to_isize() } else { None };
    let stop_val = if let PyObject::Int(i) = &*stop.borrow() { i.to_isize() } else { None };
    Ok((start_val, stop_val, step_val))
}

pub fn py_getitem(obj: &PyObjectRef, index: &PyObjectRef) -> PyResult<PyObjectRef> {
    // Check for __getitem__ on custom classes and __class_getitem__ on types (PEP 560)
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Type { dict: type_dict, mro, .. } => {
                // Real Python checks the metaclass's `__getitem__` first
                // (subscripting a class object is, at bottom, calling
                // `type(cls).__getitem__(cls, key)`) — e.g. enum's
                // `EnumType.__getitem__` for `Color['RED']` name lookup.
                // `__class_getitem__` (PEP 560, `list[int]`-style generic
                // aliasing) is the fallback for classes with no such
                // metaclass method.
                let metatype_getitem = metatype_of(obj).and_then(|mt| lookup_dunder_via_mro(&mt, "__getitem__"));
                metatype_getitem.or_else(|| {
                    type_dict.get_str("__class_getitem__").cloned().or_else(|| {
                        for base in mro.iter().skip(1) {
                            if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                if let Some(val) = base_dict.get_str("__class_getitem__") {
                                    return Some(val.clone());
                                }
                            }
                        }
                        None
                    })
                })
            }
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, mro, .. } => {
                        type_dict.get_str("__getitem__").cloned().or_else(|| {
                            // Skip a migrated native type's OWN `__getitem__`
                            // "escape hatch" entry when scanning ANCESTORS —
                            // see `lookup_dunder_via_mro`'s matching
                            // `skip_native_dunder_hatch` doc comment
                            // (`descriptors.rs`) for the full rationale: it
                            // exists for explicit unbound-style access, not
                            // to preempt a native-base subclass's ordinary
                            // instance-level dispatch (native-backing
                            // delegation + `__missing__`), which is what the
                            // fallback below this closure already handles
                            // correctly.
                            let skip_native_dunder_hatch = type_dict.contains_key_str(NATIVE_BASE_MARKER);
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if skip_native_dunder_hatch && base_dict.contains_key_str(NATIVE_VALUE_CTOR_KEY) {
                                        continue;
                                    }
                                    if let Some(val) = base_dict.get_str("__getitem__") {
                                        return Some(val.clone());
                                    }
                                }
                            }
                            None
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, obj.clone(), vec![index.clone()]);
    }
    // Not overridden anywhere in the mro: for a class that transparently
    // subclasses list/dict/str (`class Foo(list): ...`), delegate straight
    // to the native backing's own subscripting. For a dict subclass, a
    // missing key must still go through the class's own `__missing__`
    // (e.g. `collections.Counter`) before raising KeyError.
    if let Some(native) = native_backing_of(obj) {
        return match py_getitem(&native, index) {
            Err(PyError::KeyError(_)) => {
                let missing_fn = match &*obj.borrow() {
                    PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__missing__"),
                    _ => None,
                };
                match missing_fn {
                    Some(f) => call_bound_method(f, obj.clone(), vec![index.clone()]),
                    None => Err(PyError::key_error(index.str())),
                }
            }
            other => other,
        };
    }
    if matches!(&*obj.borrow(), PyObject::MemoryView { .. }) {
        return mv_getitem(obj, index);
    }
    // Dict lookups: compute the key's hash BEFORE taking `obj`'s own borrow
    // (see `PyDict::set_with_hash`'s doc comment) — a key with a custom
    // `__hash__` can run arbitrary Python, including code that mutates this
    // very dict, which would re-enter `borrow()`/`borrow_mut()` on the same
    // RefCell and panic if the hash were computed while already borrowed.
    if matches!(&*obj.borrow(), PyObject::Dict(_)) {
        let h = index.hash()?;
        let o = obj.borrow();
        if let PyObject::Dict(d) = &*o {
            return match d.get_with_hash(index, h) {
                Some(val) => Ok(val),
                None => Err(PyError::key_error(index.str())),
            };
        }
    }
    let o = obj.borrow();
    match &*o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("list index out of range"));
                }
                return Ok(items[i as usize].clone());
            }
            match &*idx {
                PyObject::Slice { start, stop, step } => {
                    let mut result = Vec::new();
                    let len = items.len();
                    let (start_val, stop_val, step_val) = extract_slice_fields(start, stop, step)?;
                    let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    } else {
                        while i > stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    }
                    Ok(py_list(result))
                }
                _ => Err(PyError::type_error(format!("list indices must be integers or slices, not {}", idx.type_name()))),
            }
        }
        PyObject::Deque { data, .. } => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("deque index out of range"))?;
                let len = data.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("deque index out of range"));
                }
                return Ok(data[i as usize].clone());
            }
            match &*idx {
                PyObject::Slice { start, stop, step } => {
                    let mut result = VecDeque::new();
                    let len = data.len();
                    let (start_val, stop_val, step_val) = extract_slice_fields(start, stop, step)?;
                    let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push_back(data[i as usize].clone());
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    } else {
                        while i > stop_n {
                            result.push_back(data[i as usize].clone());
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    }
                    // Real CPython's `deque.__getitem__(slice)` returns a
                    // plain (maxlen=None) deque.
                    Ok(py_deque(result, None))
                }
                _ => Err(PyError::type_error(format!("deque indices must be integers or slices, not {}", idx.type_name()))),
            }
        }
        PyObject::Tuple(items) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("tuple index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("tuple index out of range"));
                }
                return Ok(items[i as usize].clone());
            }
            match &*idx {
                PyObject::Slice { start, stop, step } => {
                    let mut result = Vec::new();
                    let len = items.len();
                    let (start_val, stop_val, step_val) = extract_slice_fields(start, stop, step)?;
                    let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    } else {
                        while i > stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    }
                    Ok(py_tuple(result))
                }
                _ => {
                    Err(PyError::type_error(format!("tuple indices must be integers or slices, not {}", idx.type_name())))
                }
            }
        }
        PyObject::Str(s) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let chars: Vec<char> = s.chars().collect();
                let i = i.to_isize().ok_or_else(|| PyError::index_error("string index out of range"))?;
                let len = chars.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("string index out of range"));
                }
                return Ok(py_str(&chars[i as usize].to_string()));
            }
            match &*idx {
                PyObject::Slice { start, stop, step } => {
                    let chars: Vec<char> = s.chars().collect();
                    let len = chars.len();
                    let (start_val, stop_val, step_val) = extract_slice_fields(start, stop, step)?;
                    let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut result = String::new();
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(chars[i as usize]);
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    } else {
                        while i > stop_n {
                            result.push(chars[i as usize]);
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    }
                    Ok(py_str(&result))
                }
                _ => Err(PyError::type_error(format!("string indices must be integers or slices, not {}", idx.type_name()))),
            }
        }
        // PyObject::Dict is handled above, before this borrow is taken.
        PyObject::Bytes(b) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("bytes index out of range"))?;
                let len = b.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("bytes index out of range"));
                }
                // Real CPython: `bytes[int]` returns an `int` (0-255);
                // only `bytes[slice]` returns a `bytes` object. This
                // returned a length-1 `bytes` for a plain int index
                // instead — silently broke any code doing byte-at-a-time
                // processing via `b[i]` (as opposed to `for byte in b`,
                // which already correctly yielded ints).
                return Ok(py_int(b[i as usize] as i64));
            }
            match &*idx {
                PyObject::Slice { start, stop, step } => {
                    let len = b.len();
                    let (start_val, stop_val, step_val) = extract_slice_fields(start, stop, step)?;
                    let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut result = Vec::new();
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    } else {
                        while i > stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    }
                    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                }
                _ => Err(PyError::type_error(format!("bytes indices must be integers or slices, not {}", idx.type_name()))),
            }
        }
        PyObject::ByteArray(b) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("bytearray index out of range"))?;
                let len = b.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("bytearray index out of range"));
                }
                // Same fix as `bytes[int]` above: a plain int index
                // must yield an `int`, not a length-1 `bytearray`.
                return Ok(py_int(b[i as usize] as i64));
            }
            match &*idx {
                PyObject::Slice { start, stop, step } => {
                    let len = b.len();
                    let (start_val, stop_val, step_val) = extract_slice_fields(start, stop, step)?;
                    let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut result = Vec::new();
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    } else {
                        while i > stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) { Some(next) => i = next, None => break };
                        }
                    }
                    Ok(PyObjectRef::new(PyObject::ByteArray(result)))
                }
                _ => Err(PyError::type_error(format!("bytearray indices must be integers or slices, not {}", idx.type_name()))),
            }
        }
        PyObject::Array(arr) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("array index out of range"))?;
                let len = arr.data.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("array index out of range"));
                }
                let v = arr.data[i as usize];
                if array_typecode_is_float(arr.typecode) {
                    Ok(py_float(v))
                } else {
                    Ok(py_int(v as i64))
                }
            } else {
                Err(PyError::type_error(format!("array indices must be integers, not {}", idx.type_name())))
            }
        }
        PyObject::Range { start, stop, step } => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let len = if *step > 0 && *start >= *stop { 0 }
                    else if *step < 0 && *start <= *stop { 0 }
                    else {
                        let raw_len = stop.checked_sub(*start).unwrap_or(i64::MAX);
                        let l = raw_len.checked_div(*step).unwrap_or(0);
                        if raw_len % *step != 0 { l.abs() + 1 } else { l.abs() }
                    };
                let i64_val = i.to_i64().unwrap_or(0);
                let pos = if i64_val < 0 { len + i64_val } else { i64_val };
                if pos < 0 || pos >= len {
                    return Err(PyError::index_error("range object index out of range"));
                }
                return Ok(py_int(*start + *step * pos));
            }
            match &*idx {
                PyObject::Slice { start: s, stop: e, step: p } => {
                    let len = if *step > 0 && *start >= *stop { 0 }
                        else if *step < 0 && *start <= *stop { 0 }
                        else {
                            let raw_len = stop.checked_sub(*start).unwrap_or(i64::MAX);
                            let l = raw_len.checked_div(*step).unwrap_or(0);
                            if raw_len % *step != 0 { l.abs() + 1 } else { l.abs() }
                        };
                    let sp = p.as_i64().unwrap_or(1);
                    let s_start = match &*s.borrow() { PyObject::None => if sp > 0 { 0 } else { len - 1 }, _ => s.as_i64().unwrap_or(0) };
                    let s_stop = match &*e.borrow() { PyObject::None => if sp > 0 { len } else { -len - 1 }, _ => e.as_i64().unwrap_or(0) };
                    let s_step = if sp == 0 { 1 } else { sp };
                    let norm_start = if s_start < 0 { (len + s_start).max(0) } else { s_start.min(len) };
                    let norm_stop = if s_stop < 0 { (len + s_stop).max(0) } else { s_stop.min(len) };
                    let new_start = *start + norm_start * *step;
                    let new_step = *step * s_step;
                    let new_stop = *start + norm_stop * *step;
                    Ok(PyObjectRef::imm(PyObject::Range { start: new_start, stop: new_stop, step: new_step }))
                }
                _ => Err(PyError::type_error(format!("range indices must be integers or slices, not {}", idx.type_name()))),
            }
        }
        PyObject::Instance { dict, .. } => {
            let key = index.str();
            let val = dict.get(&key).cloned();
            drop(o);
            if let Some(v) = val {
                Ok(v)
            } else {
                // Check for __missing__ (dict subclass support, e.g. Counter)
                let missing = obj.borrow().get_attribute("__missing__").ok()
                    .and_then(|m| crate::object::call_function(&m, vec![obj.clone(), index.clone()]).ok());
                match missing {
                    Some(v) => Ok(v),
                    None => Err(PyError::key_error(index.str())),
                }
            }
        }
        _ => Err(PyError::type_error(format!("'{}' object is not subscriptable", o.type_name()))),
    }
}

pub fn py_setitem(obj: &PyObjectRef, index: &PyObjectRef, value: PyObjectRef) -> PyResult<()> {
    // Check for __setitem__ on custom classes
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__setitem__"),
            _ => None,
        }
    };
    if let Some(f) = f {
        call_bound_method(f, obj.clone(), vec![index.clone(), value])?;
        return Ok(());
    }
    // Not overridden anywhere in the mro: for a class that transparently
    // subclasses list/dict/str (`class Foo(list): ...`), delegate straight
    // to the native backing's own item assignment. Must run before the
    // generic Instance fallback below, which would otherwise swallow this
    // into the instance's attribute dict under a stringified key instead.
    if let Some(native) = native_backing_of(obj) {
        return py_setitem(&native, index, value);
    }
    if matches!(&*obj.borrow(), PyObject::MemoryView { .. }) {
        return mv_setitem(obj, index, value);
    }
    // Default Instance __setitem__: store key/value in the instance dict (HashMap)
    {
        let o = obj.borrow();
        if let PyObject::Instance { dict: _, .. } = &*o {
            let key = index.str();
            drop(o);
            let mut o = obj.borrow_mut();
            if let PyObject::Instance { dict, .. } = &mut *o {
                dict.insert(key, value);
                return Ok(());
            }
        }
    }
    // Slice assignment on a list: collect the replacement items BEFORE taking
    // a mutable borrow below — `value` may alias `obj` (e.g. `lst[:] = lst`),
    // which would otherwise double-borrow when we iterate it.
    let is_list_slice = matches!(&*obj.borrow(), PyObject::List(_)) && matches!(&*index.borrow(), PyObject::Slice { .. });
    if is_list_slice {
        let new_items: Vec<PyObjectRef> = {
            let it = builtin_iter(&[value.clone()])?;
            let mut v = Vec::new();
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(x) => v.push(x),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
            v
        };
        let idx = index.borrow();
        let (start, stop, step) = match &*idx {
            PyObject::Slice { start, stop, step } => (start.clone(), stop.clone(), step.clone()),
            _ => unreachable!(),
        };
        drop(idx);
        let mut o = obj.borrow_mut();
        let items = match &mut *o {
            PyObject::List(items) => items,
            _ => unreachable!(),
        };
        let len = items.len();
        let (start_val, stop_val, step_val) = extract_slice_fields(&start, &stop, &step)?;
        let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
        if step_val == 1 {
            // `stop_n` can legitimately be < `start_n` here (e.g. an empty
            // slice like `lst[5:2]`) — `splice` requires a valid, ordered
            // range, so clamp it up to `start_n` in that case (matching
            // real Python's own "empty slice" semantics: nothing is
            // removed, `new_items` is just inserted at `start_n`).
            let stop_n = stop_n.max(start_n);
            items.splice(start_n as usize..stop_n as usize, new_items);
            return Ok(());
        } else {
            // Extended slice: replacement length must match slice length exactly
            let mut indices = Vec::new();
            let mut i = start_n;
            if step_val > 0 {
                while i < stop_n {
                    indices.push(i as usize);
                    match i.checked_add(step_val) { Some(next) => i = next, None => break }
                }
            } else {
                while i > stop_n {
                    indices.push(i as usize);
                    match i.checked_add(step_val) { Some(next) => i = next, None => break }
                }
            }
            if indices.len() != new_items.len() {
                return Err(PyError::value_error(format!(
                    "attempt to assign sequence of size {} to extended slice of size {}",
                    new_items.len(), indices.len()
                )));
            }
            for (idx, val) in indices.into_iter().zip(new_items) {
                items[idx] = val;
            }
            return Ok(());
        }
    }

    // Dict assignment: routed through `pydict_safe_set`, which never holds
    // `obj`'s own mutable borrow across a colliding key's `.equals()` call
    // (a key with a custom `__hash__`/`__eq__` can run arbitrary Python,
    // including code that mutates this very dict — real CPython tests:
    // gh-97591, gh-140551 — which would re-enter `borrow_mut()` on the same
    // RefCell and panic if such a borrow were held at the time).
    if matches!(&*obj.borrow(), PyObject::Dict(_)) {
        return pydict_safe_set(obj, index.clone(), value);
    }

    let mut o = obj.borrow_mut();
    match &mut *o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("list assignment index out of range"));
                }
                items[i as usize] = value;
                return Ok(());
            }
            Err(PyError::type_error(format!("list indices must be integers or slices, not {}", idx.type_name())))
        }
        PyObject::Deque { data, .. } => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("deque index out of range"))?;
                let len = data.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("deque assignment index out of range"));
                }
                data[i as usize] = value;
                return Ok(());
            }
            Err(PyError::type_error(format!("deque indices must be integers or slices, not {}", idx.type_name())))
        }
        // PyObject::Dict is handled above, before this borrow is taken.
        _ => Err(PyError::type_error(format!("'{}' object does not support item assignment", o.type_name()))),
    }
}

pub fn py_delitem(obj: &PyObjectRef, index: &PyObjectRef) -> PyResult<()> {
    // Check for __delitem__ on custom classes
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__delitem__"),
            _ => None,
        }
    };
    if let Some(f) = f {
        call_bound_method(f, obj.clone(), vec![index.clone()])?;
        return Ok(());
    }
    if let Some(native) = native_backing_of(obj) {
        return py_delitem(&native, index);
    }
    // Dict deletion: compute the key's hash BEFORE taking `obj`'s own
    // mutable borrow — see `PyDict::set_with_hash`'s doc comment for why
    // (a custom `__hash__` can run arbitrary Python that mutates this same
    // dict, and computing the hash while already mutably borrowed would
    // panic on re-entry).
    if matches!(&*obj.borrow(), PyObject::Dict(_)) {
        let h = index.hash()?;
        let mut o = obj.borrow_mut();
        if let PyObject::Dict(d) = &mut *o {
            d.remove_with_hash(index, h)?;
            return Ok(());
        }
    }
    let mut o = obj.borrow_mut();
    match &mut *o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("list index out of range"));
                }
                items.remove(i as usize);
                Ok(())
            } else {
                Err(PyError::type_error(format!("list indices must be integers or slices, not {}", idx.type_name())))
            }
        }
        PyObject::Deque { data, .. } => {
            let idx = index.borrow();
            if let Some(i) = sequence_index_int(&idx) {
                let i = i.to_isize().ok_or_else(|| PyError::index_error("deque index out of range"))?;
                let len = data.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("deque index out of range"));
                }
                data.remove(i as usize);
                Ok(())
            } else {
                Err(PyError::type_error(format!("deque indices must be integers or slices, not {}", idx.type_name())))
            }
        }
        // PyObject::Dict is handled above, before this borrow is taken.
        _ => Err(PyError::type_error(format!("'{}' object does not support item deletion", o.type_name()))),
    }
}

