// Split out of the former monolithic object/builtins.rs — this file holds
// iteration and sequence-length builtins (`len`, `range`, `iter`, `next`,
// `reversed`) and their helpers.
use super::*;

pub fn builtin_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("len() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Str(s) => Ok(py_int(s.chars().count())),
        PyObject::List(v) => Ok(py_int(v.len())),
        PyObject::Deque { data, .. } => Ok(py_int(data.len())),
        PyObject::Tuple(v) => Ok(py_int(v.len())),
        PyObject::Dict(d) => Ok(py_int(d.len())),
        PyObject::Globals(g) => Ok(py_int(g.borrow().len())),
        PyObject::Set(s) => Ok(py_int(s.len())),
        PyObject::FrozenSet(s) => Ok(py_int(s.len())),
        PyObject::Range { start, stop, step } => {
            let len = crate::object::ops_contains::range_len_values(start, stop, step);
            if len.to_i64().is_none() {
                return Err(PyError::overflow_error(
                    "Python int too large to convert to C ssize_t",
                ));
            }
            Ok(py_int(len))
        }
        PyObject::Bytes(b) => Ok(py_int(b.len())),
        PyObject::ByteArray(b) => Ok(py_int(b.len())),
        PyObject::Array(arr) => Ok(py_int(arr.data.len())),
        PyObject::MemoryView { .. } => {
            drop(obj);
            Ok(py_int(mv_len(&args[0])? as i64))
        }
        // Real Python's `list_iterator`/`range_iterator`/etc. all support
        // `len()` — it reports the number of REMAINING elements, not the
        // original sequence's length (used by `operator.length_hint`, and
        // directly by real code — real trigger: CPython's own
        // `test_iterlen.py`, whose whole purpose is exercising this exact
        // protocol across iterator types).
        PyObject::ListIter { list, index } => Ok(py_int(list.len().saturating_sub(*index))),
        PyObject::DequeIter {
            deque,
            index,
            start_len,
        } => {
            let remaining = {
                let dq = deque.borrow();
                if let PyObject::Deque { data, .. } = &*dq {
                    if data.len() != *start_len {
                        None
                    } else {
                        Some(data.len().saturating_sub(*index))
                    }
                } else {
                    None
                }
            };
            match remaining {
                Some(n) => Ok(py_int(n)),
                None => Ok(py_int(0)),
            }
        }
        PyObject::RangeIter {
            current,
            stop,
            step,
        } => {
            // Use BigInt throughout: `current`/`stop` can be near the i64
            // boundary (a range_iterator unpickled with adversarial bounds,
            // or a real near-i64::MAX/MIN range), and this arithmetic used
            // to overflow-panic in plain i64 (`stop - current + step - 1`)
            // instead of just returning the (possibly huge, but always
            // representable) remaining count.
            let current = current.clone();
            let stop = stop.clone();
            let step = step.clone();
            let zero = BigInt::from(0);
            let remaining = if step > zero && current < stop {
                (&stop - &current + &step - BigInt::from(1)) / &step
            } else if step < zero && current > stop {
                (&current - &stop - &step - BigInt::from(1)) / (-&step)
            } else {
                zero.clone()
            };
            Ok(py_int(remaining.max(zero)))
        }
        PyObject::Instance { typ, dict } => {
            let f = lookup_dunder_via_mro(typ, "__len__");
            let native = dict.get(NATIVE_BACKING_KEY).cloned();
            let type_name = obj.type_name();
            // Drop the borrow on args[0] before calling into `__len__` —
            // holding it across the call panics with "RefCell already
            // borrowed" the moment `__len__` mutates `self` (real trigger:
            // CPython's own `test_enumerate.py`'s `SeqWithWeirdLen.__len__`,
            // which does `self.called = True`).
            drop(obj);
            if let Some(f) = f {
                let result = call_bound_method(f, args[0].clone(), vec![])?;
                let n = result.borrow();
                if let PyObject::Int(i) = &*n {
                    // Real CPython rejects a negative `__len__()` result
                    // with `ValueError: __len__() should return >= 0` —
                    // this was missing entirely, silently accepting -1 as
                    // a length. Confirmed via CPython's own
                    // `test_bool.test_sane_len`, which asserts `bool()`'s
                    // and `len()`'s error messages for the same bad
                    // `__len__` values are identical — `bool()` delegates
                    // to this same function specifically so that holds.
                    if i.sign() == Sign::Minus {
                        return Err(PyError::value_error("__len__() should return >= 0"));
                    }
                    return Ok(py_int(i.clone()));
                }
                return Err(PyError::type_error("__len__() should return an int"));
            }
            if let Some(native) = native {
                return builtin_len(&[native]);
            }
            Err(PyError::type_error(format!(
                "object of type '{}' has no len()",
                type_name
            )))
        }
        // A class object itself, via its metaclass's `__len__` (e.g.
        // `len(SomeEnum)` — see the matching GET_ITER/builtin_iter handling
        // for why this needs metatype_of rather than ordinary lookup).
        PyObject::Type { .. } => {
            let f = metatype_of(&args[0]).and_then(|mt| lookup_dunder_via_mro(&mt, "__len__"));
            let type_name = obj.type_name();
            drop(obj);
            if let Some(f) = f {
                let result = call_bound_method(f, args[0].clone(), vec![])?;
                let n = result.borrow();
                if let PyObject::Int(i) = &*n {
                    if i.sign() == Sign::Minus {
                        return Err(PyError::value_error("__len__() should return >= 0"));
                    }
                    return Ok(py_int(i.clone()));
                }
                return Err(PyError::type_error("__len__() should return an int"));
            }
            Err(PyError::type_error(format!(
                "object of type '{}' has no len()",
                type_name
            )))
        }
        _ => Err(PyError::type_error(format!(
            "object of type '{}' has no len()",
            obj.type_name()
        ))),
    }
}


