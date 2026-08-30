// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds subscript
// access (`__getitem__`/`__setitem__`/`__delitem__` dispatch across
// list/dict/str/bytes/tuple/user-defined classes).
use super::*;

mod to_index;
pub use to_index::to_index;
mod seq_index;
pub(crate) use seq_index::sequence_index_int;
mod try_index;
pub(crate) use try_index::try_to_index;
mod slice;
pub(crate) use slice::{extract_slice_fields, normalize_slice_bounds, slice_indices_values};

pub fn py_getitem(obj: &PyObjectRef, index: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let PyObject::WeakProxy { target, .. } = &*obj.borrow() {
        if let Some(rc) = target.upgrade() {
            return py_getitem(&PyObjectRef::Mut(rc), index);
        } else {
            return Err(PyError::reference_error("weakly-referenced object no longer exists"));
        }
    }
    // Check for __getitem__ on custom classes and __class_getitem__ on types (PEP 560)
    let f = {
        let o = obj.borrow();
        match &*o {
            PyObject::Type {
                dict: type_dict,
                mro,
                ..
            } => {
                // Real Python checks the metaclass's `__getitem__` first
                // (subscripting a class object is, at bottom, calling
                // `type(cls).__getitem__(cls, key)`) — e.g. enum's
                // `EnumType.__getitem__` for `Color['RED']` name lookup.
                // `__class_getitem__` (PEP 560, `list[int]`-style generic
                // aliasing) is the fallback for classes with no such
                // metaclass method.
                let metatype_getitem =
                    metatype_of(obj).and_then(|mt| lookup_dunder_via_mro(&mt, "__getitem__"));
                metatype_getitem.or_else(|| {
                    type_dict.get_str("__class_getitem__").cloned().or_else(|| {
                        for base in mro.iter().skip(1) {
                            if let PyObject::Type {
                                dict: base_dict, ..
                            } = &*base.borrow()
                            {
                                if let Some(val) = base_dict.get_str("__class_getitem__") {
                                    return Some(val.clone());
                                }
                            }
                        }
                        // Fallback: if no __class_getitem__ found, return
                        // a GenericAlias for Generic subclasses (PEP 560).
                        // This allows `SimpleMapping[XK, XV]` to work even
                        // when the class doesn't explicitly define __class_getitem__.
                        Some(PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "__class_getitem__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error(
                                        "__class_getitem__ requires 2 arguments",
                                    ));
                                }
                                Ok(PyObjectRef::new(PyObject::BuiltinFunction {
                                    name: "_GenericAlias".to_string(),
                                    func: |_args| Ok(py_none()),
                                }))
                            },
                        }))
                    })
                })
            }
            PyObject::Instance { typ, .. } => {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type {
                        dict: type_dict,
                        mro,
                        ..
                    } => {
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
                            let skip_native_dunder_hatch =
                                type_dict.contains_key_str(NATIVE_BASE_MARKER);
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type {
                                    dict: base_dict, ..
                                } = &*base.borrow()
                                {
                                    if skip_native_dunder_hatch
                                        && base_dict.contains_key_str(NATIVE_VALUE_CTOR_KEY)
                                    {
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
                    None => Err(PyError::key_error_obj(index)),
                }
            }
            Err(PyError::Exception(t, o)) if t == "KeyError" => {
                let missing_fn = match &*obj.borrow() {
                    PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__missing__"),
                    _ => None,
                };
                match missing_fn {
                    Some(f) => call_bound_method(f, obj.clone(), vec![index.clone()]),
                    None => Err(PyError::Exception(t, o)),
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
        let h = crate::object::PyDict::dict_hash(index)?;
        let o = obj.borrow();
        if let PyObject::Dict(d) = &*o {
            return match d.get_with_hash(index, h)? {
                Some(val) => Ok(val),
                None => Err(PyError::key_error_obj(index)),
            };
        }
    }
    if let PyObject::Globals(g) = &*obj.borrow() {
        let key = match &*index.borrow() {
            PyObject::Str(s) => interner::intern(s.as_str()),
            _ => return Err(PyError::key_error(index.str())),
        };
        return match g.borrow().get(&key).cloned() {
            Some(val) => Ok(val),
            None => Err(PyError::key_error(index.str())),
        };
    }
    let o = obj.borrow();
    match &*o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("list index out of range"))?;
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
                    let (start_n, stop_n) =
                        normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    } else {
                        while i > stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    }
                    Ok(py_list(result))
                }
                _ => Err(PyError::type_error(format!(
                    "list indices must be integers or slices, not {}",
                    idx.type_name()
                ))),
            }
        }
        PyObject::Deque { data, .. } => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("deque index out of range"))?;
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
                    let (start_n, stop_n) =
                        normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push_back(data[i as usize].clone());
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    } else {
                        while i > stop_n {
                            result.push_back(data[i as usize].clone());
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    }
                    // Real CPython's `deque.__getitem__(slice)` returns a
                    // plain (maxlen=None) deque.
                    Ok(py_deque(result, None))
                }
                _ => Err(PyError::type_error(format!(
                    "deque indices must be integers or slices, not {}",
                    idx.type_name()
                ))),
            }
        }
        PyObject::Tuple(items) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("tuple index out of range"))?;
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
                    let (start_n, stop_n) =
                        normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    } else {
                        while i > stop_n {
                            result.push(items[i as usize].clone());
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    }
                    Ok(py_tuple(result))
                }
                _ => Err(PyError::type_error(format!(
                    "tuple indices must be integers or slices, not {}",
                    idx.type_name()
                ))),
            }
        }
        PyObject::Str(s) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let chars: Vec<char> = s.chars().collect();
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("string index out of range"))?;
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
                    let (start_n, stop_n) =
                        normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut result = String::new();
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(chars[i as usize]);
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    } else {
                        while i > stop_n {
                            result.push(chars[i as usize]);
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    }
                    Ok(py_str(&result))
                }
                _ => Err(PyError::type_error(format!(
                    "string indices must be integers, not '{}'",
                    idx.type_name()
                ))),
            }
        }
        // PyObject::Dict is handled above, before this borrow is taken.
        PyObject::Bytes(b) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("bytes index out of range"))?;
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
                    let (start_n, stop_n) =
                        normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut result = Vec::new();
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    } else {
                        while i > stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    }
                    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
                }
                _ => Err(PyError::type_error(format!(
                    "bytes indices must be integers or slices, not {}",
                    idx.type_name()
                ))),
            }
        }
        PyObject::ByteArray(b) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("bytearray index out of range"))?;
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
                    let (start_n, stop_n) =
                        normalize_slice_bounds(start_val, stop_val, step_val, len);
                    let mut result = Vec::new();
                    let mut i = start_n;
                    if step_val > 0 {
                        while i < stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    } else {
                        while i > stop_n {
                            result.push(b[i as usize]);
                            match i.checked_add(step_val) {
                                Some(next) => i = next,
                                None => break,
                            };
                        }
                    }
                    Ok(PyObjectRef::new(PyObject::ByteArray(result)))
                }
                _ => Err(PyError::type_error(format!(
                    "bytearray indices must be integers or slices, not {}",
                    idx.type_name()
                ))),
            }
        }
        PyObject::Array(arr) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("array index out of range"))?;
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
                Err(PyError::type_error(format!(
                    "array indices must be integers, not {}",
                    idx.type_name()
                )))
            }
        }
        PyObject::Range { start, stop, step } => {
            let idx = index.borrow();
            let zero = BigInt::from(0);
            let range_len = crate::object::ops_contains::range_len_values;
            if let Some(i) = try_to_index(index) {
                let len = range_len(start, stop, step);
                let pos = if i < zero { &len + &i } else { i };
                if pos < zero || pos >= len {
                    return Err(PyError::index_error("range object index out of range"));
                }
                return Ok(py_int(start + step * pos));
            }
            match &*idx {
                PyObject::Slice {
                    start: s,
                    stop: e,
                    step: p,
                } => {
                    let len = range_len(start, stop, step);
                    let (norm_start, norm_stop, norm_step) = slice_indices_values(s, e, p, &len)?;
                    let new_start = start + norm_start * step;
                    let new_step = step * norm_step;
                    let new_stop = start + norm_stop * step;
                    Ok(PyObjectRef::imm(PyObject::Range {
                        start: new_start,
                        stop: new_stop,
                        step: new_step,
                    }))
                }
                _ => Err(PyError::type_error(format!(
                    "range indices must be integers or slices, not {}",
                    idx.type_name()
                ))),
            }
        }
        // A plain Instance with no `__getitem__` anywhere in its MRO and no
        // native backing. Real CPython raises TypeError here ("'X' object is
        // not subscriptable") — it does NOT fall back to treating the
        // instance's `__dict__` as a mapping. (That wrong fallback surfaced
        // via test_bisect's `LenOnly`/`GetOnly` error-handling classes:
        // `bisect` indexing `a[mid]` on an object with no `__getitem__` got
        // a KeyError instead of the required TypeError.)
        PyObject::Instance { typ, .. } => {
            let type_name = get_type_name_for_instance(typ);
            Err(PyError::type_error(format!(
                "'{}' object is not subscriptable",
                type_name
            )))
        }
        _ => Err(PyError::type_error(format!(
            "'{}' object is not subscriptable",
            o.type_name()
        ))),
    }
}

