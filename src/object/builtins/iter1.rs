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