/// Cheap, best-effort size hint for materializing an arbitrary iterable
/// into a `Vec` (used by `list()`/`tuple()`). Real CPython pre-sizes via
/// `PyObject_LengthHint` before iterating, so a source with an O(1) `len()`
/// (e.g. `range(huge)`) fails fast with a single allocation attempt instead
/// of growing the backing buffer one doubling at a time — which, for
/// something like `list(range(sys.maxsize // 2))`, can consume the
/// system's entire RAM over many reallocations before ever failing (each
/// individual `push()` succeeds right up until physical memory runs out).
/// Returns `None` (not an error) when the object has no usable `__len__` —
/// callers should just skip pre-reservation and fall back to incremental
/// growth, which is fine for ordinary bounded iterables/generators.
pub(crate) fn iterable_length_hint(obj: &PyObjectRef) -> Option<usize> {
    let len_obj = builtin_len(std::slice::from_ref(obj)).ok()?;
    let borrowed = len_obj.borrow();
    match &*borrowed {
        PyObject::Int(n) => n.to_usize(),
        _ => None,
    }
}


// real `range()` accepts anything implementing `__index__`, not just a
// literal `int` (`crate::object::subscript::to_index` already implements
// that same "native int, or call `__index__` via mro" protocol for
// slicing) — found via CPython's own `test_range.py`, which constructs
// `range()` bounds from custom `__index__`-only objects.
pub(crate) fn range_index_arg(obj: &PyObjectRef) -> PyResult<num_bigint::BigInt> {
    to_index(obj)
}


pub fn builtin_range(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let one = num_bigint::BigInt::from(1);
    let zero = num_bigint::BigInt::from(0);
    match args.len() {
        1 => {
            let stop = range_index_arg(&args[0])?;
            Ok(PyObjectRef::imm(PyObject::Range {
                start: zero,
                stop,
                step: one,
            }))
        }
        2 => {
            let a = range_index_arg(&args[0])?;
            let b = range_index_arg(&args[1])?;
            Ok(PyObjectRef::imm(PyObject::Range {
                start: a,
                stop: b,
                step: one,
            }))
        }
        3 => {
            let a = range_index_arg(&args[0])?;
            let b = range_index_arg(&args[1])?;
            let s = range_index_arg(&args[2])?;
            if s == zero {
                return Err(PyError::value_error("range() arg 3 must not be zero"));
            }
            Ok(PyObjectRef::imm(PyObject::Range {
                start: a,
                stop: b,
                step: s,
            }))
        }
        _ => {
            let msg = if args.is_empty() {
                format!("range expected at least 1 argument, got 0")
            } else {
                format!("range expected at most 3 arguments, got {}", args.len())
            };
            Err(PyError::type_error(msg))
        }
    }
}


