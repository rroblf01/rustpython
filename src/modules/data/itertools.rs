use crate::object::*;
use std::collections::HashMap;

pub fn create_itertools_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! it_func {
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

    // chain is represented as a callable Instance (not a bare
    // BuiltinFunction) so it can also expose `chain.from_iterable(...)` —
    // BuiltinFunction has no attribute storage at all (set_attribute has no
    // arm for it), so a plain function couldn't hold a from_iterable
    // sibling method the way real itertools.chain does.
    {
        let mut chain_type_dict = HashMap::new();
        chain_type_dict.insert_str(
            "__call__",
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    // vm.call_function's `__call__` dispatch always prepends self
                    // (matching a real Python `__call__(self, *args)` method) before
                    // calling whatever `__call__` resolves to — unlike attribute
                    // access via LOAD_ATTR, which does NOT auto-bind a bare Closure.
                    // args[0] here is the chain instance itself; skip it.
                    let mut items = Vec::new();
                    for arg in args.iter().skip(1) {
                        if let Ok(it) = builtin_iter(&[arg.clone()]) {
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => items.push(v),
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                    Ok(py_list(items))
                },
            ))),
        );
        chain_type_dict.insert_str(
            "from_iterable",
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if args.is_empty() {
                        return Err(PyError::type_error("from_iterable() missing argument"));
                    }
                    let mut items = Vec::new();
                    if let Ok(outer_it) = builtin_iter(&[args[0].clone()]) {
                        loop {
                            match builtin_next(&[outer_it.clone()]) {
                                Ok(inner) => {
                                    if let Ok(inner_it) = builtin_iter(&[inner]) {
                                        loop {
                                            match builtin_next(&[inner_it.clone()]) {
                                                Ok(v) => items.push(v),
                                                Err(PyError::StopIteration) => break,
                                                Err(e) => return Err(e),
                                            }
                                        }
                                    }
                                }
                                Err(PyError::StopIteration) => break,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    Ok(py_list(items))
                },
            ))),
        );
        let chain_type = PyObjectRef::new(PyObject::Type {
            name: "chain".to_string(),
            dict: Box::new(str_map_to_typedict(chain_type_dict)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str(
            "chain",
            PyObjectRef::new(PyObject::Instance {
                typ: chain_type,
                dict: AttrMap::new(),
            }),
        );
    }

    it_func!("count", |args| {
        let start = if args.len() > 0 {
            if let Some(n) = args[0].as_i64() {
                n
            } else {
                0i64
            }
        } else {
            0i64
        };
        let step = if args.len() > 1 {
            if let Some(n) = args[1].as_i64() {
                n
            } else {
                1i64
            }
        } else {
            1i64
        };
        let mut current = start;
        let mut items = Vec::new();
        for _ in 0..10000 {
            items.push(py_int(current));
            current += step;
        }
        Ok(py_list(items))
    });

    // `itertools.cycle(iterable)` was missing entirely — unlike this
    // file's other itertools functions (`count`/`repeat`/etc.), which
    // approximate "infinite" by eagerly materializing a large-but-bounded
    // number of items, `cycle` gets a REAL lazy iterator (`PyObject::
    // CycleIter`, `object.rs`) since eager materialization is simply
    // impossible for something with no natural cutoff at all — real code
    // commonly relies on `cycle()` running genuinely forever (e.g. paired
    // with `itertools.islice` to take just the first N, or driven by an
    // external `break`).
    it_func!("cycle", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("cycle() missing required argument"));
        }
        let it = builtin_iter(&[args[0].clone()])?;
        let mut items = Vec::new();
        loop {
            match builtin_next(&[it.clone()]) {
                Ok(v) => items.push(v),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(PyObjectRef::new(PyObject::CycleIter { items, index: 0 }))
    });

    it_func!("product", |args| {
        let mut args: Vec<PyObjectRef> = args.to_vec();
        let mut repeat = 1;
        if let Some(last) = args.last().map(|a| a.clone()) {
            let is_dict = matches!(&*last.borrow(), PyObject::Dict(_));
            if is_dict {
                if let PyObject::Dict(dict) = &*last.borrow() {
                    if let Ok(Some(r)) = dict.get(&crate::object::py_str("repeat")) {
                        repeat = r
                            .as_i64()
                            .ok_or_else(|| PyError::type_error("repeat must be int"))?
                            as usize;
                    }
                }
                args.pop();
            }
        }
        if args.is_empty() || repeat == 0 {
            return Ok(py_list(vec![py_tuple(vec![])]));
        }
        let mut pools: Vec<Vec<PyObjectRef>> = Vec::new();
        for _ in 0..repeat {
            for arg in &args {
                let mut pool = Vec::new();
                if let Ok(it) = builtin_iter(&[arg.clone()]) {
                    loop {
                        match builtin_next(&[it.clone()]) {
                            Ok(v) => pool.push(v),
                            Err(PyError::StopIteration) => break,
                            Err(e) => return Err(e),
                        }
                    }
                }
                pools.push(pool);
            }
        }
        let mut result = vec![vec![]];
        for pool in &pools {
            let mut new_result = Vec::new();
            for prefix in &result {
                for item in pool {
                    let mut new_prefix = prefix.clone();
                    new_prefix.push(item.clone());
                    new_result.push(new_prefix);
                }
            }
            result = new_result;
        }
        Ok(py_list(result.into_iter().map(|v| py_tuple(v)).collect()))
    });

    it_func!("combinations", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("combinations() missing argument"));
        }
        let mut pool = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => pool.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let n = pool.len();
        let r = if args.len() > 1 {
            args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("r must be int"))? as usize
        } else {
            n
        };
        let mut result = Vec::new();
        if r <= n {
            let mut indices: Vec<usize> = (0..r).collect();
            loop {
                result.push(py_tuple(indices.iter().map(|&i| pool[i].clone()).collect()));
                let mut i = r;
                loop {
                    if i == 0 {
                        return Ok(py_list(result));
                    }
                    i -= 1;
                    if indices[i] != i + n - r {
                        break;
                    }
                    if i == 0 {
                        return Ok(py_list(result));
                    }
                }
                indices[i] += 1;
                for j in i + 1..r {
                    indices[j] = indices[j - 1] + 1;
                }
            }
        }
        Ok(py_list(result))
    });

    it_func!("combinations_with_replacement", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "combinations_with_replacement() missing argument",
            ));
        }
        let mut pool = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => pool.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let n = pool.len();
        let r = if args.len() > 1 {
            args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("r must be int"))? as usize
        } else {
            n
        };
        let mut result = Vec::new();
        if n > 0 || r == 0 {
            let mut indices = vec![0usize; r];
            loop {
                result.push(py_tuple(indices.iter().map(|&i| pool[i].clone()).collect()));
                let mut i_opt = None;
                for i in (0..r).rev() {
                    if indices[i] != n - 1 {
                        i_opt = Some(i);
                        break;
                    }
                }
                match i_opt {
                    None => break,
                    Some(i) => {
                        let v = indices[i] + 1;
                        for j in i..r {
                            indices[j] = v;
                        }
                    }
                }
            }
        }
        Ok(py_list(result))
    });

    it_func!("permutations", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("permutations() missing argument"));
        }
        let mut pool = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => pool.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let n = pool.len();
        let r = if args.len() > 1 && !matches!(&*args[1].borrow(), PyObject::None) {
            args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("r must be int"))? as usize
        } else {
            n
        };
        let mut result = Vec::new();
        if r <= n {
            let mut indices: Vec<usize> = (0..n).collect();
            let mut cycles: Vec<usize> = (0..r).map(|i| n - i).collect();
            result.push(py_tuple(
                indices[0..r].iter().map(|&i| pool[i].clone()).collect(),
            ));
            'outer: loop {
                let mut i = r;
                loop {
                    if i == 0 {
                        break 'outer;
                    }
                    i -= 1;
                    cycles[i] -= 1;
                    if cycles[i] == 0 {
                        let first = indices[i];
                        for k in i..n - 1 {
                            indices[k] = indices[k + 1];
                        }
                        indices[n - 1] = first;
                        cycles[i] = n - i;
                    } else {
                        let j = n - cycles[i];
                        indices.swap(i, j);
                        result.push(py_tuple(
                            indices[0..r].iter().map(|&i| pool[i].clone()).collect(),
                        ));
                        continue 'outer;
                    }
                    if i == 0 {
                        break 'outer;
                    }
                }
            }
        }
        Ok(py_list(result))
    });

    it_func!("repeat", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("repeat() missing argument"));
        }
        let obj = args[0].clone();
        // `None` distinguishes "no count given" (real infinite repeat) from
        // an explicit `times=0` (a real, valid call meaning "repeat zero
        // times" — an empty iterator) — these used to collapse onto the
        // same `0` sentinel, so `itertools.repeat(x, 0)` wrongly produced
        // 1000 items instead of none.
        let times: Option<usize> = if args.len() > 1 {
            let n = args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("times must be int"))?;
            Some(n.max(0) as usize)
        } else {
            None
        };
        // Cap materialization regardless of the requested count — this
        // itertools implementation is eager (builds a real list), not a
        // true lazy iterator, so an astronomically large explicit count
        // (a common real-world test pattern like `repeat(x, sys.maxsize)`
        // combined with `islice` to only ever pull a few items, relying on
        // real itertools' laziness to never actually materialize the rest)
        // would otherwise try to allocate a vector sized by that count
        // directly, crashing with a Rust allocator "capacity overflow"
        // panic instead of a graceful Python-level result. Real trigger:
        // CPython's own `test_itertools.py`.
        const MAX_MATERIALIZED: usize = 100_000;
        let n = times.unwrap_or(1000).min(MAX_MATERIALIZED);
        let mut items = Vec::with_capacity(n);
        for _ in 0..n {
            items.push(obj.clone());
        }
        Ok(py_list(items))
    });

    // `islice(iterable, [start,] stop[, step])` — its ENTIRE reason to
    // exist in real Python is slicing a bound out of a POTENTIALLY INFINITE
    // iterator (`itertools.count()`, `itertools.cycle()`, a hand-written
    // infinite generator) without ever materializing it in full. The
    // previous implementation eagerly drained the WHOLE input into a `Vec`
    // BEFORE looking at `start`/`stop`/`step` at all — hung forever on any
    // genuinely infinite source (confirmed via the simplest repro,
    // `list(itertools.islice(itertools.cycle('ab'), 5))`). Fixed to pull at
    // most `stop` items from the source lazily, stopping as soon as enough
    // have been read — matching real `islice`'s whole purpose. A `stop`
    // of `None` (real Python's "take everything from `start` onward," only
    // meaningful for a source that eventually ends on its own) still drains
    // to real exhaustion, same as before — that's correct there, not a bug.
    it_func!("islice", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("islice() missing arguments"));
        }
        let (start, stop, step) = match args.len() {
            1 => return Err(PyError::type_error("islice() missing stop argument")),
            2 => (
                0i64,
                if matches!(&*args[1].borrow(), PyObject::None) {
                    None
                } else {
                    Some(args[1].as_i64().unwrap_or(0))
                },
                1i64,
            ),
            _ => {
                let start = args[1].as_i64().unwrap_or(0);
                let stop = if matches!(&*args[2].borrow(), PyObject::None) {
                    None
                } else {
                    Some(args[2].as_i64().unwrap_or(0))
                };
                let step = if args.len() > 3 {
                    args[3].as_i64().unwrap_or(1)
                } else {
                    1
                };
                (start, stop, step)
            }
        };
        let start = start.max(0);
        let step = step.max(1);
        let it = builtin_iter(&[args[0].clone()])?;
        let mut result = Vec::new();
        let mut i: i64 = 0;
        loop {
            if let Some(stop_v) = stop {
                if i >= stop_v {
                    break;
                }
            }
            match builtin_next(&[it.clone()]) {
                Ok(v) => {
                    if i >= start && (i - start) % step == 0 {
                        result.push(v);
                    }
                    i += 1;
                }
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(py_list(result))
    });

    it_func!("tee", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("tee() missing argument"));
        }
        let n = if args.len() > 1 {
            args[1].as_i64().unwrap_or(2) as usize
        } else {
            2
        };
        let mut items = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => items.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let mut tees = Vec::with_capacity(n);
        for _ in 0..n {
            let it = builtin_iter(&[py_list(items.clone())])?;
            tees.push(it);
        }
        Ok(py_tuple(tees))
    });

    it_func!("zip_longest", |args| {
        let mut fillvalue = py_none();
        let mut iterables = args;
        if let Some(last) = iterables.last() {
            if let PyObject::Dict(d) = &*last.borrow() {
                if let Ok(Some(v)) = d.get(&py_str("fillvalue")) {
                    fillvalue = v;
                }
                iterables = &iterables[..iterables.len() - 1];
            }
        }
        let mut lists: Vec<Vec<PyObjectRef>> = Vec::new();
        for arg in iterables {
            let mut items = Vec::new();
            if let Ok(it) = builtin_iter(&[arg.clone()]) {
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(v) => items.push(v),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            lists.push(items);
        }
        let max_len = lists.iter().map(|l| l.len()).max().unwrap_or(0);
        let mut result = Vec::with_capacity(max_len);
        for i in 0..max_len {
            let row: Vec<PyObjectRef> = lists
                .iter()
                .map(|l| l.get(i).cloned().unwrap_or_else(|| fillvalue.clone()))
                .collect();
            result.push(py_tuple(row));
        }
        Ok(py_list(result))
    });

    it_func!("accumulate", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("accumulate() missing argument"));
        }
        let mut items = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            let mut total: Option<i64> = None;
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => {
                        if let Some(n) = v.as_i64() {
                            total = Some(total.unwrap_or(0) + n);
                            items.push(py_int(total.unwrap()));
                        } else {
                            items.push(v);
                        }
                    }
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(py_list(items))
    });

    // groupby(iterable, key=None) — groups consecutive elements sharing the
    // same key. Constructs a real, lazy `PyObject::GroupByIter` (see its
    // own doc comment in `object.rs` for why this MUST be lazy — an
    // earlier eager version crashed on CPython's own
    // `test_groupby_reentrant_eq_does_not_crash`, gh-143543); the actual
    // per-`next()` state machine lives in `builtin_next`'s dedicated
    // `GroupByIter` handling.
    it_func!("groupby", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("groupby() missing argument"));
        }
        // The key function may arrive positionally (args[1]) or as a
        // trailing kwargs dict (`key=...`) per this project's established
        // calling convention (see e.g. `str.format`'s own doc comment).
        let mut key_func: Option<PyObjectRef> = None;
        if args.len() > 1 {
            let last = &args[args.len() - 1];
            if let PyObject::Dict(d) = &*last.borrow() {
                if let Ok(Some(k)) = d.get(&py_str("key")) {
                    if !matches!(&*k.borrow(), PyObject::None) {
                        key_func = Some(k);
                    }
                }
            } else if !matches!(&*last.borrow(), PyObject::None) {
                key_func = Some(last.clone());
            }
        }
        let source = builtin_iter(&[args[0].clone()])?;
        Ok(PyObjectRef::new(PyObject::GroupByIter {
            source,
            key_func,
            pending: None,
            exhausted: false,
        }))
    });

    // filterfalse(func, iterable) — filter elements where func is False
    it_func!("filterfalse", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("filterfalse() requires 2 arguments"));
        }
        let predicate = if matches!(&*args[0].borrow(), PyObject::None) {
            None
        } else {
            Some(args[0].clone())
        };
        let iterable = crate::object::builtin_iter(&[args[1].clone()])?;
        let mut result = Vec::new();
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(item) => {
                     let should_keep = match &predicate {
                        Some(f) => {
                            // Call predicate directly; it may already be a bound
                            // method (e.g. `dict.__contains__` via `b.__contains__`)
                            // so we must not re-wrap it with a placeholder self.
                            let mut vm = crate::vm::VirtualMachine::new();
                            match vm.call_function(f.clone(), vec![item.clone()], vec![]) {
                                Ok(val) => !val.truthy(),
                                Err(_) => true,
                            }
                        }
                        None => !item.truthy(),
                    };
                    if should_keep {
                        result.push(item);
                    }
                }
                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(py_list(result))
    });

    d
}
