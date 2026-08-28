use crate::object::*;
use std::collections::HashMap;

/// `bisect`/`heapq` need ordering comparisons that consult a user-defined
/// class's own `__lt__` (real code: bisect-inserting/heap-ordering custom
/// objects, e.g. Django's `(creation_counter, field)` tuples during model
/// construction) — `PyObjectRef::lt()`/`Compare::lt` is a raw, native-types
/// only comparison with no dunder dispatch at all (`Instance` isn't handled,
/// always `TypeError`). `py_compare` is the general, dunder-aware version
/// already used by `sorted()`/`list.sort()` — route through it instead.
fn py_lt(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
    Ok(py_compare(a, b, 0)?.truthy())
}

pub fn create_bisect_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! bisect_func {
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

    // Shared argument parsing for every bisect/insort function: positional
    // `a, x[, lo[, hi]]` OR the keyword forms `a=..., x=..., lo=..., hi=...,
    // key=...` (the VM packs keywords into a trailing `PyObject::Dict`).
    // Returns the sequence, the probe, lo/hi as Option (None = default), and
    // the optional key callable.
    fn bisect_parse<'a>(
        args: &'a [PyObjectRef],
    ) -> PyResult<(
        PyObjectRef,
        PyObjectRef,
        Option<i64>,
        Option<i64>,
        Option<PyObjectRef>,
    )> {
        let mut pos: Vec<PyObjectRef> = args.to_vec();
        let mut kw_a: Option<PyObjectRef> = None;
        let mut kw_x: Option<PyObjectRef> = None;
        let mut kw_lo: Option<i64> = None;
        let mut kw_hi: Option<i64> = None;
        let mut kw_key: Option<PyObjectRef> = None;
        if let Some(last) = pos.last().cloned() {
            if let PyObject::Dict(d) = &*last.borrow() {
                for (k, v) in d.items() {
                    match k.str().as_str() {
                        "a" => kw_a = Some(v),
                        "x" => kw_x = Some(v),
                        "lo" => {
                            kw_lo = Some(
                                v.as_i64()
                                    .ok_or_else(|| PyError::type_error("lo must be an integer"))?,
                            )
                        }
                        "hi" => {
                            kw_hi = Some(
                                v.as_i64()
                                    .ok_or_else(|| PyError::type_error("hi must be an integer"))?,
                            )
                        }
                        "key" => kw_key = Some(v),
                        other => {
                            return Err(PyError::type_error(format!(
                                "bisect() got an unexpected keyword argument '{}'",
                                other
                            )))
                        }
                    }
                }
                pos.pop();
            }
        }
        let a = match kw_a {
            Some(a) => a,
            None => pos
                .first()
                .cloned()
                .ok_or_else(|| PyError::type_error("missing required argument: 'a'"))?,
        };
        let x = match kw_x {
            Some(x) => x,
            None => pos
                .get(1)
                .cloned()
                .ok_or_else(|| PyError::type_error("missing required argument: 'x'"))?,
        };
        let p_lo = pos
            .get(2)
            .map(|v| {
                v.as_i64()
                    .ok_or_else(|| PyError::type_error("lo must be an integer"))
            })
            .transpose()?;
        let p_hi = pos
            .get(3)
            .map(|v| {
                v.as_i64()
                    .ok_or_else(|| PyError::type_error("hi must be an integer"))
            })
            .transpose()?;
        Ok((a, x, kw_lo.or(p_lo), kw_hi.or(p_hi), kw_key))
    }

    // Apply the optional key function to `obj`.
    fn bisect_key(key: &Option<PyObjectRef>, obj: &PyObjectRef) -> PyResult<PyObjectRef> {
        match key {
            Some(k) => builtin_call(k, &[obj.clone()]),
            None => Ok(obj.clone()),
        }
    }

    // Bisect works on ANY random-access sequence (`a[mid]` + `len(a)`), not
    // just lists — real CPython's own test_bisect runs it against `range`
    // with n = sys.maxsize (test_large_range). Use the generic
    // `py_getitem`/`builtin_len` instead of destructuring a List.
    fn bisect_locate(
        a: &PyObjectRef,
        x: &PyObjectRef,
        lo: Option<i64>,
        hi: Option<i64>,
        right: bool,
        key: &Option<PyObjectRef>,
    ) -> PyResult<PyObjectRef> {
        let len = builtin_len(&[a.clone()])?
            .as_i64()
            .ok_or_else(|| PyError::type_error("sequence length must be an integer"))?;
        let key_x = bisect_key(key, x)?;
        let lo_raw = lo.unwrap_or(0);
        if lo_raw < 0 {
            return Err(PyError::value_error("lo must be non-negative"));
        }
        let hi_raw = hi.unwrap_or(len);
        if hi_raw < lo_raw {
            return Err(PyError::value_error("hi must be greater than lo"));
        }
        let mut lo = lo_raw as usize;
        let mut hi = hi_raw.min(len) as usize;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_item = crate::object::py_getitem(a, &py_int(mid as i64))?;
            let key_mid = bisect_key(key, &mid_item)?;
            if right {
                // bisect_right: find the first position where `x` can be
                // inserted while staying to the RIGHT of equal elements:
                // if key(x) < key(a[mid]) go left, else go right.
                if py_lt(&key_x, &key_mid)? {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            } else {
                // bisect_left: first position >= x.
                if py_lt(&key_mid, &key_x)? {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
        }
        Ok(py_int(lo as i64))
    }

    bisect_func!("bisect_left", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "bisect_left() missing required argument: 'a'",
            ));
        }
        let (a, x, lo, hi, key) = bisect_parse(args)?;
        bisect_locate(&a, &x, lo, hi, false, &key)
    });

    // bisect = bisect_right (CPython convention) — test_bisect asserts
    // `bisect is bisect_right`, so both names must hold the SAME object.
    let bisect_right = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "bisect_right".to_string(),
        func: |args: &[PyObjectRef]| {
            if args.is_empty() {
                return Err(PyError::type_error(
                    "bisect_right() missing required argument: 'a'",
                ));
            }
            let (a, x, lo, hi, key) = bisect_parse(args)?;
            bisect_locate(&a, &x, lo, hi, true, &key)
        },
    });
    d.insert("bisect_right".to_string(), bisect_right.clone());
    d.insert("bisect".to_string(), bisect_right);

    fn bisect_insert(
        a: &PyObjectRef,
        x: &PyObjectRef,
        lo: Option<i64>,
        hi: Option<i64>,
        right: bool,
        key: &Option<PyObjectRef>,
    ) -> PyResult<PyObjectRef> {
        let pos = bisect_locate(a, x, lo, hi, right, key)?
            .as_i64()
            .ok_or_else(|| PyError::type_error("internal"))? as usize;
        // Call `a.insert(pos, x)` — real CPython's insort goes through the
        // object's own `insert` method, so it works on list subclasses and
        // duck-typed sequences (test_bisect's custom `Range`, which records
        // last_insert, and a `List(list)` subclass with its own insert),
        // not just bare lists. Rebind a native BuiltinMethod to the real
        // `a` (get_attribute leaves a placeholder self_obj); a raw Function
        // (user-defined insert) gets `a` passed positionally as self.
        let method = a.borrow().get_attribute("insert")?;
        let result = match &*method.borrow() {
            PyObject::BuiltinMethod { name, func, .. } => {
                let bound = PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: name.clone(),
                    func: *func,
                    self_obj: a.clone(),
                });
                call_function_disposable(&bound, vec![py_int(pos as i64), x.clone()], vec![])
            }
            _ => call_function_disposable(
                &method,
                vec![a.clone(), py_int(pos as i64), x.clone()],
                vec![],
            ),
        };
        result.map(|_| py_none())
    }

    bisect_func!("insort_left", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "insort_left() missing required argument: 'a'",
            ));
        }
        let (a, x, lo, hi, key) = bisect_parse(args)?;
        bisect_insert(&a, &x, lo, hi, false, &key)
    });

    // insort = insort_right (CPython convention) — `insort is insort_right`
    // in test_bisect, so share the object.
    let insort_right = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "insort_right".to_string(),
        func: |args: &[PyObjectRef]| {
            if args.is_empty() {
                return Err(PyError::type_error(
                    "insort_right() missing required argument: 'a'",
                ));
            }
            let (a, x, lo, hi, key) = bisect_parse(args)?;
            bisect_insert(&a, &x, lo, hi, true, &key)
        },
    });
    d.insert("insort_right".to_string(), insort_right.clone());
    d.insert("insort".to_string(), insort_right);

    d
}