pub fn builtin_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() == 1 {
        if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
            if let Some(rc) = target.upgrade() {
                return builtin_iter(&[PyObjectRef::Imm(rc)]);
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }
    }
    // Two-argument form: `iter(callable, sentinel)` — calls `callable()`
    // repeatedly, yielding each result until one equals `sentinel`. Real,
    // commonly-used Python (`iter(file.readline, '')` is the classic
    // idiom), not just a test-only construct.
    if args.len() == 2 {
        if !builtin_callable(&[args[0].clone()])?.truthy() {
            return Err(PyError::type_error(format!(
                "iter(v, w): v must be callable"
            )));
        }
        return Ok(PyObjectRef::new(PyObject::CallSentinelIter {
            func: args[0].clone(),
            sentinel: args[1].clone(),
            exhausted: false,
        }));
    }
    if args.len() != 1 {
        return Err(PyError::type_error("iter() takes exactly one argument"));
    }
    // Check for __iter__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__iter__"),
            PyObject::Generator { .. } => {
                // Generators are their own iterator (return self)
                return Ok(args[0].clone());
            }
            // A class object itself, iterable via its metaclass's
            // `__iter__` (e.g. `iter(SomeEnum)` / `list(SomeEnum)`) — see
            // the matching GET_ITER opcode handling in vm.rs for why this
            // needs metatype_of rather than ordinary attribute lookup.
            PyObject::Type { .. } => {
                metatype_of(&args[0]).and_then(|mt| lookup_dunder_via_mro(&mt, "__iter__"))
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![]);
    }
    if let Some(native) = native_backing_of(&args[0]) {
        return builtin_iter(&[native]);
    }
    // Real Python's "old-style sequence iteration" fallback: an object with
    // `__getitem__` but no `__iter__` is still iterable — `for x in obj:`
    // calls `obj[0]`, `obj[1]`, ... until `IndexError`. Checked AFTER the
    // `__iter__` lookup above (which already returned if present) and
    // BEFORE the native-type match below (native types needing this exist
    // as their own dedicated arms already).
    if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
        if lookup_dunder_via_mro(typ, "__getitem__").is_some() {
            return Ok(PyObjectRef::new(PyObject::GetItemIter {
                obj: args[0].clone(),
                index: 0,
            }));
        }
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Tuple(v) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: v.clone(),
            index: 0,
        })),
        PyObject::Str(s) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: s.chars().map(|c| py_str(&c.to_string())).collect(),
            index: 0,
        })),
        PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: b.iter().map(|byte| py_int(*byte as i64)).collect(),
            index: 0,
        })),
        PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: b.iter().map(|byte| py_int(*byte as i64)).collect(),
            index: 0,
        })),
        PyObject::Array(arr) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: arr
                .data
                .iter()
                .map(|v| {
                    if crate::object::array_typecode_is_float(arr.typecode) {
                        py_float(*v)
                    } else if arr.typecode == 'w' || arr.typecode == 'u' {
                        let ch = (*v as u32).try_into().ok().and_then(char::from_u32).unwrap_or('\0');
                        py_str(&ch.to_string())
                    } else {
                        py_int(*v as i64)
                    }
                })
                .collect(),
            index: 0,
        })),
        PyObject::MemoryView { .. } => {
            drop(obj);
            let len = mv_len(&args[0])?;
            let mut items = Vec::with_capacity(len);
            for i in 0..len {
                items.push(mv_getitem(&args[0], &py_int(i as i64))?);
            }
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: items,
                index: 0,
            }))
        }
        // `iter(a_set)` must return a real ITERATOR (advanceable via
        // `builtin_next`), not the bare materialized list `py_list` builds —
        // a raw `PyObject::List` isn't itself a valid iterator shape (unlike
        // `List`/`Dict` just above and below, both correctly wrapped in
        // `ListIter`). This meant `for x in frozenset(...)`/`for x in
        // some_set:` raised `TypeError: 'frozenset' object is not
        // iterable` outright — a foundational gap for two of Python's most
        // basic builtin container types. Real trigger: vendoring
        // `_strptime.py` (`for i in calendar.day_abbr` style iteration
        // deeper in its own dependency chain hits a frozenset somewhere in
        // `unicodedata`/locale data) — but the bug is general, not specific
        // to that file.
        PyObject::Set(s) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: s.to_vec(),
            index: 0,
        })),
        PyObject::FrozenSet(s) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: s.to_vec(),
            index: 0,
        })),
        PyObject::Range { start, stop, step } => Ok(PyObjectRef::new(PyObject::RangeIter {
            current: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
        })),
        PyObject::List(v) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: v.clone(),
            index: 0,
        })),
        PyObject::Deque { data, .. } => Ok(PyObjectRef::new(PyObject::DequeIter {
            deque: args[0].clone(),
            index: 0,
            start_len: data.len(),
        })),
        PyObject::Dict(d) => Ok(PyObjectRef::new(PyObject::ListIter {
            list: d.keys(),
            index: 0,
        })),
        PyObject::Globals(g) => {
            let keys: Vec<PyObjectRef> = g
                .borrow()
                .keys()
                .map(|k| py_str(interner::lookup_str(*k)))
                .collect();
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: keys,
                index: 0,
            }))
        }
        // `iter(f)`/`for line in f:` — see the matching `GET_ITER` opcode
        // handling in `vm.rs` for the full story; this is the SEPARATE
        // free-function path (`iter(f)` called explicitly, or anything
        // routing through `collect_iterable`) that needs the identical fix.
        PyObject::File { file, binary, .. } => {
            use std::io::Read;
            let mut rest = Vec::new();
            file.borrow_mut()
                .read_to_end(&mut rest)
                .map_err(|e| PyError::os_error_from_io(&e))?;
            let mut lines: Vec<PyObjectRef> = Vec::new();
            let mut current: Vec<u8> = Vec::new();
            for byte in rest {
                current.push(byte);
                if byte == b'\n' {
                    lines.push(if *binary {
                        PyObjectRef::imm(PyObject::Bytes(current.clone()))
                    } else {
                        py_str(&String::from_utf8_lossy(&current))
                    });
                    current.clear();
                }
            }
            if !current.is_empty() {
                lines.push(if *binary {
                    PyObjectRef::imm(PyObject::Bytes(current.clone()))
                } else {
                    py_str(&String::from_utf8_lossy(&current))
                });
            }
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: lines,
                index: 0,
            }))
        }
        // Already an iterator object (one of `builtin_next`'s own
        // recognized variants) — `iter(it)` on an existing iterator
        // just returns it unchanged, matching real Python.
        PyObject::ListIter { .. }
        | PyObject::RangeIter { .. }
        | PyObject::CycleIter { .. }
        | PyObject::EnumerateIter { .. }
        | PyObject::MapIterator { .. }
        | PyObject::FilterIterator { .. }
        | PyObject::ZipIterator { .. }
        | PyObject::FutureAwaitIterator { .. }
        | PyObject::GroupByIter { .. }
        | PyObject::GetItemIter { .. }
        | PyObject::CallSentinelIter { .. }
        | PyObject::DequeIter { .. }
        | PyObject::DequeRevIter { .. } => Ok(args[0].clone()),
        // Anything else (plain functions, ints, ...) is genuinely not
        // iterable. The previous fallback (`Ok(args[0].clone())`)
        // silently treated ANY object as if it were already a valid
        // iterator instead of raising here — `builtin_next` then had no
        // recognized shape to advance either, and its OWN fallback
        // apparently tried calling the object as if `__next__` meant
        // "call it", reentrantly re-borrowing the same `RefCell` and
        // panicking with "RefCell already borrowed" instead of a clean
        // `TypeError` (confirmed via `operator.countOf(countOf, countOf)`
        // — a non-iterable `BuiltinFunction` passed to `iter()` — from
        // CPython's own `test_iter.py::test_countOf`).
        other => Err(PyError::type_error(format!(
            "'{}' object is not iterable",
            other.type_name()
        ))),
    }
}