pub fn py_setitem(obj: &PyObjectRef, index: &PyObjectRef, value: PyObjectRef) -> PyResult<()> {
    if let PyObject::WeakProxy { target, .. } = &*obj.borrow() {
        if let Some(rc) = target.upgrade() {
            return py_setitem(&PyObjectRef::Mut(rc), index, value);
        } else {
            return Err(PyError::reference_error("weakly-referenced object no longer exists"));
        }
    }
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
    let is_list_slice = matches!(&*obj.borrow(), PyObject::List(_))
        && matches!(&*index.borrow(), PyObject::Slice { .. });
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
                    match i.checked_add(step_val) {
                        Some(next) => i = next,
                        None => break,
                    }
                }
            } else {
                while i > stop_n {
                    indices.push(i as usize);
                    match i.checked_add(step_val) {
                        Some(next) => i = next,
                        None => break,
                    }
                }
            }
            if indices.len() != new_items.len() {
                return Err(PyError::value_error(format!(
                    "attempt to assign sequence of size {} to extended slice of size {}",
                    new_items.len(),
                    indices.len()
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
    if let PyObject::Globals(g) = &*obj.borrow() {
        let key = match &*index.borrow() {
            PyObject::Str(s) => interner::intern(s.as_str()),
            _ => {
                return Err(PyError::type_error(
                    "globals keys must be strings".to_string(),
                ))
            }
        };
        g.borrow_mut().insert(key, value);
        return Ok(());
    }

    // Non-mutable (inline/Imm) objects cannot be assigned into — raise
    // TypeError before borrow_mut would panic on them.
    if !matches!(obj, PyObjectRef::Mut(_)) {
        return Err(PyError::type_error(format!(
            "'{}' object does not support item assignment",
            obj.borrow().type_name()
        )));
    }

    let mut o = obj.borrow_mut();
    match &mut *o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("list index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("list assignment index out of range"));
                }
                items[i as usize] = value;
                return Ok(());
            }
            Err(PyError::type_error(format!(
                "list indices must be integers or slices, not {}",
                idx.type_name()
            )))
        }
        PyObject::Deque { data, .. } => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("deque index out of range"))?;
                let len = data.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("deque assignment index out of range"));
                }
                data[i as usize] = value;
                return Ok(());
            }
            Err(PyError::type_error(format!(
                "deque indices must be integers or slices, not {}",
                idx.type_name()
            )))
        }
        PyObject::ByteArray(b) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("bytearray index out of range"))?;
                let len = b.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("bytearray index out of range"));
                }
                let val = value
                    .as_i64()
                    .ok_or_else(|| PyError::type_error("an integer is required"))?;
                if !(0..=255).contains(&val) {
                    return Err(PyError::value_error("byte must be in range(0, 256)"));
                }
                b[i as usize] = val as u8;
                return Ok(());
            }
            match &*idx {
                PyObject::Slice { start, stop, step } => {
                    let len = b.len();
                    let (start_val, stop_val, step_val) =
                        extract_slice_fields(start, stop, step)?;
                    let (start_n, stop_n) =
                        normalize_slice_bounds(start_val, stop_val, step_val, len);
                    // Collect replacement bytes from value (bytes, bytearray, or iterable of ints)
                    let new_bytes: Vec<u8> = {
                        let vt = value.borrow();
                        match &*vt {
                            PyObject::Bytes(v) => v.clone(),
                            PyObject::ByteArray(v) => v.clone(),
                            PyObject::List(items) => {
                                let mut vec = Vec::new();
                                for item in items {
                                    let v = item
                                        .as_i64()
                                        .ok_or_else(|| {
                                            PyError::type_error("an integer is required")
                                        })?;
                                    if !(0..=255).contains(&v) {
                                        return Err(PyError::value_error(
                                            "byte must be in range(0, 256)",
                                        ));
                                    }
                                    vec.push(v as u8);
                                }
                                vec
                            }
                            PyObject::Str(s) => s.as_bytes().to_vec(),
                            _ => {
                                // Generic iterable fallback
                                drop(vt);
                                let it = crate::object::builtin_iter(&[value.clone()])?;
                                let mut vec = Vec::new();
                                loop {
                                    match crate::object::builtin_next(&[it.clone()]) {
                                        Ok(item) => {
                                            let v = item.as_i64().ok_or_else(|| {
                                                PyError::type_error("an integer is required")
                                            })?;
                                            if !(0..=255).contains(&v) {
                                                return Err(PyError::value_error(
                                                    "byte must be in range(0, 256)",
                                                ));
                                            }
                                            vec.push(v as u8);
                                        }
                                        Err(PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                                vec
                            }
                        }
                    };
                    if step_val == 1 {
                        let stop_n = stop_n.max(start_n);
                        b.splice(start_n as usize..stop_n as usize, new_bytes);
                        return Ok(());
                    } else {
                        let mut indices = Vec::new();
                        let mut i = start_n;
                        if step_val > 0 {
                            while i < stop_n {
                                indices.push(i as usize);
                                match i.checked_add(step_val) {
                                    Some(next) => i = next,
                                    None => break,
                                }
                            }
                        } else {
                            while i > stop_n {
                                indices.push(i as usize);
                                match i.checked_add(step_val) {
                                    Some(next) => i = next,
                                    None => break,
                                }
                            }
                        }
                        if indices.len() != new_bytes.len() {
                            return Err(PyError::value_error(format!(
                                "attempt to assign sequence of size {} to extended slice of size {}",
                                new_bytes.len(),
                                indices.len()
                            )));
                        }
                        for (idx, val) in indices.into_iter().zip(new_bytes) {
                            b[idx] = val;
                        }
                        return Ok(());
                    }
                }
                _ => {
                    return Err(PyError::type_error(format!(
                        "bytearray indices must be integers or slices, not {}",
                        idx.type_name()
                    )))
                }
            }
        }
        // PyObject::Dict is handled above, before this borrow is taken.
        _ => Err(PyError::type_error(format!(
            "'{}' object does not support item assignment",
            o.type_name()
        ))),
    }
}

