use super::*;
use super::iter1::{builtin_iter, builtin_len};

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
