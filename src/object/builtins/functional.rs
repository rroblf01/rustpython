// Split out of the former monolithic object/builtins.rs — this file holds
// functional / aggregation builtins (`sorted`, `enumerate`, `sum`, `max`,
// `min`, `any`, `all`, `callable`, `breakpoint`) and their helpers.
use super::*;

pub fn builtin_sorted(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("sorted() takes at least 1 argument"));
    }
    // Check for key/reverse keyword arguments (last arg could be a dict with
    // "key"/"reverse").
    let key_fn: Option<PyObjectRef> = if args.len() >= 2 {
        // Check if last arg is a dict (keyword args container)
        let last = args.last().unwrap();
        let last_borrowed = last.borrow();
        if let PyObject::Dict(kwargs) = &*last_borrowed {
            kwargs.get(&py_str("key")).unwrap_or(None)
        } else {
            None
        }
    } else {
        None
    };
    let reverse: bool = if args.len() >= 2 {
        let last = args.last().unwrap();
        let last_borrowed = last.borrow();
        if let PyObject::Dict(kwargs) = &*last_borrowed {
            kwargs
                .get(&py_str("reverse"))
                .unwrap_or(None)
                .map(|v| v.truthy())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };
    let mut v = Vec::new();
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => v.push(val),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    // Sort with comparison, optionally applying key function. Uses the
    // panic-tolerant `py_stable_sort_by` (see its own doc comment) rather
    // than `Vec::sort_by`, since a deliberately-inconsistent comparator
    // (real CPython test: `test_sort.py`'s `test_bug453523`) makes the
    // standard library's sort abort the whole process.
    let len = v.len();
    if len > 1 {
        let key_fn_ref = key_fn.clone();
        let reverse_flag = reverse;
        v = py_stable_sort_by(v, &|a, b| {
            let a_val = if let Some(ref kf) = key_fn_ref {
                call_bound_method(kf.clone(), a.clone(), vec![]).unwrap_or_else(|_| a.clone())
            } else {
                a.clone()
            };
            let b_val = if let Some(ref kf) = key_fn_ref {
                call_bound_method(kf.clone(), b.clone(), vec![]).unwrap_or_else(|_| b.clone())
            } else {
                b.clone()
            };
            // Route through py_compare (not the raw Compare trait methods)
            // so user-defined classes' __lt__/__gt__ are consulted — the
            // trait impl alone has no notion of Instance dunder dispatch.
            // When reverse is true, compare b < a instead of a < b to keep
            // stability for equal keys (CPython's sort is stable even with reverse;
            // simply reversing after sort would invert equal-key order).
            let cmp = if reverse_flag {
                py_compare(&b_val, &a_val, 0)
            } else {
                py_compare(&a_val, &b_val, 0)
            };
            cmp.map(|r| r.truthy()).unwrap_or(false)
        });
    }
    Ok(py_list(v))
}

