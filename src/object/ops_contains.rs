// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the `in`/`not in`
// operator implementation (`contains_op`/`py_contains`).
use super::*;

fn is_index_error(e: &PyError) -> bool {
    matches!(e, PyError::IndexError(_))
}

/// Get the class name for an object, handling Instance types correctly.
fn class_name_for_obj(obj: &PyObjectRef) -> String {
    match &*obj.borrow() {
        PyObject::Instance { typ, .. } => get_type_name_for_instance(typ),
        _ => obj.get_type_name(),
    }
}

pub fn contains_op(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
    // Check for __contains__ on instances
    let f = {
        let container = a.borrow();
        match &*container {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__contains__"),
            // `x in SomeClass` — a class object's metaclass may define
            // `__contains__` directly (e.g. enum's EnumType); otherwise
            // fall through below to the generic iterate-and-compare path,
            // which already works for any class whose metatype provides
            // `__iter__` (see builtin_iter's PyObject::Type arm).
            PyObject::Type { .. } => {
                metatype_of(a).and_then(|mt| lookup_dunder_via_mro(&mt, "__contains__"))
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        // If __contains__ is explicitly set to None, it blocks the containment check
        if matches!(&*f.borrow(), PyObject::None) {
            return Err(PyError::type_error(format!(
                "argument of type '{}' is not a container or iterable",
                class_name_for_obj(a)
            )));
        }
        let result = call_bound_method(f, a.clone(), vec![b.clone()])?;
        return Ok(result.truthy());
    }
    if let Some(native) = native_backing_of(a) {
        return contains_op(&native, b);
    }
    if matches!(&*a.borrow(), PyObject::Type { .. }) {
        let it = builtin_iter(&[a.clone()])?;
        loop {
            match builtin_next(&[it.clone()]) {
                Ok(item) => {
                    if item.equals(b)? {
                        return Ok(true);
                    }
                }
                Err(e) if is_stop_iteration_error(&e) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
    }
    // Instance fallback: if __contains__ is not defined, try __getitem__
    if matches!(&*a.borrow(), PyObject::Instance { .. }) {
        let getitem_f = {
            let container = a.borrow();
            if let PyObject::Instance { typ, .. } = &*container {
                lookup_dunder_via_mro(typ, "__getitem__")
            } else {
                None
            }
        };
        if let Some(f) = getitem_f {
            let mut i = 0i64;
            loop {
                let args_vec = vec![a.clone(), py_int(i)];
                let result = call_bound_method(f.clone(), a.clone(), vec![py_int(i)]);
                match result {
                    Ok(item) => {
                        if item.equals(b)? {
                            return Ok(true);
                        }
                    }
                    Err(e) if is_index_error(&e) => return Ok(false),
                    Err(e) => return Err(e),
                }
                i += 1;
            }
        }
        // Helper: get the correct class name for an Instance object
        return Err(PyError::type_error(format!(
            "argument of type '{}' is not a container or iterable",
            class_name_for_obj(a)
        )));
    }
    let container = a.borrow();
    match &*container {
        PyObject::Str(s) => {
            // Real `'x' in some_str` requires `x` to ALREADY be a `str` —
            // was coercing ANY `b` via `.str()` (e.g. `None` -> `"None"`)
            // and checking substring containment against that, silently
            // returning `False` for a nonsensical comparison instead of
            // raising. Confirmed via CPython's own `test_contains.py`
            // (`None in 'abc'` must raise `TypeError`).
            if !matches!(&*b.borrow(), PyObject::Str(_)) {
                return Err(PyError::type_error(format!(
                    "'in <string>' requires string as left operand, not {}",
                    b.get_type_name()
                )));
            }
            let item_str = b.str();
            Ok(s.contains(&item_str))
        }
        PyObject::List(items) => {
            // Clone the item list out of the borrow BEFORE iterating —
            // `item.equals(b)` can run arbitrary Python (a custom
            // `__eq__`), and holding `container`'s borrow live across that
            // call means a pathological `__eq__` that mutates THIS SAME
            // list (e.g. a test deliberately exercising a
            // clears-itself-during-comparison edge case) hits `list.clear()`
            // (or any other mutator) while this borrow is still held,
            // panicking with "RefCell already borrowed" instead of either
            // completing or raising a normal Python-level error. Confirmed
            // via a real trigger in CPython's own `test_deque.py`.
            let items = items.clone();
            drop(container);
            for item in &items {
                // Identity check first (for NaN: nan is nan but nan != nan)
                if item.is(b) || item.equals(b)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        PyObject::Tuple(items) => {
            let items = items.clone();
            drop(container);
            for item in &items {
                if item.is(b) || item.equals(b)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        PyObject::Deque { data, .. } => {
            // Same clone-out-of-the-borrow treatment as `List` above,
            // PLUS CPython's length-change mutation detection: a
            // pathological element `__eq__` that mutates this SAME deque
            // mid-scan (real triggers: `test_deque.py`'s `test_contains`
            // — `MutateCmp` clears the deque and returns False; a class
            // whose `__eq__` clears the deque and returns `NotImplemented`)
            // must raise `RuntimeError: deque mutated during iteration`,
            // not just give a wrong answer.
            let items: Vec<PyObjectRef> = data.iter().cloned().collect();
            let start_len = data.len();
            drop(container);
            for item in &items {
                // Rich `==` (not raw `equals`) so a custom Instance
                // element's reflected `__eq__` is consulted (`ALWAYS_EQ
                // in deque([1])` — `1 == ALWAYS_EQ` must call
                // `ALWAYS_EQ.__eq__(1)` and match).
                if item.is(b) || py_compare(item, b, 2)?.truthy() {
                    return Ok(true);
                }
                let current_len = a.borrow();
                if let PyObject::Deque { data, .. } = &*current_len {
                    if data.len() != start_len {
                        return Err(PyError::runtime_error("deque mutated during iteration"));
                    }
                }
            }
            Ok(false)
        }
        // Clone the dict/set out of the borrow BEFORE probing — same
        // reason as the `List`/`Tuple` arms just above: `.contains()`
        // internally calls `.equals()` against each colliding key/member,
        // which for an `Instance` key/member invokes the user's `__eq__`
        // and can reentrantly touch this SAME dict/set (e.g. `d.clear()`
        // from within its own `__eq__` — the identical hazard already
        // fixed for `d[k]=v`/`setdefault()`/`set.add()`, just not yet for
        // the `in` operator/`contains_op`). Holding `container`'s borrow
        // across that used to panic with "RefCell already borrowed" the
        // instant the reentrant call made its own borrow — confirmed via a
        // real, adversarial CPython test reached once `test_dict.py` got
        // far enough to hit it (previously masked behind an earlier,
        // now-fixed hang/panic in the same file).
        PyObject::Dict(d) => {
            let d = d.clone();
            drop(container);
            d.contains(b)
        }
        PyObject::Set(items) => {
            let items = items.clone();
            drop(container);
            items.contains(b)
        }
        PyObject::FrozenSet(items) => {
            let items = items.clone();
            drop(container);
            items.contains(b)
        }
        // `x in some_bytes` — real CPython accepts either a single int
        // byte value (`0x25 in b"a%b"`) or a bytes-like subsequence
        // (`b"%20" in b"a%20b"`). Was missing entirely (fell through to
        // the `_` catch-all's "is not iterable" TypeError) — a real
        // trigger: `urllib.parse.unquote_to_bytes`'s `b"%" not in string`
        // early-exit check.
        PyObject::Bytes(data) | PyObject::ByteArray(data) => {
            let bval = b.borrow();
            match &*bval {
                PyObject::Int(n) => {
                    let byte = n.to_i64().unwrap_or(-1);
                    Ok((0..=255).contains(&byte) && data.contains(&(byte as u8)))
                }
                PyObject::Bytes(needle) | PyObject::ByteArray(needle) => {
                    if needle.is_empty() {
                        Ok(true)
                    } else {
                        Ok(data.windows(needle.len()).any(|w| w == needle.as_slice()))
                    }
                }
                _ => Err(PyError::type_error(
                    "argument should be integer or bytes-like object",
                )),
            }
        }
        PyObject::Range { start, stop, step } => {
            let item = b.borrow();
            if let PyObject::Int(n) = &*item {
                let n = n.to_i64().unwrap_or(0);
                if *step > 0 {
                    Ok(n >= *start && n < *stop && (n - *start) % *step == 0)
                } else {
                    Ok(n <= *start && n > *stop && (n - *start) % *step == 0)
                }
            } else {
                Ok(false)
            }
        }
        _ => Err(PyError::type_error(format!(
            "argument of type '{}' is not iterable",
            container.type_name()
        ))),
    }
}

pub fn py_contains(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    contains_op(a, b).map(py_bool)
}