pub fn create_heapq_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! heap_func {
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

    // Internal: sift-down (for heappop, heapreplace, heapify)
    fn _siftdown(heap: &mut Vec<PyObjectRef>, start: usize, pos: usize) {
        let mut pos = pos;
        while pos > start {
            let parent = (pos - 1) / 2;
            if py_lt(&heap[pos], &heap[parent]).unwrap_or(false) {
                heap.swap(pos, parent);
                pos = parent;
            } else {
                break;
            }
        }
    }

    // Internal: sift-up (for heapify)
    fn _siftup(heap: &mut Vec<PyObjectRef>, pos: usize) {
        let end = heap.len();
        let mut pos = pos;
        let start = pos;
        while pos < end {
            let left = 2 * pos + 1;
            let right = 2 * pos + 2;
            let mut smallest = pos;
            if left < end && py_lt(&heap[left], &heap[smallest]).unwrap_or(false) {
                smallest = left;
            }
            if right < end && py_lt(&heap[right], &heap[smallest]).unwrap_or(false) {
                smallest = right;
            }
            if smallest == pos {
                break;
            }
            heap.swap(pos, smallest);
            pos = smallest;
        }
        // Bubble back up if needed (after moving nodes)
        _siftdown(heap, start, pos);
    }

    // `_siftdown`/`_siftup` above take a bare `&mut Vec<PyObjectRef>` — fine
    // for `nlargest`/`nsmallest`'s own purely-local working buffer (never
    // shared with Python code, so nothing can reenter and mutate it), but
    // unsafe for `heapify`/`heappush`/`heappop`/`heapreplace`, which operate
    // on the caller's REAL, live, Python-visible list: those held the
    // list's own `borrow_mut()` for the ENTIRE sift operation, including
    // every `py_lt` comparison — which can run arbitrary Python `__lt__`
    // code. Real trigger: CPython's own `test_heapq.py`'s
    // `test_comparison_operator_modifying_heap`/`..._two_heaps`, whose
    // custom `__lt__` mutates the SAME heap list mid-comparison (append/
    // clear/etc.) — needing `list.borrow_mut()` again while the outer one
    // was still held, panicking with "RefCell already mutably borrowed".
    // These `_live` variants take the list's own `PyObjectRef` instead,
    // re-borrowing briefly (and by INDEX, with an explicit bounds check —
    // matching real CPython's own C implementation, which re-fetches
    // `PyList_GET_SIZE` after every comparison for the exact same reason)
    // for each individual read/swap, never holding a borrow across a
    // comparison call.
    fn heap_get_live(heap_ref: &PyObjectRef, idx: usize) -> Option<PyObjectRef> {
        if let PyObject::List(list) = &*heap_ref.borrow() {
            list.get(idx).cloned()
        } else {
            None
        }
    }
    fn heap_len_live(heap_ref: &PyObjectRef) -> usize {
        if let PyObject::List(list) = &*heap_ref.borrow() {
            list.len()
        } else {
            0
        }
    }
    fn heap_swap_live(heap_ref: &PyObjectRef, i: usize, j: usize) {
        if let PyObject::List(list) = &mut *heap_ref.borrow_mut() {
            if i < list.len() && j < list.len() {
                list.swap(i, j);
            }
        }
    }
    fn _siftdown_live(heap_ref: &PyObjectRef, start: usize, pos: usize) {
        let mut pos = pos;
        while pos > start {
            let parent = (pos - 1) / 2;
            let (item_pos, item_parent) = match (
                heap_get_live(heap_ref, pos),
                heap_get_live(heap_ref, parent),
            ) {
                (Some(a), Some(b)) => (a, b),
                _ => return,
            };
            if py_lt(&item_pos, &item_parent).unwrap_or(false) {
                heap_swap_live(heap_ref, pos, parent);
                pos = parent;
            } else {
                break;
            }
        }
    }
    fn _siftup_live(heap_ref: &PyObjectRef, pos: usize) {
        let end = heap_len_live(heap_ref);
        let mut pos = pos;
        let start = pos;
        while pos < end {
            let left = 2 * pos + 1;
            let right = 2 * pos + 2;
            let mut smallest = pos;
            if left < end {
                if let (Some(l), Some(s)) = (
                    heap_get_live(heap_ref, left),
                    heap_get_live(heap_ref, smallest),
                ) {
                    if py_lt(&l, &s).unwrap_or(false) {
                        smallest = left;
                    }
                }
            }
            if right < end {
                if let (Some(r), Some(s)) = (
                    heap_get_live(heap_ref, right),
                    heap_get_live(heap_ref, smallest),
                ) {
                    if py_lt(&r, &s).unwrap_or(false) {
                        smallest = right;
                    }
                }
            }
            if smallest == pos {
                break;
            }
            heap_swap_live(heap_ref, pos, smallest);
            pos = smallest;
        }
        _siftdown_live(heap_ref, start, pos);
    }

    heap_func!("heapify", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("heapify() missing required argument"));
        }
        if !matches!(&*args[0].borrow(), PyObject::List(_)) {
            return Err(PyError::type_error("heapify() argument must be a list"));
        }
        let n = heap_len_live(&args[0]);
        if n > 1 {
            for i in (0..n / 2).rev() {
                _siftup_live(&args[0], i);
            }
        }
        Ok(py_none())
    });

    heap_func!("heappush", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "heappush() requires 2 arguments (heap, item)",
            ));
        }
        // Check the variant via an IMMUTABLE borrow first — `.borrow_mut()`
        // panics outright (rather than erroring) on a non-`Mut` value like
        // `PyObjectRef::None`/`SmallInt`, so calling it unconditionally
        // before confirming `args[0]` is really a list crashed instead of
        // raising `TypeError` for e.g. `heappush(None, x)`. Real trigger:
        // CPython's own `test_heapq.py`, which explicitly exercises
        // `assertRaises(TypeError, ...)` with non-list arguments.
        if !matches!(&*args[0].borrow(), PyObject::List(_)) {
            return Err(PyError::type_error("heappush() argument must be a list"));
        }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            list.push(args[1].clone());
        }
        let last = heap_len_live(&args[0]).saturating_sub(1);
        _siftdown_live(&args[0], 0, last);
        Ok(py_none())
    });

    heap_func!("heappop", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("heappop() missing required argument"));
        }
        if !matches!(&*args[0].borrow(), PyObject::List(_)) {
            return Err(PyError::type_error("heappop() argument must be a list"));
        }
        let result = if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            if list.is_empty() {
                return Err(PyError::index_error("pop from an empty heap"));
            }
            let last = list.len() - 1;
            list.swap(0, last);
            list.pop().unwrap()
        } else {
            unreachable!()
        };
        if heap_len_live(&args[0]) > 0 {
            _siftup_live(&args[0], 0);
        }
        Ok(result)
    });

    heap_func!("heapreplace", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "heapreplace() requires 2 arguments (heap, item)",
            ));
        }
        if !matches!(&*args[0].borrow(), PyObject::List(_)) {
            return Err(PyError::type_error("heapreplace() argument must be a list"));
        }
        let result = if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            if list.is_empty() {
                return Err(PyError::index_error("heapreplace() on empty heap"));
            }
            let result = list[0].clone();
            list[0] = args[1].clone();
            result
        } else {
            return Err(PyError::type_error("heapreplace() argument must be a list"));
        };
        _siftup_live(&args[0], 0);
        Ok(result)
    });

    // Helper: extract comparable values for nlargest/nsmallest
    fn _extract_items(args: &[PyObjectRef]) -> PyResult<(usize, Vec<PyObjectRef>)> {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "requires at least 2 arguments (n, iterable)",
            ));
        }
        let n = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("n must be an integer"))?;
        if n < 0 {
            return Err(PyError::value_error("n must be non-negative"));
        }
        let n = n as usize;
        // Extract items from iterable
        let iterable = crate::object::builtin_iter(&[args[1].clone()])?;
        let mut items = Vec::new();
        loop {
            match crate::object::builtin_next(&[iterable.clone()]) {
                Ok(val) => items.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok((n, items))
    }

    heap_func!("nlargest", |args| {
        let (n, mut items) = _extract_items(args)?;
        if n == 0 {
            return Ok(py_list(Vec::new()));
        }
        // Use a min-heap of size n to track largest n elements
        if items.len() <= n {
            // Sort descending
            items.sort_by(|a, b| py_lt(b, a).unwrap_or(false).cmp(&true).reverse());
            return Ok(py_list(items));
        }
        // Build a min-heap of the first n elements
        let mut heap: Vec<PyObjectRef> = items.drain(..n).collect();
        if heap.len() > 1 {
            for i in (0..heap.len() / 2).rev() {
                _siftup(&mut heap, i);
            }
        }
        for item in items {
            if py_lt(&item, &heap[0]).unwrap_or(false) {
                // item < smallest in heap, skip
            } else {
                heap[0] = item;
                _siftup(&mut heap, 0);
            }
        }
        // Sort descending
        heap.sort_by(|a, b| py_lt(b, a).unwrap_or(false).cmp(&true).reverse());
        Ok(py_list(heap))
    });

    heap_func!("nsmallest", |args| {
        let (n, mut items) = _extract_items(args)?;
        if n == 0 {
            return Ok(py_list(Vec::new()));
        }
        if items.len() <= n {
            items.sort_by(|a, b| py_lt(a, b).unwrap_or(false).cmp(&true));
            return Ok(py_list(items));
        }
        // Use a max-heap (negation) of size n to track smallest n elements
        // Actually, we can use a max-heap: track largest in the small set
        // For max-heap we invert comparison
        let mut heap: Vec<PyObjectRef> = items.drain(..n).collect();
        if heap.len() > 1 {
            for i in (0..heap.len() / 2).rev() {
                _siftup_max(&mut heap, i);
            }
        }
        for item in items {
            if py_lt(&heap[0], &item).unwrap_or(false) {
                // item < heap[0], skip
            } else {
                heap[0] = item;
                _siftup_max(&mut heap, 0);
            }
        }
        heap.sort_by(|a, b| py_lt(a, b).unwrap_or(false).cmp(&true));
        Ok(py_list(heap))
    });

    fn _siftup_max(heap: &mut Vec<PyObjectRef>, pos: usize) {
        let end = heap.len();
        let mut pos = pos;
        while pos < end {
            let left = 2 * pos + 1;
            let right = 2 * pos + 2;
            let mut largest = pos;
            if left < end && py_lt(&heap[largest], &heap[left]).unwrap_or(false) {
                largest = left;
            }
            if right < end && py_lt(&heap[largest], &heap[right]).unwrap_or(false) {
                largest = right;
            }
            if largest == pos {
                break;
            }
            heap.swap(pos, largest);
            pos = largest;
        }
    }

    d
}