pub fn py_delitem(obj: &PyObjectRef, index: &PyObjectRef) -> PyResult<()> {
    if let PyObject::WeakProxy { target, .. } = &*obj.borrow() {
        if let Some(rc) = target.upgrade() {
            return py_delitem(&PyObjectRef::Mut(rc), index);
        } else {
            return Err(PyError::reference_error("weakly-referenced object no longer exists"));
        }
    }
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
    if matches!(&*obj.borrow(), PyObject::Dict(_)) {
        crate::object::pydict_safe_remove(obj, index)?;
        return Ok(());
    }
    if let PyObject::Globals(g) = &*obj.borrow() {
        let key = match &*index.borrow() {
            PyObject::Str(s) => interner::intern(s.as_str()),
            _ => return Err(PyError::key_error(index.str())),
        };
        g.borrow_mut().remove(&key);
        return Ok(());
    }
    // Non-mutable (inline/Imm) objects cannot be deleted from — raise
    // TypeError before borrow_mut would panic on them.
    if !matches!(obj, PyObjectRef::Mut(_)) {
        return Err(PyError::type_error(format!(
            "'{}' object does not support item deletion",
            obj.borrow().type_name()
        )));
    }
    let mut o = obj.borrow_mut();
    match &mut *o {
        PyObject::List(items) => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("list index out of range"))?;
                let len = items.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("list index out of range"));
                }
                items.remove(i as usize);
                Ok(())
            } else {
                match &*idx {
                    // `del a[slice]` (test_list::test_delslice etc.) —
                    // previously rejected slices with the "not slice" error.
                    PyObject::Slice { start, stop, step } => {
                        let len = items.len();
                        let (start_val, stop_val, step_val) =
                            extract_slice_fields(start, stop, step)?;
                        let (start_n, stop_n) =
                            normalize_slice_bounds(start_val, stop_val, step_val, len);
                        let mut to_remove: Vec<usize> = Vec::new();
                        if step_val > 0 {
                            let mut i = start_n;
                            while i < stop_n {
                                to_remove.push(i as usize);
                                i += step_val;
                            }
                        } else {
                            let mut i = start_n;
                            while i > stop_n {
                                to_remove.push(i as usize);
                                match i.checked_add(step_val) {
                                    Some(next) => i = next,
                                    None => break,
                                };
                            }
                        }
                        for idx2 in to_remove.into_iter().rev() {
                            items.remove(idx2);
                        }
                        Ok(())
                    }
                    _ => Err(PyError::type_error(format!(
                        "list indices must be integers or slices, not {}",
                        idx.type_name()
                    ))),
                }
            }
        }
        PyObject::Deque { data, .. } => {
            let idx = index.borrow();
            if let Some(i) = try_to_index(index) {
                let i = i
                    .to_isize()
                    .ok_or_else(|| PyError::index_error("deque index out of range"))?;
                let len = data.len() as isize;
                let i = if i < 0 { len + i } else { i };
                if i < 0 || i >= len {
                    return Err(PyError::index_error("deque index out of range"));
                }
                data.remove(i as usize);
                Ok(())
            } else {
                Err(PyError::type_error(format!(
                    "deque indices must be integers or slices, not {}",
                    idx.type_name()
                )))
            }
        }
        // PyObject::Dict is handled above, before this borrow is taken.
        _ => Err(PyError::type_error(format!(
            "'{}' object does not support item deletion",
            o.type_name()
        ))),
    }
}