pub fn builtin_enumerate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Keyword args (iterable=/start=) arrive as a trailing dict.
    let mut iterable: Option<PyObjectRef> = None;
    let mut start_val: Option<PyObjectRef> = None;
    let mut positional: Vec<PyObjectRef> = Vec::new();
    let kwargs_dict = if let Some(last) = args.last() {
        if let PyObject::Dict(d) = &*last.borrow() {
            let has_iterable = d.get(&py_str("iterable")).ok().flatten().is_some();
            let has_start = d.get(&py_str("start")).ok().flatten().is_some();
            if has_iterable || has_start {
                Some(last.clone())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let positional_args: &[PyObjectRef] = if kwargs_dict.is_some() {
        &args[..args.len() - 1]
    } else {
        args
    };
    if let Some(kw) = kwargs_dict {
        if let PyObject::Dict(d) = &*kw.borrow() {
            if let Ok(Some(v)) = d.get(&py_str("iterable")) {
                iterable = Some(v.clone());
            }
            if let Ok(Some(v)) = d.get(&py_str("start")) {
                start_val = Some(v.clone());
            }
        }
    }
    for a in positional_args {
        positional.push(a.clone());
    }
    if iterable.is_none() && positional.is_empty() {
        return Err(PyError::type_error("enumerate() takes at least 1 argument"));
    }
    if positional.len() > 2 {
        return Err(PyError::type_error(format!(
            "enumerate() takes at most 2 arguments ({} given)",
            positional.len()
        )));
    }
    let iterable = iterable.or_else(|| positional.first().cloned());
    let start_arg = start_val.or_else(|| positional.get(1).cloned());
    let start: usize = match start_arg {
        None => 0,
        Some(a) => match crate::object::int_or_bool_value(&a) {
            Some(v) => v.to_usize().unwrap_or(usize::MAX) as usize,
            None => {
                return Err(PyError::type_error(format!(
                    "enumerate() argument 2 must be int, not {}",
                    a.borrow().type_name()
                )))
            }
        },
    };
    let iterable = iterable.ok_or_else(|| PyError::type_error("enumerate() missing iterable"))?;
    // Lazily wrap the source iterator — see `PyObject::EnumerateIter`'s own
    // doc comment for why eagerly draining it here (the previous approach)
    // was a real bug, not just a style choice.
    let iterable = builtin_iter(&[iterable])?;
    Ok(PyObjectRef::new(PyObject::EnumerateIter {
        source: iterable,
        pos: 0,
        start,
    }))
}

pub fn builtin_sum(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("sum() takes at least 1 argument"));
    }
    let start = if args.len() >= 2 {
        args[1].clone()
    } else {
        py_int(0)
    };
    let mut total = start;
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => {
                total = py_add(&total, &val)?;
            }
            Err(PyError::StopIteration) => return Ok(total),
            Err(e) => return Err(e),
        }
    }
}

fn compare_gt(a: &PyObjectRef, b: &PyObjectRef) -> std::cmp::Ordering {
    // Route through py_compare so user-defined classes' __gt__/__lt__ are
    // consulted (the raw Compare trait has no notion of Instance dispatch).
    match py_compare(a, b, 4) {
        Ok(result) if result.truthy() => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Less,
    }
}

fn parse_max_min_kwargs(args: &[PyObjectRef]) -> PyResult<(Vec<PyObjectRef>, Option<PyObjectRef>, Option<PyObjectRef>, bool)> {
    if args.is_empty() {
        return Ok((Vec::new(), None, None, false));
    }
    let last = args.last().unwrap();
    let is_kwargs = if let PyObject::Dict(d) = &*last.borrow() {
        let mut has_key = false;
        let mut has_default = false;
        for (k, _) in d.iter() {
            let ks = k.str();
            if ks == "key" {
                has_key = true;
            } else if ks == "default" {
                has_default = true;
            }
        }
        has_key || has_default
    } else {
        false
    };
    if !is_kwargs {
        return Ok((args.to_vec(), None, None, false));
    }
    let dict = if let PyObject::Dict(d) = &*last.borrow() {
        (**d).clone()
    } else {
        unreachable!()
    };
    let mut key_fn: Option<PyObjectRef> = None;
    let mut default_val: Option<PyObjectRef> = None;
    let mut has_default = false;
    for (k, v) in dict.items() {
        let ks = k.str();
        if ks == "key" {
            if !matches!(&*v.borrow(), PyObject::None) {
                key_fn = Some(v.clone());
            } else {
                key_fn = None;
            }
        } else if ks == "default" {
            default_val = Some(v.clone());
            has_default = true;
        } else {
            return Err(PyError::type_error(format!(
                "max() got an unexpected keyword argument '{}'",
                ks
            )));
        }
    }
    let pos_args = args[..args.len() - 1].to_vec();
    Ok((pos_args, key_fn, default_val, has_default))
}

fn max_min_with_key(items: Vec<PyObjectRef>, key_fn: Option<PyObjectRef>, is_max: bool) -> PyResult<PyObjectRef> {
    if items.is_empty() {
        return Err(PyError::value_error("max() arg is an empty sequence"));
    }
    if key_fn.is_none() {
        if is_max {
            return items.into_iter().max_by(compare_gt)
                .ok_or_else(|| PyError::value_error("max() arg is an empty sequence"));
        } else {
            return items.into_iter().min_by(compare_gt)
                .ok_or_else(|| PyError::value_error("min() arg is an empty sequence"));
        }
    }
    let kf = key_fn.unwrap();
    let mut best: Option<PyObjectRef> = None;
    let mut best_key: Option<PyObjectRef> = None;
    for item in items {
        let k = call_bound_method(kf.clone(), item.clone(), vec![])?;
        match best {
            None => {
                best_key = Some(k);
                best = Some(item);
            }
            Some(_) => {
                let bk = best_key.as_ref().unwrap();
                let should_replace = if is_max {
                    py_compare(bk, &k, 0)?.truthy()
                } else {
                    py_compare(&k, bk, 0)?.truthy()
                };
                if should_replace {
                    best_key = Some(k);
                    best = Some(item);
                }
            }
        }
    }
    Ok(best.unwrap())
}

