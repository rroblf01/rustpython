// Split from src/object/import_builtin.rs — super/map/filter/zip builtins.
use super::*;
use crate::object::*;

pub fn builtin_super(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // super() with no args or super(class, instance)
    if args.len() == 2 {
        let cls = args[0].clone();
        let obj = args[1].clone();
        Ok(PyObjectRef::new(PyObject::Super { cls, obj }))
    } else {
        Err(PyError::type_error("super() requires 2 arguments"))
    }
}

pub fn builtin_map(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("map() requires at least 2 arguments"));
    }
    let func = args[0].clone();
    let iter = builtin_iter(&[args[1].clone()])?;
    Ok(PyObjectRef::new(PyObject::MapIterator {
        func,
        iterator: Box::new(iter),
    }))
}

pub fn builtin_filter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("filter() requires exactly 2 arguments"));
    }
    let func = args[0].clone();
    let iter = builtin_iter(&[args[1].clone()])?;
    Ok(PyObjectRef::new(PyObject::FilterIterator {
        func,
        iterator: Box::new(iter),
    }))
}

pub fn builtin_zip(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Real CPython's `zip()` accepts zero positional iterables and returns
    // an empty iterator (`list(zip()) == []`) — it's only `map()` that
    // requires at least one. This used to reject `zip()` outright with a
    // TypeError, which broke real `Lib/_collections_abc.py`'s own
    // `zip_iterator = type(iter(zip()))` at module import time (the
    // `iterables.is_empty()` case a few lines below already builds the
    // correct empty `ZipIterator`, so this is just a spurious early
    // rejection prevented it from ever being reached with 0 args, though
    // it's also reachable with 1+ args that are all `strict=`/`strict`
    // kwargs-only, hence checking after kwargs are stripped rather than
    // moving the check there).
    // Keyword args (only `strict` is defined for zip()) arrive packed into a
    // trailing dict, per the calling convention call_function uses for all
    // BuiltinFunction calls. Without stripping it here, `zip(a, b,
    // strict=True)` treated the kwargs dict itself as one more iterable to
    // zip — iterating a dict yields its keys, so it silently zipped in the
    // literal string "strict" as a bogus extra column instead of enforcing
    // equal lengths.
    let (iterables, strict) = match args.last() {
        Some(last) => {
            let last_borrowed = last.borrow();
            if let PyObject::Dict(kwargs) = &*last_borrowed {
                let strict = kwargs
                    .get(&py_str("strict"))
                    .ok()
                    .flatten()
                    .map(|v| v.truthy())
                    .unwrap_or(false);
                (&args[..args.len() - 1], strict)
            } else {
                (args, false)
            }
        }
        None => (args, false),
    };
    if iterables.is_empty() {
        return Ok(PyObjectRef::new(PyObject::ZipIterator {
            iterators: vec![],
        }));
    }
    let iters: Vec<PyObjectRef> = iterables
        .iter()
        .map(|a| builtin_iter(&[a.clone()]))
        .collect::<PyResult<Vec<_>>>()?;
    if strict {
        // Eagerly materialize and check equal lengths — the lazy
        // ZipIterator has no way to distinguish "ran out because lengths
        // differ" from "ran out because we're done" once iteration starts,
        // so `strict` must be enforced up front.
        let mut rows: Vec<PyObjectRef> = Vec::new();
        loop {
            let mut row = Vec::with_capacity(iters.len());
            let mut stopped_indices = Vec::new();
            for (idx, it) in iters.iter().enumerate() {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => row.push(v),
                    Err(e) if is_stop_iteration_error(&e) => stopped_indices.push(idx),
                    Err(e) => return Err(e),
                }
            }
            if !stopped_indices.is_empty() {
                if stopped_indices.len() != iters.len() {
                    let shorter_at = stopped_indices[0];
                    let longer_at = (0..iters.len())
                        .find(|i| !stopped_indices.contains(i))
                        .unwrap();
                    return Err(PyError::value_error(format!(
                        "zip() argument {} is shorter than argument {}",
                        shorter_at + 1,
                        longer_at + 1,
                    )));
                }
                break;
            }
            rows.push(py_tuple(row));
        }
        return Ok(PyObjectRef::new(PyObject::ListIter {
            list: rows,
            index: 0,
        }));
    }
    Ok(PyObjectRef::new(PyObject::ZipIterator { iterators: iters }))
}
