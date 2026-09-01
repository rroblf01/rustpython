// Auto-extracted from src/object/attrs.rs lines 2623-3157
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Deque { data, maxlen } => {
                // `maxlen` is a read-only ATTRIBUTE (setting it raises
                // AttributeError — handled by `set_attribute`'s reject-
                // everything-for-native-values path), not a method.
                if name == "maxlen" {
                    return match maxlen {
                        Some(n) => Ok(py_int(*n as i64)),
                        None => Ok(py_none()),
                    };
                }
                match name {
                    "__init__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__init__".to_string(),
                        func: |args| {
                            // `d.__init__(iterable)` / `deque.__init__(d, iterable)`
                            // — rebuild the deque's contents, KEEPING its
                            // fixed maxlen (real CPython: `deque.__init__`
                            // never changes `maxlen`).
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "__init__() missing required argument: 'self'",
                                ));
                            }
                            let maxlen = {
                                let b = args[0].borrow();
                                if let PyObject::Deque { maxlen, .. } = &*b {
                                    *maxlen
                                } else {
                                    return Err(PyError::runtime_error("__init__ on non-deque"));
                                }
                            };
                            let mut items: Vec<PyObjectRef> = Vec::new();
                            if let Some(iterable) = args.get(1) {
                                // A trailing keywords dict (e.g. `maxlen=`)
                                // is meaningless here — maxlen is already
                                // fixed — so skip it.
                                if !matches!(&*iterable.borrow(), PyObject::Dict(_)) {
                                    let it = builtin_iter(&[iterable.clone()])?;
                                    loop {
                                        match builtin_next(&[it.clone()]) {
                                            Ok(v) => items.push(v),
                                            Err(PyError::StopIteration) => break,
                                            Err(e) => return Err(e),
                                        }
                                    }
                                }
                            }
                            if let PyObject::Deque { data, maxlen: ml } = &mut *args[0].borrow_mut()
                            {
                                data.clear();
                                for item in items {
                                    data.push_back(item);
                                    if let Some(m) = ml {
                                        while data.len() > *m {
                                            data.pop_front();
                                        }
                                    }
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "append" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "append".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "append() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Deque { data, maxlen } = &mut *args[0].borrow_mut() {
                                data.push_back(args[1].clone());
                                if let Some(maxlen) = maxlen {
                                    while data.len() > *maxlen {
                                        data.pop_front();
                                    }
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("append on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "appendleft" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "appendleft".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "appendleft() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Deque { data, maxlen } = &mut *args[0].borrow_mut() {
                                data.push_front(args[1].clone());
                                if let Some(maxlen) = maxlen {
                                    while data.len() > *maxlen {
                                        data.pop_back();
                                    }
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("appendleft on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if args.len() > 1 {
                                return Err(PyError::type_error(
                                    "pop() takes no arguments (1 given)",
                                ));
                            }
                            if let PyObject::Deque { data, .. } = &mut *args[0].borrow_mut() {
                                data.pop_back()
                                    .ok_or_else(|| PyError::index_error("pop from an empty deque"))
                            } else {
                                Err(PyError::runtime_error("pop on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "popleft" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "popleft".to_string(),
                        func: |args| {
                            if args.len() > 1 {
                                return Err(PyError::type_error(
                                    "popleft() takes no arguments (1 given)",
                                ));
                            }
                            if let PyObject::Deque { data, .. } = &mut *args[0].borrow_mut() {
                                data.pop_front()
                                    .ok_or_else(|| PyError::index_error("pop from an empty deque"))
                            } else {
                                Err(PyError::runtime_error("popleft on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "extend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "extend".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "extend() takes exactly one argument",
                                ));
                            }
                            // Materialize BEFORE taking the mutable borrow
                            // (self-extend `d.extend(d)` — the borrow would
                            // otherwise conflict, matching list.extend).
                            let it = builtin_iter(&[args[1].clone()])?;
                            let mut items = Vec::new();
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => items.push(v),
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            if let PyObject::Deque { data, maxlen } = &mut *args[0].borrow_mut() {
                                for item in items {
                                    data.push_back(item);
                                    if let Some(maxlen) = maxlen {
                                        while data.len() > *maxlen {
                                            data.pop_front();
                                        }
                                    }
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("extend on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "extendleft" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "extendleft".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "extendleft() takes exactly one argument",
                                ));
                            }
                            let it = builtin_iter(&[args[1].clone()])?;
                            let mut items = Vec::new();
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => items.push(v),
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            if let PyObject::Deque { data, maxlen } = &mut *args[0].borrow_mut() {
                                // appends in reverse order — `extendleft('bcd')`
                                // on 'a' yields 'dcba' (each item is
                                // appendleft'd in iteration order).
                                for item in items {
                                    data.push_front(item);
                                    if let Some(maxlen) = maxlen {
                                        while data.len() > *maxlen {
                                            data.pop_back();
                                        }
                                    }
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("extendleft on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Deque { data, .. } = &mut *args[0].borrow_mut() {
                                data.clear();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("clear on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rotate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rotate".to_string(),
                        func: |args| {
                            if args.len() > 2 {
                                return Err(PyError::type_error(
                                    "rotate() takes at most one argument",
                                ));
                            }
                            let n = if args.len() < 2 {
                                1
                            } else {
                                args[1]
                                    .as_i64()
                                    .ok_or_else(|| PyError::type_error("an integer is required"))?
                            };
                            if let PyObject::Deque { data, .. } = &mut *args[0].borrow_mut() {
                                if !data.is_empty() {
                                    let len = data.len() as i64;
                                    let n = n.rem_euclid(len);
                                    data.rotate_right(n as usize);
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("rotate on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| {
                            if args.len() != 2 {
                                return Err(PyError::type_error(
                                    "count() takes exactly one argument",
                                ));
                            }
                            let (items, start_len) = snapshot_deque(&args[0])?;
                            let mut c = 0;
                            for item in &items {
                                if deque_rich_eq(item, &args[1])? {
                                    c += 1;
                                }
                                check_deque_not_mutated(&args[0], start_len, "runtime")?;
                            }
                            Ok(py_int(c as i64))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "index".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "index() takes at least 1 argument",
                                ));
                            }
                            let (items, start_len) = snapshot_deque(&args[0])?;
                            // start/stop are clamped with BIGINT arithmetic —
                            // real code passes `sys.maxsize`-scale bounds
                            // (e.g. `seq_tests`'s `4*sys.maxsize`) that
                            // overflow i64 (`as_i64()` returns None and the
                            // `unwrap_or(0)` fallback then silently changed
                            // a huge positive stop into 0).
                            let start = if args.len() > 2 {
                                crate::object::to_index(&args[2]).ok()
                            } else {
                                None
                            };
                            let stop = if args.len() > 3 {
                                crate::object::to_index(&args[3]).ok()
                            } else {
                                None
                            };
                            let len = num_bigint::BigInt::from(items.len());
                            let zero = num_bigint::BigInt::from(0);
                            use num_traits::Signed;
                            let start_n = match &start {
                                Some(s) if s.sign() == num_bigint::Sign::Minus => {
                                    (&len + s).max(zero.clone()).to_usize().unwrap_or(0)
                                }
                                Some(s) => s.min(&len).to_usize().unwrap_or(items.len()),
                                None => 0,
                            };
                            let stop_n = match &stop {
                                Some(s) if s.sign() == num_bigint::Sign::Minus => {
                                    (&len + s).max(zero.clone()).to_usize().unwrap_or(0)
                                }
                                Some(s) => s.min(&len).to_usize().unwrap_or(items.len()),
                                None => items.len(),
                            };
                            for i in start_n..stop_n {
                                if deque_rich_eq(&items[i], &args[1])? {
                                    return Ok(py_int(i as i64));
                                }
                                check_deque_not_mutated(&args[0], start_len, "runtime")?;
                            }
                            Err(PyError::value_error(format!(
                                "{} is not in deque",
                                args[1].str()
                            )))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "insert" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "insert".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "insert() takes exactly 2 arguments",
                                ));
                            }
                            let idx = args[1]
                                .as_i64()
                                .ok_or_else(|| PyError::type_error("an integer is required"))?;
                            if let PyObject::Deque { data, maxlen } = &mut *args[0].borrow_mut() {
                                // Inserting into a FULL bounded deque raises
                                // IndexError (CPython's `test_insert_bug_26194`).
                                if let Some(maxlen) = maxlen {
                                    if data.len() >= *maxlen {
                                        return Err(PyError::index_error(
                                            "deque already at its maximum size",
                                        ));
                                    }
                                }
                                let len = data.len() as i64;
                                let idx = if idx < 0 {
                                    (len + idx).max(0)
                                } else {
                                    idx.min(len)
                                };
                                let idx = idx as usize;
                                if idx == 0 {
                                    data.push_front(args[2].clone());
                                } else if idx == len as usize {
                                    data.push_back(args[2].clone());
                                } else {
                                    // VecDeque has no mid-insert; split at idx.
                                    let back: Vec<PyObjectRef> =
                                        data.iter().skip(idx).cloned().collect();
                                    data.truncate(idx);
                                    data.push_back(args[2].clone());
                                    for item in back {
                                        data.push_back(item);
                                    }
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("insert on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "remove" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "remove".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "remove() takes exactly one argument",
                                ));
                            }
                            // Snapshot items + find match WITHOUT holding the
                            // borrow; a comparison may mutate the deque (or
                            // raise). Only delete from the LIVE deque after a
                            // clean scan, and re-check the index is still in
                            // range (CPython: `d.remove('c')` on a deque a
                            // mutator cleared raises IndexError, and a failed
                            // scan leaves the deque unchanged).
                            let (items, start_len) = snapshot_deque(&args[0])?;
                            let mut pos = None;
                            for (i, item) in items.iter().enumerate() {
                                if deque_rich_eq(item, &args[1])? {
                                    pos = Some(i);
                                    break;
                                }
                                check_deque_not_mutated(&args[0], start_len, "index")?;
                            }
                            let pos = match pos {
                                Some(p) => p,
                                None => {
                                    check_deque_not_mutated(&args[0], start_len, "index")?;
                                    return Err(PyError::value_error(format!(
                                        "{} is not in deque",
                                        args[1].str()
                                    )));
                                }
                            };
                            if let PyObject::Deque { data, .. } = &mut *args[0].borrow_mut() {
                                if pos < data.len() {
                                    data.remove(pos);
                                    Ok(py_none())
                                } else {
                                    Err(PyError::index_error("deque mutated during remove"))
                                }
                            } else {
                                Err(PyError::runtime_error("remove on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "reverse" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "reverse".to_string(),
                        func: |args| {
                            if args.len() > 1 {
                                return Err(PyError::type_error(
                                    "reverse() takes no arguments (1 given)",
                                ));
                            }
                            if let PyObject::Deque { data, .. } = &mut *args[0].borrow_mut() {
                                let items: Vec<PyObjectRef> = data.iter().cloned().collect();
                                *data = items.into_iter().rev().collect();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("reverse on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" | "__copy__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            if let PyObject::Deque { data, maxlen } = &*args[0].borrow() {
                                Ok(py_deque(data.clone(), *maxlen))
                            } else {
                                Err(PyError::runtime_error("copy on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__reversed__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__reversed__".to_string(),
                        func: |args| {
                            if let PyObject::Deque { data, .. } = &*args[0].borrow() {
                                let items: Vec<PyObjectRef> = data.iter().cloned().collect();
                                Ok(PyObjectRef::new(PyObject::List(
                                    items.into_iter().rev().collect(),
                                )))
                            } else {
                                Err(PyError::runtime_error("__reversed__ on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Deque { data, .. } = &*args[0].borrow() {
                                Ok(py_int((48 + (data.len() as i64) * 8) + 8))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-deque"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__contains__() takes exactly one argument",
                                ));
                            }
                            Ok(py_bool(crate::object::contains_op(&args[0], &args[1])?))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__getitem__() takes exactly one argument",
                                ));
                            }
                            py_getitem(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__setitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setitem__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "__setitem__() takes exactly 2 arguments",
                                ));
                            }
                            py_setitem(&args[0], &args[1], args[2].clone())?;
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__delitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__delitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__delitem__() takes exactly one argument",
                                ));
                            }
                            py_delitem(&args[0], &args[1])?;
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'deque' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}