/// `range_iterator.__setstate__(state)` — real CPython's pickle protocol
/// uses this to restore a saved iteration position (`state` = number of
/// items already consumed). Since `RangeIter.current` already tracks the
/// LIVE position (not the original start), this only produces the exactly
/// correct absolute position when called on a freshly-created iterator (the
/// only realistic real-world use — restoring right after `__reduce__`/
/// unpickling, before any `next()` calls) — advancing `current` by
/// `state * step` from wherever it currently sits. Found via CPython's own
/// `test_range.py::test_iterator_setstate`.
pub fn range_iter_setstate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__setstate__() takes exactly one argument",
        ));
    }
    let state = to_index(&args[1]).map_err(|_| {
        PyError::type_error(format!(
            "an integer is required (got type {})",
            args[1].borrow().type_name()
        ))
    })?;
    let mut obj = args[0].try_borrow_mut()?;
    if let PyObject::RangeIter { current, step, .. } = &mut *obj {
        *current += &state * &*step;
    }
    Ok(py_none())
}


/// `list_iterator.__setstate__(state)` — same protocol as `range_iterator`'s
/// above, but simpler: `ListIter.index` already IS the absolute position, so
/// this just sets it directly (clamped to the list's length, matching real
/// CPython's own clamping behavior for an out-of-range state).
pub fn list_iter_setstate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__setstate__() takes exactly one argument",
        ));
    }
    let state = to_index(&args[1]).map_err(|_| {
        PyError::type_error(format!(
            "an integer is required (got type {})",
            args[1].borrow().type_name()
        ))
    })?;
    let mut obj = args[0].try_borrow_mut()?;
    if let PyObject::ListIter { list, index } = &mut *obj {
        let n = state.to_usize().unwrap_or(0).min(list.len());
        *index = n;
    }
    Ok(py_none())
}