pub fn builtin_max(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let (pos_args, key_fn, default_val, has_default) = parse_max_min_kwargs(args)?;
    if pos_args.is_empty() {
        return Err(PyError::type_error("max() requires at least 1 argument"));
    }
    let items: Vec<PyObjectRef> = if pos_args.len() == 1 {
        let mut v = Vec::new();
        let iterable = builtin_iter(&[pos_args[0].clone()])?;
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(val) => v.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        v
    } else {
        pos_args.clone()
    };
    if items.is_empty() {
        if has_default {
            return Ok(default_val.unwrap());
        }
        return Err(PyError::value_error("max() arg is an empty sequence"));
    }
    max_min_with_key(items, key_fn, true)
}

pub fn builtin_min(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let (pos_args, key_fn, default_val, has_default) = parse_max_min_kwargs(args)?;
    if pos_args.is_empty() {
        return Err(PyError::type_error("min() requires at least 1 argument"));
    }
    let items: Vec<PyObjectRef> = if pos_args.len() == 1 {
        let mut v = Vec::new();
        let iterable = builtin_iter(&[pos_args[0].clone()])?;
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(val) => v.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        v
    } else {
        pos_args.clone()
    };
    if items.is_empty() {
        if has_default {
            return Ok(default_val.unwrap());
        }
        return Err(PyError::value_error("min() arg is an empty sequence"));
    }
    max_min_with_key(items, key_fn, false)
}

pub fn builtin_any(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("any() requires at least 1 argument"));
    }
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => {
                if val.truthy() {
                    return Ok(py_bool(true));
                }
            }
            Err(PyError::StopIteration) => return Ok(py_bool(false)),
            Err(e) => return Err(e),
        }
    }
}

pub fn builtin_all(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("all() requires at least 1 argument"));
    }
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => {
                if !val.truthy() {
                    return Ok(py_bool(false));
                }
            }
            Err(PyError::StopIteration) => return Ok(py_bool(true)),
            Err(e) => return Err(e),
        }
    }
}

pub fn builtin_callable(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("callable() takes exactly one argument"));
    }
    if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
        if let Some(rc) = target.upgrade() {
            return builtin_callable(&[PyObjectRef::Mut(rc)]);
        } else {
            return Err(PyError::reference_error("weakly-referenced object no longer exists"));
        }
    }
    let obj = args[0].borrow();
    let is_callable = matches!(
        &*obj,
        PyObject::Function(_) | PyObject::BuiltinFunction { .. } |
        PyObject::BuiltinMethod { .. } | PyObject::Type { .. } | PyObject::BuildClass |
        PyObject::BoundMethod { .. } | PyObject::Partial { .. } |
        PyObject::Generator { .. } | PyObject::Coroutine { .. } |
        // Instances may be callable if they have __call__
        PyObject::Instance { .. }
    );
    // For instances, check if the type (or a base, via MRO) has __call__
    if !is_callable {
        Ok(py_bool(false))
    } else if let PyObject::Instance { typ, .. } = &*obj {
        Ok(py_bool(lookup_dunder_via_mro(typ, "__call__").is_some()))
    } else {
        Ok(py_bool(true))
    }
}

pub fn builtin_breakpoint(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if !args.is_empty() {
        eprintln!(
            "Breakpoint reached with {} argument(s) — debugger not available in this interpreter",
            args.len()
        );
        for (i, arg) in args.iter().enumerate() {
            eprintln!("  arg[{}]: {}", i, arg.str());
        }
    } else {
        eprintln!("Breakpoint reached — debugger not available in this interpreter");
    }
    Ok(py_none())
}
