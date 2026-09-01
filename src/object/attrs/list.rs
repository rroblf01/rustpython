// Auto-extracted from src/object/attrs.rs lines 1909-2622
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::List(_v) => {
                match name {
                    "__iadd__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iadd__".to_string(),
                        func: |args| {
                            if args.len() != 2 {
                                return Err(PyError::type_error(format!(
                                    "__iadd__() takes exactly one argument ({} given)",
                                    args.len() - 1
                                )));
                            }
                            // Extend in place and return self (CPython's
                            // list.__iadd__). Direct `l.__iadd__(non_iterable)`
                            // must TypeError.
                            let it = crate::object::builtin_iter(&[args[1].clone()])?;
                            let mut items = Vec::new();
                            loop {
                                match crate::object::builtin_next(&[it.clone()]) {
                                    Ok(v) => items.push(v),
                                    Err(crate::object::PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                list.extend(items);
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__init__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__init__".to_string(),
                        func: |args| {
                            // `l.__init__()` clears; `l.__init__(it)` replaces
                            // (test_list::test_init).
                            let items: Vec<PyObjectRef> = if args.len() > 1 {
                                crate::object::collect_iterable(&args[1])?
                            } else {
                                Vec::new()
                            };
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                *list = items;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__imul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__imul__".to_string(),
                        func: |args| {
                            if args.len() != 2 {
                                return Err(PyError::type_error(format!(
                                    "__imul__() takes exactly one argument ({} given)",
                                    args.len() - 1
                                )));
                            }
                            let n = crate::object::to_index(&args[1])
                                .map(|n| n.to_i64().unwrap_or(0).max(0))
                                .unwrap_or(0) as usize;
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                let items: Vec<PyObjectRef> = list.clone();
                                list.clear();
                                for _ in 0..n {
                                    list.extend(items.clone());
                                }
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__mul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__mul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__rmul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__rmul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__rmul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
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
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                list.push(args[1].clone());
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("append on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__iter__" => {
                        let list_clone = _v.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                crate::object::builtin_iter(&[PyObjectRef::new(PyObject::List(list_clone.clone()))])
                            },
                        ))))
                    }
                    "__len__" => {
                        let len = _v.len() as i64;
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_int(len))
                            },
                        ))))
                    }
                    "__contains__" => {
                        let list_clone = _v.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                if args.is_empty() {
                                    return Err(PyError::type_error("__contains__() takes exactly 1 argument"));
                                }
                                for item in &list_clone {
                                    if item.borrow().equals(&args[0])? {
                                        return Ok(py_bool(true));
                                    }
                                }
                                Ok(py_bool(false))
                            },
                        ))))
                    }
                    "__getitem__" => {
                        let list_clone = _v.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                if args.is_empty() {
                                    return Err(PyError::type_error("__getitem__() takes exactly 1 argument"));
                                }
                                crate::object::py_getitem(&PyObjectRef::new(PyObject::List(list_clone.clone())), &args[0])
                            },
                        ))))
                    }
                    "__setitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setitem__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error("__setitem__() takes exactly 2 arguments"));
                            }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                let idx = crate::object::to_index(&args[1]).map_err(|_| {
                                    PyError::type_error("list indices must be integers or slices, not custom object")
                                })?;
                                let i = idx.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                                let len = list.len() as isize;
                                let i = if i < 0 { len + i } else { i };
                                if i < 0 || i >= len {
                                    return Err(PyError::index_error("list assignment index out of range"));
                                }
                                list[i as usize] = args[2].clone();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("__setitem__ on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__delitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__delitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__delitem__() takes exactly 1 argument"));
                            }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                let idx = crate::object::to_index(&args[1]).map_err(|_| {
                                    PyError::type_error("list indices must be integers or slices, not custom object")
                                })?;
                                let i = idx.to_isize().ok_or_else(|| PyError::index_error("list index out of range"))?;
                                let len = list.len() as isize;
                                let i = if i < 0 { len + i } else { i };
                                if i < 0 || i >= len {
                                    return Err(PyError::index_error("list index out of range"));
                                }
                                list.remove(i as usize);
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("__delitem__ on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if args.len() > 2 {
                                return Err(PyError::type_error(format!(
                                    "pop() takes at most one argument ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                if args.len() > 1 {
                                    let idx = args[1].as_i64().ok_or_else(|| {
                                        PyError::type_error("pop index must be an integer")
                                    })?;
                                    let len = list.len() as i64;
                                    let idx = if idx < 0 { len + idx } else { idx };
                                    if idx < 0 || idx >= len {
                                        return Err(PyError::index_error("pop index out of range"));
                                    }
                                    Ok(list.remove(idx as usize))
                                } else {
                                    list.pop()
                                        .ok_or_else(|| PyError::index_error("pop from empty list"))
                                }
                            } else {
                                Err(PyError::runtime_error("pop on non-list"))
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
                            // Materialize the iterable BEFORE taking the
                            // mutable borrow below — `args[1]` may alias
                            // `args[0]` (`d.extend(d)`, a real CPython test
                            // pattern, `test_deque.py`'s `test_extend`),
                            // which would otherwise try to `.borrow()` the
                            // same RefCell while it's already mutably
                            // borrowed by `list.push(...)`'s own
                            // `borrow_mut()`, panicking instead of
                            // completing (matches real CPython's
                            // `list.extend`, which safe-copies a
                            // self-referential source first).
                            let it = builtin_iter(&[args[1].clone()])?;
                            let mut items = Vec::new();
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => items.push(v),
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                list.extend(items);
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("extend on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if args.len() != 1 {
                                return Err(PyError::type_error(format!(
                                    "clear() takes no arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                list.clear();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("clear on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "reverse" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "reverse".to_string(),
                        func: |args| {
                            if args.len() != 1 {
                                return Err(PyError::type_error(format!(
                                    "reverse() takes no arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                list.reverse();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("reverse on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "remove" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "remove".to_string(),
                        func: |args| {
                            if args.len() != 2 {
                                return Err(PyError::type_error(format!(
                                    "remove() takes exactly one argument ({} given)",
                                    args.len() - 1
                                )));
                            }
                            let items = if let PyObject::List(list) = &*args[0].borrow() {
                                list.clone()
                            } else {
                                return Err(PyError::runtime_error("remove on non-list"));
                            };
                            // Propagate a raising __eq__ (test_remove's
                            // BadCmp/BadCmp2), don't swallow it like the old
                            // `.unwrap_or(false)` did.
                            let mut pos: Option<usize> = None;
                            for (i, item) in items.iter().enumerate() {
                                if item.is(&args[1]) || item.equals(&args[1])? {
                                    pos = Some(i);
                                    break;
                                }
                            }
                            let pos = pos.ok_or_else(|| {
                                PyError::value_error(format!("{} is not in list", args[1].str()))
                            })?;
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                list.remove(pos);
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("remove on non-list"))
                            }
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
                            let items = if let PyObject::List(list) = &*args[0].borrow() {
                                list.clone()
                            } else {
                                return Err(PyError::runtime_error("index on non-list"));
                            };
                            // `list.index(x, start, stop)` — the start/stop
                            // bounds were previously IGNORED entirely (always
                            // scanning the whole list), so `lst.index(x, 3, 1)`
                            // returned a hit where CPython raises ValueError.
                            // Apply CPython's slice-style clamping using
                            // arbitrary-precision ints (start/stop can exceed
                            // i64, e.g. `4*sys.maxsize` — as_i64 would
                            // silently collapse them to 0/MAX and miss the
                            // ValueError the test expects).
                            use num_traits::ToPrimitive;
                            let clamp = |v: &PyObjectRef, len: i64| -> i64 {
                                let n = crate::object::to_index(v).unwrap_or_else(|_| 0.into());
                                let len_big = num_bigint::BigInt::from(len);
                                let c = if n.sign() == num_bigint::Sign::Minus {
                                    (len_big.clone() + &n).max(0.into())
                                } else {
                                    n.min(len_big)
                                };
                                c.to_i64().unwrap_or(0)
                            };
                            let len = items.len() as i64;
                            let start = if args.len() > 2 {
                                clamp(&args[2], len)
                            } else {
                                0
                            };
                            let stop = if args.len() > 3 {
                                clamp(&args[3], len)
                            } else {
                                len
                            };
                            for i in start..stop {
                                if items[i as usize].is(&args[1])
                                    || items[i as usize].equals(&args[1])?
                                {
                                    return Ok(py_int(i));
                                }
                            }
                            Err(PyError::value_error(format!(
                                "{} is not in list",
                                args[1].str()
                            )))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| {
                            if args.len() != 2 {
                                return Err(PyError::type_error(format!(
                                    "count() takes exactly one argument ({} given)",
                                    args.len() - 1
                                )));
                            }
                            let items = if let PyObject::List(list) = &*args[0].borrow() {
                                list.clone()
                            } else {
                                return Err(PyError::runtime_error("count on non-list"));
                            };
                            // Propagate a raising __eq__ (test_count's BadExc),
                            // don't swallow it like `.unwrap_or(false)` did.
                            let mut c = 0i64;
                            for item in &items {
                                if item.is(&args[1]) || item.equals(&args[1])? {
                                    c += 1;
                                }
                            }
                            Ok(py_int(c))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "sort" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "sort".to_string(),
                        func: |args| {
                            if args.len() > 2
                                || (args.len() == 2
                                    && !matches!(&*args[1].borrow(), PyObject::Dict(_)))
                            {
                                return Err(PyError::type_error(format!(
                                    "sort() takes no positional arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            // Snapshot the list's items into a DETACHED `Vec`
                            // and sort THAT, rather than holding
                            // `args[0].borrow_mut()` for the whole
                            // `sort_by()` call — `py_compare` can invoke a
                            // user-defined `__lt__`/`__gt__` that mutates
                            // THIS SAME list mid-sort (real CPython handles
                            // this by sorting a detached internal copy too,
                            // then writing the result back — see
                            // `list.sort`'s own docs on "the list … is not
                            // guaranteed to be in any particular state"
                            // during a comparison that mutates it). Holding
                            // a live borrow across that used to panic with
                            // "RefCell already borrowed" the instant the
                            // reentrant comparator tried its own borrow —
                            // confirmed via CPython's own `test_sort.py`.
                            let items: Vec<PyObjectRef> = {
                                let obj = args[0].borrow();
                                match &*obj {
                                    PyObject::List(list) => list.clone(),
                                    _ => return Err(PyError::runtime_error("sort on non-list")),
                                }
                            };
                            let snapshot_len = items.len();
                            let check_not_modified = |live: &PyObjectRef| -> PyResult<()> {
                                // CPython's timsort raises ValueError when the
                                // list is modified during the sort (a
                                // self-modifying comparator) — our detached-
                                // copy approach wouldn't otherwise notice.
                                let b = live.borrow();
                                let l = match &*b {
                                    PyObject::List(l) => l.len(),
                                    _ => return Ok(()),
                                };
                                if l != snapshot_len {
                                    return Err(PyError::value_error("list modified during sort"));
                                }
                                Ok(())
                            };
                            // `sort(key=..., reverse=...)` — keyword args arrive
                            // as a trailing kwargs dict.
                            let mut key_func: Option<PyObjectRef> = None;
                            let mut reverse = false;
                            if let Some(last) = args.get(1) {
                                if let PyObject::Dict(d) = &*last.borrow() {
                                    if std::env::var("RPY_DEBUG_SORT").is_ok() {
                                        eprintln!("SORT kwargs dict, len={}", d.len());
                                    }
                                    if let Ok(Some(k)) = d.get(&py_str("key")) {
                                        if !matches!(&*k.borrow(), PyObject::None) {
                                            key_func = Some(k.clone());
                                        }
                                    }
                                    if let Ok(Some(r)) = d.get(&py_str("reverse")) {
                                        reverse = r.truthy();
                                    }
                                }
                            }
                            // Route through py_compare so user-defined
                            // classes' __lt__/__gt__ are consulted —
                            // this used to only compare ints/floats
                            // correctly and fall back to comparing
                            // str() reprs for everything else. Uses the
                            // panic-tolerant `py_stable_sort_by` (see its
                            // own doc comment) rather than `Vec::sort_by`,
                            // since a deliberately-inconsistent comparator
                            // (real CPython test: `test_bug453523`) makes
                            // the standard library's sort abort the whole
                            // process. With a `key=`, decorate-sort-undecorate:
                            // compute each element's key ONCE, sort the
                            // (key, original_item) pairs by key (stable),
                            // then drop the keys.
                            let items = if let Some(keyf) = key_func {
                                let mut decorated: Vec<(PyObjectRef, PyObjectRef)> =
                                    Vec::with_capacity(items.len());
                                for item in items.into_iter() {
                                    let k = crate::object::call_function_disposable(
                                        &keyf,
                                        vec![item.clone()],
                                        vec![],
                                    )?;
                                    decorated.push((k, item));
                                }
                                if reverse {
                                    decorated.sort_by(|a, b| {
                                        if py_compare(&b.0, &a.0, 0)
                                            .map(|r| r.truthy())
                                            .unwrap_or(false)
                                        {
                                            std::cmp::Ordering::Less
                                        } else {
                                            std::cmp::Ordering::Greater
                                        }
                                    });
                                } else {
                                    decorated.sort_by(|a, b| {
                                        if py_compare(&a.0, &b.0, 0)
                                            .map(|r| r.truthy())
                                            .unwrap_or(false)
                                        {
                                            std::cmp::Ordering::Less
                                        } else {
                                            std::cmp::Ordering::Greater
                                        }
                                    });
                                }
                                decorated.into_iter().map(|(_, item)| item).collect()
                            } else {
                                if reverse {
                                    py_stable_sort_by(items, &|a, b| {
                                        py_compare(b, a, 0).map(|r| r.truthy()).unwrap_or(false)
                                    })
                                } else {
                                    py_stable_sort_by(items, &|a, b| {
                                        py_compare(a, b, 0).map(|r| r.truthy()).unwrap_or(false)
                                    })
                                }
                            };
                            check_not_modified(&args[0])?;
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                *list = items;
                            }
                            Ok(py_none())
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
                            if let PyObject::List(list) = &mut *args[0].borrow_mut() {
                                // Negative indices were cast straight to
                                // `usize` (wrapping to a huge number that
                                // `.min(len)` then clamped to the END) —
                                // `lst.insert(-5, x)` appended instead of
                                // inserting near the front. Clamp negatives
                                // to 0 (CPython's list.insert semantics).
                                let idx = args[1].as_i64().unwrap_or(0);
                                let len = list.len() as i64;
                                let idx = if idx < 0 {
                                    (len + idx).max(0)
                                } else {
                                    idx.min(len)
                                } as usize;
                                list.insert(idx, args[2].clone());
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("insert on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            if args.len() != 1 {
                                return Err(PyError::type_error(format!(
                                    "copy() takes no arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::List(list) = &*args[0].borrow() {
                                Ok(py_list(list.clone()))
                            } else {
                                Err(PyError::runtime_error("copy on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__reversed__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__reversed__".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &*args[0].borrow() {
                                let mut rev = list.clone();
                                rev.reverse();
                                Ok(PyObjectRef::new(PyObject::List(rev)))
                            } else {
                                Err(PyError::runtime_error("__reversed__ on non-list"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::List(list) = &*args[0].borrow() {
                                Ok(py_int(56 + (list.len() as i64) * 8))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-list"))
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
                            let items = if let PyObject::List(list) = &*args[0].borrow() {
                                list.clone()
                            } else {
                                return Err(PyError::runtime_error("__contains__ on non-list"));
                            };
                            for item in items.iter() {
                                if item.is(&args[1]) || item.equals(&args[1])? {
                                    return Ok(py_bool(true));
                                }
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `__getitem__`/`__setitem__`/`__delitem__` as directly
                    // ACCESSIBLE named attributes (`[].__getitem__(0)`, not
                    // just the `[0]` subscript syntax itself, which already
                    // worked via a separate internal dispatch path) — were
                    // missing entirely, raising `AttributeError` even though
                    // `list` is a real migrated `Type` now (see this
                    // session's "native types as real Type objects" work).
                    // Real trigger: CPython's own `test_list.py`'s
                    // `test_getitem`/`test_setitem`/`test_delitem`/
                    // `test_subscript`/`test_set_subscript`, which call
                    // these by name directly. Delegate to the exact same
                    // `py_getitem`/`py_setitem`/`py_delitem` free functions
                    // the subscript operators themselves already use.
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
                        "'list' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}