pub fn builtin_next(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("next() takes at least 1 argument"));
    }
    // Check for __next__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__next__"),
            PyObject::Generator { .. } => {
                drop(obj);
                let next_func = args[0].borrow().get_attribute("__next__")?;
                let (_n, f) = {
                    let b = next_func.borrow();
                    if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                        (name.clone(), *func)
                    } else {
                        return Err(PyError::runtime_error("expected __next__ method"));
                    }
                };
                let result = f(&[args[0].clone()]);
                // Convert raise StopIteration into PyError::StopIteration for next() protocol
                if let Err(ref e) = result {
                    if is_stop_iteration_error(e) {
                        // `next(gen, default)` — exhausted iterators return
                        // the default instead of raising StopIteration
                        // (difflib.py's `_line_iterator` relies on it via
                        // `next(diff_lines_iterator, 'X')`).
                        if args.len() >= 2 {
                            return Ok(args[1].clone());
                        }
                        return Err(PyError::StopIteration);
                    }
                }
                return result;
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        let result = call_bound_method(f, args[0].clone(), vec![]);
        // Convert raise StopIteration into PyError::StopIteration for next() protocol
        if let Err(PyError::Exception(_, ref exc)) = result {
            let is_stop = match &*exc.borrow() {
                PyObject::Exception { typ, .. } if typ == "StopIteration" => true,
                _ => false,
            };
            if is_stop {
                return Err(PyError::StopIteration);
            }
        }
        return result;
    }
    // Fallback to list-based iteration
    // Inline types (SmallInt etc.) and Imm types (Str, Tuple, Int, etc.)
    // are not iterable iterators — return TypeError without calling
    // borrow_mut on something that doesn't support it.
    match args[0] {
        PyObjectRef::SmallInt(_)
        | PyObjectRef::SmallBool(_)
        | PyObjectRef::SmallFloat(_)
        | PyObjectRef::SmallStr(_)
        | PyObjectRef::None
        | PyObjectRef::Imm(_) => {
            return Err(PyError::type_error(format!(
                "'{}' object is not an iterator",
                args[0].get_type_name()
            )));
        }
        _ => {}
    }
    // `GroupByIter` handled as its own, separate pre-check (not inside the
    // `match &mut *obj` below) because its advance logic must call
    // arbitrary Python code (the key function, `equals()` on keys) WITHOUT
    // holding this object's own `borrow_mut()` — otherwise a reentrant
    // `next()` on this SAME groupby object from within that callback
    // (real, deliberately adversarial CPython regression test:
    // `test_groupby_reentrant_eq_does_not_crash`, gh-143543) hits the exact
    // same double-borrow panic this restructuring exists to avoid. Extract
    // the state under a SHORT borrow, do all the scanning/calling with NO
    // borrow held at all, then a second SHORT borrow to write the result
    // back.
    let is_groupby = matches!(&*args[0].borrow(), PyObject::GroupByIter { .. });
    if is_groupby {
        let (source, key_func, mut pending, exhausted) = {
            let mut obj = args[0].borrow_mut();
            if let PyObject::GroupByIter {
                source,
                key_func,
                pending,
                exhausted,
            } = &mut *obj
            {
                (source.clone(), key_func.clone(), pending.take(), *exhausted)
            } else {
                unreachable!()
            }
        };
        if exhausted {
            return if args.len() >= 2 {
                Ok(args[1].clone())
            } else {
                Err(PyError::stop_iteration())
            };
        }
        let compute_key = |v: &PyObjectRef| -> PyResult<PyObjectRef> {
            match &key_func {
                Some(f) => call_bound_method(f.clone(), v.clone(), vec![]),
                None => Ok(v.clone()),
            }
        };
        // First item of this group: either carried over from the previous
        // call's lookahead, or freshly read from the source.
        let (this_key, first_val) = match pending.take() {
            Some((k, v)) => (k, v),
            None => match builtin_next(&[source.clone()]) {
                Ok(v) => {
                    let k = compute_key(&v)?;
                    (k, v)
                }
                Err(PyError::StopIteration) => {
                    // `next()` on a non-iterator must raise TypeError, not panic: only
                    // Mut-wrapped values are borrow_mut-able below, so an Imm/inline value
                    // (str, int, tuple, Function, ...) reaching this point would hit
                    // borrow_mut's "non-Mut value" panic instead (repro: `next('abc')`).
                    if !matches!(args[0], PyObjectRef::Mut(_)) {
                        return Err(PyError::type_error(format!(
                            "'{}' is not an iterator",
                            args[0].borrow().type_name()
                        )));
                    }
                    let mut obj = args[0].borrow_mut();
                    if let PyObject::GroupByIter { exhausted, .. } = &mut *obj {
                        *exhausted = true;
                    }
                    return if args.len() >= 2 {
                        Ok(args[1].clone())
                    } else {
                        Err(PyError::stop_iteration())
                    };
                }
                Err(e) => return Err(e),
            },
        };
        let mut group = vec![first_val];
        let mut new_pending = None;
        let mut new_exhausted = false;
        loop {
            match builtin_next(&[source.clone()]) {
                Ok(v) => {
                    let k = compute_key(&v)?;
                    if this_key.equals(&k)? {
                        group.push(v);
                    } else {
                        new_pending = Some((k, v));
                        break;
                    }
                }
                Err(PyError::StopIteration) => {
                    new_exhausted = true;
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        {
            let mut obj = args[0].borrow_mut();
            if let PyObject::GroupByIter {
                pending, exhausted, ..
            } = &mut *obj
            {
                *pending = new_pending;
                *exhausted = new_exhausted;
            }
        }
        return Ok(py_tuple(vec![
            this_key,
            PyObjectRef::new(PyObject::ListIter {
                list: group,
                index: 0,
            }),
        ]));
    }
    // Same reentrancy concern as `GroupByIter` just above: advancing this
    // needs to call the underlying object's own `__getitem__` (arbitrary
    // Python), so extract state under a short borrow, call with NO borrow
    // held, then a second short borrow to write the new index back.
    let getitem_state = {
        let obj = args[0].borrow();
        if let PyObject::GetItemIter { obj: inner, index } = &*obj {
            Some((inner.clone(), *index))
        } else {
            None
        }
    };
    let call_sentinel_state = {
        let obj = args[0].borrow();
        if let PyObject::CallSentinelIter {
            func,
            sentinel,
            exhausted,
        } = &*obj
        {
            Some((func.clone(), sentinel.clone(), *exhausted))
        } else {
            None
        }
    };
    if let Some((func, sentinel, exhausted)) = call_sentinel_state {
        if exhausted {
            return if args.len() >= 2 {
                Ok(args[1].clone())
            } else {
                Err(PyError::stop_iteration())
            };
        }
        let result = builtin_call(&func, &[])?;
        if result.equals(&sentinel)? {
            let mut obj = args[0].borrow_mut();
            if let PyObject::CallSentinelIter { exhausted, .. } = &mut *obj {
                *exhausted = true;
            }
            return if args.len() >= 2 {
                Ok(args[1].clone())
            } else {
                Err(PyError::stop_iteration())
            };
        }
        return Ok(result);
    }
    if let Some((inner, index)) = getitem_state {
        return match py_getitem(&inner, &py_int(index)) {
            Ok(v) => {
                let mut obj = args[0].borrow_mut();
                if let PyObject::GetItemIter { index, .. } = &mut *obj {
                    *index += 1;
                }
                Ok(v)
            }
            // Real Python accepts a Python-level `raise IndexError(...)`
            // from a custom `__getitem__` just as readily as this
            // interpreter's own native `PyError::IndexError` — not checking
            // the `PyError::Exception` form too meant a completely
            // ordinary `class C: def __getitem__(self, i): if i >= n: raise
            // IndexError` (the standard idiom) would propagate the
            // IndexError instead of stopping iteration.
            Err(ref e)
                if matches!(e, PyError::IndexError(_))
                    || matches!(e, PyError::Exception(_, exc) if matches!(&*exc.borrow(), PyObject::Exception { typ, .. } if crate::vm::is_exception_subclass(typ, "IndexError"))) =>
            {
                // Advance past the failed index so a subsequent append doesn't
                // make an already-exhausted iterator appear to have more items
                // (test_list::test_exhausted_iterator expects `exhit` to stay []
                // after `a.append(9)` while `empit` at the same index does see
                // the new item — the difference is that `exhit` has already
                // attempted and failed at that index, while `empit` hasn't).
                let mut obj = args[0].borrow_mut();
                if let PyObject::GetItemIter { index, .. } = &mut *obj {
                    *index += 1;
                }
                drop(obj);
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            }
            Err(e) => Err(e),
        };
    }
    let mut obj = args[0].borrow_mut();
    match &mut *obj {
        PyObject::List(v) => {
            if v.is_empty() {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                // Convert to ListIter for O(1) iteration
                let list = std::mem::take(v);
                *obj = PyObject::ListIter { list, index: 0 };
                drop(obj);
                let mut obj = args[0].borrow_mut();
                if let PyObject::ListIter { list, index } = &mut *obj {
                    let v = list[*index].clone();
                    *index += 1;
                    Ok(v)
                } else {
                    unreachable!()
                }
            }
        }
        PyObject::ListIter { list, index } => {
            if *index >= list.len() {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                let v = list[*index].clone();
                *index += 1;
                Ok(v)
            }
        }
        PyObject::CycleIter { items, index } => {
            if items.is_empty() {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                let v = items[*index % items.len()].clone();
                *index += 1;
                Ok(v)
            }
        }
        PyObject::EnumerateIter { source, pos, start } => {
            // Genuinely lazy — pulls one item from the underlying `source`
            // iterator per call instead of the OLD approach (a
            // pre-materialized `items: Vec<PyObjectRef>`, built by eagerly
            // draining the whole input up front in `builtin_enumerate`).
            // That eager drain hung forever on any genuinely infinite
            // iterable (`itertools.cycle(...)`, `itertools.count()` past
            // its own internal materialization cap) — confirmed via the
            // simplest repro, `enumerate(itertools.cycle([1,2,3]))`, which
            // never even got to yield its first pair.
            let idx = *start + *pos;
            *pos += 1;
            let source = source.clone();
            drop(obj);
            match builtin_next(&[source]) {
                Ok(val) => Ok(py_tuple(vec![py_int(idx as i64), val])),
                Err(PyError::StopIteration) => {
                    if args.len() >= 2 {
                        Ok(args[1].clone())
                    } else {
                        Err(PyError::stop_iteration())
                    }
                }
                Err(e) => Err(e),
            }
        }
        PyObject::MapIterator { func, iterator } => {
            let iter = iterator.as_ref().clone();
            let next = builtin_next(&[iter]);
            match next {
                Ok(val) => {
                    if func.borrow().type_name() == "NoneType" {
                        Ok(val)
                    } else {
                        let mapped = builtin_call(func, &[val])?;
                        Ok(mapped)
                    }
                }
                Err(e) => {
                    if args.len() >= 2 {
                        Ok(args[1].clone())
                    } else {
                        Err(e)
                    }
                }
            }
        }
        PyObject::FilterIterator { func, iterator } => {
            let iter = iterator.as_ref().clone();
            loop {
                let next = builtin_next(&[iter.clone()]);
                match next {
                    Ok(val) => {
                        // `filter(None, iterable)` keeps only the TRUTHY
                        // elements of `iterable` itself (equivalent to
                        // `filter(bool, iterable)`) — the previous
                        // `is_none() || call(...).truthy()` short-circuited
                        // to unconditionally `true` whenever `func` was
                        // `None`, silently keeping EVERY element (including
                        // falsy ones like `0`/`""`/`[]`) instead of
                        // filtering by truthiness at all.
                        let should_keep = if func.borrow().type_name() == "NoneType" {
                            val.truthy()
                        } else {
                            builtin_call(func, &[val.clone()])?.truthy()
                        };
                        if should_keep {
                            return Ok(val);
                        }
                    }
                    Err(e) => {
                        if args.len() >= 2 {
                            return Ok(args[1].clone());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        }
        PyObject::ZipIterator { iterators } => {
            let mut results = Vec::with_capacity(iterators.len());
            for it in iterators.iter() {
                match builtin_next(&[it.clone()]) {
                    Ok(val) => results.push(val),
                    Err(e) => {
                        if args.len() >= 2 {
                            return Ok(args[1].clone());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            Ok(py_tuple(results))
        }
        PyObject::RangeIter {
            current,
            stop,
            step,
        } => {
            let exhausted = (step.sign() == num_bigint::Sign::Plus && current >= stop)
                || (step.sign() == num_bigint::Sign::Minus && current <= stop);
            if exhausted {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else {
                let v = py_int(current.clone());
                *current += &*step;
                Ok(v)
            }
        }
        PyObject::DequeIter {
            deque,
            index,
            start_len,
        } => {
            let native = crate::object::native_backing_of(deque).or_else(|| {
                let dq = deque.borrow();
                if matches!(&*dq, PyObject::Deque { .. }) {
                    Some(deque.clone())
                } else {
                    None
                }
            }).ok_or_else(|| PyError::runtime_error("deque iterator over non-deque"))?;
            let (is_done, maybe_item) = {
                let nb = native.borrow();
                if let PyObject::Deque { data, .. } = &*nb {
                    if data.len() != *start_len {
                        return Err(PyError::runtime_error("deque mutated during iteration"));
                    }
                    if *index >= data.len() {
                        (true, None)
                    } else {
                        (false, Some(data[*index].clone()))
                    }
                } else {
                    (true, None)
                }
            };
            if is_done {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else if let Some(v) = maybe_item {
                if let PyObject::DequeIter { index, .. } = &mut *obj {
                    *index += 1;
                }
                Ok(v)
            } else {
                Err(PyError::runtime_error("deque iterator over non-deque"))
            }
        }
        PyObject::DequeRevIter {
            deque,
            index,
            start_len,
        } => {
            let native = crate::object::native_backing_of(deque).or_else(|| {
                let dq = deque.borrow();
                if matches!(&*dq, PyObject::Deque { .. }) {
                    Some(deque.clone())
                } else {
                    None
                }
            }).ok_or_else(|| PyError::runtime_error("deque iterator over non-deque"))?;
            let (is_done, maybe_item) = {
                let nb = native.borrow();
                if let PyObject::Deque { data, .. } = &*nb {
                    if data.len() != *start_len {
                        return Err(PyError::runtime_error("deque mutated during iteration"));
                    }
                    if *index < 0 || (*index as usize) >= data.len() {
                        (true, None)
                    } else {
                        (false, Some(data[*index as usize].clone()))
                    }
                } else {
                    (true, None)
                }
            };
            if is_done {
                if args.len() >= 2 {
                    Ok(args[1].clone())
                } else {
                    Err(PyError::stop_iteration())
                }
            } else if let Some(v) = maybe_item {
                if let PyObject::DequeRevIter { index, .. } = &mut *obj {
                    *index -= 1;
                }
                Ok(v)
            } else {
                Err(PyError::runtime_error("deque iterator over non-deque"))
            }
        }
        _ => Err(PyError::type_error(format!(
            "'{}' is not an iterator",
            obj.type_name()
        ))),
    }
}



pub fn builtin_reversed(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("reversed() takes exactly one argument"));
    }
    // Check type with a short-lived borrow to avoid holding the RefCell
    // borrow while iterating (which could trigger borrow_mut conflicts).
    // `range` needs its own O(1) case (real CPython's `range.__reversed__`)
    // — without this it fell into the generic "drain every element into a
    // Vec, then reverse" fallback further down, which for a `range` spanning
    // billions of elements tries to materialize the WHOLE thing first. Same
    // unbounded-incremental-growth bug as the `list()`/`list * n` memory
    // bombs fixed elsewhere (confirmed via CPython's own `test_range.py`,
    // `test_range_iterators`, whose `reversed(range(start, end, step))`
    // calls span ranges up to ~2**33 elements — enough to consume all
    // available RAM before ever finishing). `range`'s length is always
    // O(1) to compute, so the reversed sequence can be derived directly,
    // arithmetically, without ever iterating the original.
    {
        let obj = args[0].borrow();
        if let PyObject::Range { start, stop, step } = &*obj {
            let (start, stop, step) = (start.clone(), stop.clone(), step.clone());
            let empty = (step.sign() == num_bigint::Sign::Plus && start >= stop)
                || (step.sign() == num_bigint::Sign::Minus && start <= stop);
            if empty {
                return Ok(PyObjectRef::new(PyObject::RangeIter {
                    current: num_bigint::BigInt::from(0),
                    stop: num_bigint::BigInt::from(0),
                    step: num_bigint::BigInt::from(1),
                }));
            }
            let raw_len = &stop - &start;
            let q = &raw_len / &step;
            let count = if (&raw_len % &step).sign() != num_bigint::Sign::NoSign {
                q.abs() + 1
            } else {
                q.abs()
            };
            let last = &start + (&count - 1) * &step;
            let new_stop = &start - &step;
            return Ok(PyObjectRef::new(PyObject::RangeIter {
                current: last,
                stop: new_stop,
                step: -step,
            }));
        }
    }
    let kind = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(_) => 1,
            PyObject::Tuple(_) => 2,
            PyObject::Str(_) => 3,
            _ => 0,
        }
    };
    if kind != 0 {
        let obj = args[0].borrow();
        return match &*obj {
            PyObject::List(v) => {
                let mut rev = v.clone();
                rev.reverse();
                // A GetItemIter (no __len__), NOT a ListIter: real CPython's
                // reversed-list iterator has no `__len__`, so
                // `len(reversed([1,2,3]))` must raise TypeError
                // (test_list::test_reversed).
                Ok(PyObjectRef::new(PyObject::GetItemIter {
                    obj: PyObjectRef::new(PyObject::List(rev)),
                    index: 0,
                }))
            }
            PyObject::Tuple(v) => {
                let mut rev = v.clone();
                rev.reverse();
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: rev,
                    index: 0,
                }))
            }
            PyObject::Str(s) => {
                let chars: Vec<PyObjectRef> =
                    s.chars().rev().map(|c| py_str(&c.to_string())).collect();
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: chars,
                    index: 0,
                }))
            }
            _ => unreachable!(),
        };
    }
    // Native-backed sequence subclasses (e.g. `class A(tuple): ...`) store
    // their value in `__native__` and have no explicit `__len__`/
    // `__getitem__` in the type's MRO (tuple's own dunders are not in the
    // type dict). `builtin_len` already falls back to the native backing,
    // but the `lookup_dunder_via_mro` check below would still fail and raise
    // "argument to reversed() must be a sequence" — real trigger:
    // `test_tuple.py::test_free_after_iterating` which does `reversed(A())`
    // where `A` is a tuple subclass.
    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
        if let Some(native) = dict.get(crate::object::NATIVE_BACKING_KEY).cloned() {
            let kind2 = {
                let nb = native.borrow();
                match &*nb {
                    PyObject::List(_) => 1,
                    PyObject::Tuple(_) => 2,
                    PyObject::Str(_) => 3,
                    _ => 0,
                }
            };
            if kind2 != 0 {
                let nb = native.borrow();
                return match &*nb {
                    PyObject::List(v) => {
                        let mut rev = v.clone();
                        rev.reverse();
                        Ok(PyObjectRef::new(PyObject::GetItemIter {
                            obj: PyObjectRef::new(PyObject::List(rev)),
                            index: 0,
                        }))
                    }
                    PyObject::Tuple(v) => {
                        let mut rev = v.clone();
                        rev.reverse();
                        Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }))
                    }
                    PyObject::Str(s) => {
                        let chars: Vec<PyObjectRef> =
                            s.chars().rev().map(|c| py_str(&c.to_string())).collect();
                        Ok(PyObjectRef::new(PyObject::ListIter { list: chars, index: 0 }))
                    }
                    _ => unreachable!(),
                };
            }
        }
    }
    // Native deque: reversed(deque) returns a dedicated reverse iterator
    // (deque_reverse_iterator) that is itself callable with a deque argument
    // (test_deque::test_reversed_new: klass = type(reversed(deque())) ;
    // list(klass(deque(s))) == list(reversed(s))). The generic drain fallback
    // below would instead return a list_iterator which is not callable with a
    // deque and fails that test.
    {
        let is_deque_native = matches!(&*args[0].borrow(), PyObject::Deque { .. });
        if is_deque_native {
            let (deque, start_len) = {
                let b = args[0].borrow();
                if let PyObject::Deque { data, .. } = &*b {
                    (args[0].clone(), data.len())
                } else {
                    unreachable!()
                }
            };
            let idx = if start_len == 0 { -1 } else { (start_len as isize) - 1 };
            return Ok(PyObjectRef::new(PyObject::DequeRevIter {
                deque,
                index: idx,
                start_len,
            }));
        }
        // Deque subclass (Instance with native backing == Deque) — same behavior
        // as the native case above (reversed(DequeSubclass(...)) must also be
        // a deque_reverse_iterator, not a generic list_iterator).
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            if let Some(native) = dict.get(crate::object::NATIVE_BACKING_KEY).cloned() {
                if matches!(&*native.borrow(), PyObject::Deque { .. }) {
                    let start_len = {
                        let nb = native.borrow();
                        if let PyObject::Deque { data, .. } = &*nb {
                            data.len()
                        } else {
                            0
                        }
                    };
                    let idx = if start_len == 0 { -1 } else { (start_len as isize) - 1 };
                    return Ok(PyObjectRef::new(PyObject::DequeRevIter {
                        deque: args[0].clone(),
                        index: idx,
                        start_len,
                    }));
                }
            }
        }
    }
    // Real Python's `reversed(obj)` protocol for a plain instance (no native
    // fast path above): use `obj.__reversed__()` if defined, else `obj[len(
    // obj)-1]`, `obj[len(obj)-2]`, ..., `obj[0]` via `__len__`+`__getitem__`
    // — NEVER by draining a FORWARD iterator and reversing the result
    // (the previous fallback below, which this replaces for the Instance
    // case). That forward-drain approach only happens to work for
    // `__iter__`-based objects with a genuine end; for a `__len__`+
    // `__getitem__`-only object whose `__getitem__` never raises `IndexError`
    // for an out-of-range index (a real, deliberate CPython regression
    // test's own `Seq` class: `__getitem__` unconditionally `return
    // index` — CPython's `reversed()` never needs `IndexError` from such an
    // object since it's bounded by `__len__` instead), forward-draining
    // hangs FOREVER. Found via `test_enumerate.py`'s `TestReversed.test_gc`
    // — this only started hanging once `builtin_iter`'s own new `__getitem__`
    // fallback (see `GetItemIter`) made `iter()` succeed on such objects at
    // all, where it previously raised a quick (if wrong) `TypeError`.
    let instance_typ = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
        Some(typ.clone())
    } else {
        None
    };
    if let Some(typ) = &instance_typ {
        if let Some(f) = lookup_dunder_via_mro(typ, "__reversed__") {
            // `__reversed__ = None` is real Python's documented way to
            // explicitly DISABLE reversal on a class that would otherwise
            // qualify via `__len__`/`__getitem__` — must raise `TypeError`
            // outright (matching real CPython, and `test_enumerate.py`'s
            // own `TestReversed.test_objmethods::Blocked` class), not fall
            // through to the `__len__` fallback below (which `Blocked`
            // would otherwise satisfy) or try calling `None` as a function
            // (not callable — previously produced a confusing unrelated
            // error instead of a clean `TypeError`).
            if matches!(&*f.borrow(), PyObject::None) {
                return Err(PyError::type_error(format!(
                    "'{}' object is not reversible",
                    get_type_name_for_instance(typ)
                )));
            }
            return call_bound_method(f, args[0].clone(), vec![]);
        }
        // Real Python's `reversed()` fallback (no `__reversed__`) STRICTLY
        // requires `__len__` — it does NOT support the same "call
        // `__getitem__` until `IndexError`" protocol forward iteration
        // does. An object with `__getitem__` but no `__len__` (real
        // trigger: `test_enumerate.py`'s own `TestReversed.test_objmethods`,
        // `class NoLen: def __getitem__(self, i): return 1`) must raise
        // `TypeError` here, NOT fall through to the generic "unknown type:
        // drain via iteration" path below — that path now succeeds (via
        // `GetItemIter`) but drains FOREVER for an object whose
        // `__getitem__` never raises `IndexError` for any index (which
        // `reversed()` never needed to rely on in the first place, since
        // real CPython bounds the count via `__len__` instead).
        return if lookup_dunder_via_mro(typ, "__len__").is_some()
            && lookup_dunder_via_mro(typ, "__getitem__").is_some()
        {
            let len = builtin_len(&[args[0].clone()])?
                .as_i64()
                .ok_or_else(|| PyError::type_error("__len__() should return an int"))?;
            let mut v = Vec::with_capacity(len.max(0) as usize);
            let mut i = len - 1;
            while i >= 0 {
                v.push(py_getitem(&args[0], &py_int(i))?);
                i -= 1;
            }
            Ok(PyObjectRef::new(PyObject::ListIter { list: v, index: 0 }))
        } else {
            Err(PyError::type_error(
                "argument to reversed() must be a sequence",
            ))
        };
    }
    // Unknown type: drain via iteration (no active borrow on args[0])
    let mut v = Vec::new();
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => v.push(val),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    v.reverse();
    Ok(PyObjectRef::new(PyObject::ListIter { list: v, index: 0 }))
}
