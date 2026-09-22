use crate::object::*;
use num_bigint::{BigInt, Sign};
use num_traits::ToPrimitive;
use std::collections::HashMap;

pub fn register_collection_types(
    builtins: &mut HashMap<String, PyObjectRef>,
    object_type: &PyObjectRef,
) {
    // `list` — same shape as `int`/`str` above. No method is reached ONLY
    // at the class level (unlike `int.from_bytes`) — every instance method
    // (`.append()`, `.sort()`, etc.) keeps resolving via native-backing
    // delegation on `class MyList(list)` instances — so `list_dict` needs
    // just the ctor marker.
    let mut list_dict: HashMap<String, PyObjectRef> = HashMap::new();
    list_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "list".to_string(),
            func: builtin_list,
        }),
    );
    list_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    list_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_list_repr as BuiltinFunc,
        }),
    );
    // Unbound CLASS-level access to list methods (`list.append`,
    // `map(list.append, ...)`, `list.__len__(a)` — test_descrtut). Same
    // delegating-closure shape as `str_unbound!` in primitive_types.rs,
    // with one crucial difference: `str`'s method bodies read the receiver
    // through the universal `.str()` conversion (which sees through
    // subclass instances), while `list`'s bodies match `PyObject::List`
    // exactly — so the receiver is first normalized to its native backing
    // (a no-op for exact lists). Without this, every subclass-instance
    // call fails with "append on non-list", and resolving through the
    // instance's own MRO instead would hand right back THIS closure and
    // recurse forever. Instance-level `a.append` keeps resolving via
    // attrs/list.rs untouched — these entries only fire for type-level
    // access and subclass instances.
    macro_rules! list_unbound {
        ($name:literal) => {
            list_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: |args: &[PyObjectRef]| {
                        if args.is_empty() {
                            return Err(PyError::type_error(concat!(
                                $name,
                                "() missing required argument: 'self'"
                            )));
                        }
                        let recv = crate::object::native_backing_of(&args[0])
                            .unwrap_or_else(|| args[0].clone());
                        let method = recv.borrow().get_attribute($name)?;
                        // Two native calling conventions coexist: most
                        // methods are `BuiltinMethod`s taking the receiver
                        // as `args[0]`, but a few (`__len__`,
                        // `__getitem__`, `__contains__`, `__iter__`) are
                        // `Closure`s with the receiver already captured —
                        // those take only the REMAINING args (passing the
                        // receiver again would shift every parameter).
                        match &*method.borrow() {
                            PyObject::BuiltinMethod { func, .. } => {
                                let mut all = vec![recv];
                                all.extend_from_slice(&args[1..]);
                                return func(&all);
                            }
                            PyObject::Closure(f) => {
                                return f(&args[1..]);
                            }
                            PyObject::BuiltinFunction { func, .. } => {
                                let mut all = vec![recv];
                                all.extend_from_slice(&args[1..]);
                                return func(&all);
                            }
                            _ => {}
                        }
                        let mut all = vec![recv];
                        all.extend_from_slice(&args[1..]);
                        crate::object::call_function_disposable(&method, all, vec![])
                    },
                }),
            );
        };
    }
    list_unbound!("append");
    list_unbound!("clear");
    list_unbound!("copy");
    list_unbound!("count");
    list_unbound!("extend");
    list_unbound!("index");
    list_unbound!("insert");
    list_unbound!("pop");
    list_unbound!("remove");
    list_unbound!("reverse");
    list_unbound!("sort");
    list_unbound!("__len__");
    list_unbound!("__getitem__");
    list_unbound!("__setitem__");
    list_unbound!("__delitem__");
    list_unbound!("__iter__");
    list_unbound!("__contains__");
    list_unbound!("__reversed__");
    list_unbound!("__sizeof__");
    // `__getstate__`/`__reduce_ex__` are REAL here (not best-effort):
    // `Lib/copy.py`'s `copy`/`deepcopy` consult `__reduce_ex__` (falling
    // back to `__reduce__`) and `__getstate__` on the INSTANCE — a stub
    // that merely fails would shadow a user-defined `__reduce__` and
    // break `test_copy.py` outright. `__reduce_ex__` mirrors CPython's
    // `object.__reduce_ex__`: defer to a user `__reduce__` override when
    // one exists anywhere in the MRO except `object` itself, else return
    // `(reconstructor, (cls,), state, iter(items), None)` — a shape the
    // vendored `copy._reconstruct` consumes directly (verified: it calls
    // `func(*args)`, applies state, extends from listiter, deepcopying
    // each when memo is set). State itself comes from the (possibly
    // custom) `__getstate__`, resolved bound. `__getstate__` alone is the
    // instance `__dict__` copy, or None when empty (matches CPython
    // exactly, probed).
    list_dict.insert_str(
        "__getstate__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getstate__".to_string(),
            func: list_getstate,
        }),
    );
    list_dict.insert_str(
        "__reduce_ex__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__reduce_ex__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "__reduce_ex__() missing required arguments",
                    ));
                }
                let receiver = args[0].clone();
                let typ = match &*receiver.borrow() {
                    PyObject::Instance { typ, .. } => typ.clone(),
                    _ => crate::object::get_primitive_type("list")
                        .expect("list type must be registered"),
                };
                // User-defined `__reduce__` anywhere in the MRO (other
                // than `object`'s own default) wins, exactly like real
                // CPython's `object.__reduce_ex__` delegating to it.
                if let Some(custom) = find_custom_reduce(&receiver) {
                    return call_user_override(&custom, &receiver, &typ);
                }
                let cls = typ;
                // State via the (possibly custom) `__getstate__`, exactly
                // like real CPython's `object.__reduce_ex__` does — this is
                // what makes a user `__getstate__` override (e.g.
                // test_descr's C3 returning `cls.ARGS`) flow into the tuple
                // instead of the raw `__dict__`. Raw MRO entries are never
                // pre-bound (binding happens at LOAD_ATTR, not in
                // `get_attribute`), so invocation uses the explicit
                // per-shape convention — never a bare bound call. Falls
                // back to the `__dict__`-or-None default when absent
                // (exact lists).
                let state = match find_custom_getstate(&receiver) {
                    Some((gs, gs_typ)) => call_user_override(&gs, &receiver, &gs_typ)?,
                    None => list_instance_state(&receiver).unwrap_or_else(py_none),
                };
                let items = list_items_of(&receiver).unwrap_or_default();
                let iter = PyObjectRef::new(PyObject::ListIter {
                    list: items,
                    index: 0,
                });
                Ok(py_tuple(vec![
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "list__reduce_new__".to_string(),
                        func: list_reduce_new,
                    }),
                    py_tuple(vec![cls]),
                    state,
                    iter,
                    py_none(),
                ]))
            },
        }),
    );
    /// `func` slot of the `__reduce_ex__` tuple above: `y = func(cls)`
    /// must yield an empty instance WITHOUT running `__init__`, WITH
    /// native backing attached (plain `cls.__new__(cls)` returns a
    /// backing-less instance for native subclasses).
    fn list_reduce_new(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        let cls = args
            .first()
            .ok_or_else(|| PyError::type_error("list reconstruction missing class argument"))?;
        if crate::object::get_primitive_type("list").map_or(false, |t| cls.is(&t)) {
            return Ok(py_list(vec![]));
        }
        match crate::object::native_base_of_type(cls).as_deref() {
            Some("list") => Ok(crate::object::make_subclass_instance(cls, py_list(vec![]))),
            _ => Err(PyError::type_error("list reconstruction of non-list class")),
        }
    }
    // In-place operators return the SAME object (identity!), including a
    // subclass instance (never its backing) — so they get direct
    // implementations, not delegation (delegating `__iadd__` would return
    // the backing and silently slice off the subclass).
    list_dict.insert_str(
        "__iadd__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__iadd__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() != 2 {
                    return Err(PyError::type_error("__iadd__() takes exactly one argument"));
                }
                let items = crate::object::collect_iterable(&args[1])?;
                let myself = args[0].clone();
                // Immutable probe FIRST: an `if let ... borrow_mut()`
                // scrutinee temporary outlives into its own `else` branch,
                // so probing mutably and then falling back to the native
                // backing panics with "already mutably borrowed". (And the
                // borrow must end before `myself` moves into the return.)
                let mut done = false;
                if matches!(&*myself.borrow(), PyObject::List(_)) {
                    if let PyObject::List(list) = &mut *myself.borrow_mut() {
                        list.extend(items);
                        done = true;
                    }
                } else if let Some(native) = crate::object::native_backing_of(&myself) {
                    if let PyObject::List(list) = &mut *native.borrow_mut() {
                        list.extend(items);
                        done = true;
                    }
                }
                if done {
                    return Ok(myself);
                }
                Err(PyError::type_error("expected a list"))
            },
        }),
    );
    list_dict.insert_str(
        "__imul__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__imul__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() != 2 {
                    return Err(PyError::type_error("__imul__() takes exactly one argument"));
                }
                let n = list_repeat_count(&args[1])?;
                let myself = args[0].clone();
                // Immutable probe first (see `__iadd__` above for why a
                // direct `borrow_mut()` scrutinee panics on the fallback).
                let mut done = false;
                if matches!(&*myself.borrow(), PyObject::List(_)) {
                    if let PyObject::List(list) = &mut *myself.borrow_mut() {
                        repeat_list_in_place(list, &n)?;
                        done = true;
                    }
                } else if let Some(native) = crate::object::native_backing_of(&myself) {
                    if let PyObject::List(list) = &mut *native.borrow_mut() {
                        repeat_list_in_place(list, &n)?;
                        done = true;
                    }
                }
                if done {
                    return Ok(myself);
                }
                Err(PyError::type_error("expected a list"))
            },
        }),
    );
    // Binary operators MUST unwrap to natives first and construct directly:
    // routing through `py_add`/`py_mul` would re-enter dunder dispatch,
    // find THIS VERY entry via the mro, and recurse forever on subclass
    // instances (exact lists never consult type dicts, so they would be
    // safe — but the entry can't tell; unwrap unconditionally).
    fn list_items_of(v: &PyObjectRef) -> Option<Vec<PyObjectRef>> {
        match &*v.borrow() {
            PyObject::List(items) => Some(items.clone()),
            _ => crate::object::native_backing_of(v).and_then(|n| match &*n.borrow() {
                PyObject::List(items) => Some(items.clone()),
                _ => None,
            }),
        }
    }
    /// Instance `__dict__` state for `__getstate__`: a fresh `Dict` copy
    /// minus the internal native-backing bookkeeping key, or `None` when
    /// there is nothing (exact lists have no dict at all). Returns `None`
    /// (not an error) when the value isn't list-shaped, mirroring lookup
    /// failure — the caller decides.
    fn list_instance_state(v: &PyObjectRef) -> Option<PyObjectRef> {
        let b = v.borrow();
        let PyObject::Instance { dict, .. } = &*b else {
            return None;
        };
        let mut pd = crate::object::PyDict::new();
        for (k, val) in dict.iter() {
            if k == crate::object::NATIVE_BACKING_KEY {
                continue;
            }
            let _ = pd.set(py_str(k), val.clone());
        }
        if pd.len() == 0 {
            None
        } else {
            Some(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
        }
    }
    /// A user-defined `__reduce__` override in the receiver's MRO (other
    /// than `object`'s own default, which is the "no override" signal).
    /// Exact lists can't have one (no per-value dict); subclass instances
    /// can (test_copy.py's `test_reduce_4tuple`).
    fn find_custom_reduce(v: &PyObjectRef) -> Option<PyObjectRef> {
        find_mro_override(v, "__reduce__")
    }
    /// A user-defined `__getstate__` override in the receiver's MRO.
    /// `list`'s own dict entry (this module's `list_getstate` below) is
    /// skipped by identity — it IS the default being defined here, not an
    /// override. Returns the raw (never pre-bound) dict entry plus the
    /// receiver's type, so the caller can invoke it with the convention
    /// matching its shape (see `call_user_override`).
    fn find_custom_getstate(v: &PyObjectRef) -> Option<(PyObjectRef, PyObjectRef)> {
        let b = v.borrow();
        let PyObject::Instance { typ, .. } = &*b else {
            return None;
        };
        let tb = typ.borrow();
        let PyObject::Type { mro, .. } = &*tb else {
            return None;
        };
        for base in mro {
            if let PyObject::Type { dict, .. } = &*base.borrow() {
                if let Some(f) = dict.get_str("__getstate__") {
                    let is_ours = matches!(&*f.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, list_getstate as BuiltinFunc));
                    if !is_ours {
                        return Some((f.clone(), typ.clone()));
                    }
                }
            }
        }
        None
    }
    /// The `__getstate__` dict entry above, as a NAMED function (not a
    /// closure) so `find_custom_getstate` can recognize it by identity
    /// and skip it when hunting user overrides.
    fn list_getstate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.is_empty() {
            return Err(PyError::type_error(
                "__getstate__() missing required argument: 'self'",
            ));
        }
        Ok(list_instance_state(&args[0]).unwrap_or_else(py_none))
    }
    /// Raw MRO-dict override lookup shared by `find_custom_reduce`
    /// (skips `object`'s default) — dict entries come back NEVER
    /// pre-bound (binding happens at the LOAD_ATTR opcode level, not in
    /// `get_attribute`), so invocation conventions below are explicit.
    fn find_mro_override(v: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
        let b = v.borrow();
        let PyObject::Instance { typ, .. } = &*b else {
            return None;
        };
        let tb = typ.borrow();
        let PyObject::Type { mro, .. } = &*tb else {
            return None;
        };
        for base in mro {
            if let PyObject::Type {
                name: bname, dict, ..
            } = &*base.borrow()
            {
                if bname == "object" {
                    continue;
                }
                if let Some(f) = dict.get_str(name) {
                    return Some(f.clone());
                }
            }
        }
        None
    }
    /// Invoke a raw MRO-dict override with the convention matching its
    /// shape: plain functions take the receiver, classmethods take the
    /// class, staticmethods take nothing. (Anything else falls back to
    /// receiver-first, matching the unbound-function convention.)
    fn call_user_override(
        func_val: &PyObjectRef,
        receiver: &PyObjectRef,
        typ: &PyObjectRef,
    ) -> PyResult<PyObjectRef> {
        match &*func_val.borrow() {
            PyObject::Function(_) => {
                crate::object::call_function_disposable(func_val, vec![receiver.clone()], vec![])
            }
            PyObject::ClassMethod { func } => {
                crate::object::call_function_disposable(func, vec![typ.clone()], vec![])
            }
            PyObject::StaticMethod { func } => {
                crate::object::call_function_disposable(func, vec![], vec![])
            }
            _ => crate::object::call_function_disposable(func_val, vec![receiver.clone()], vec![]),
        }
    }
    /// Repeat count shared by `__mul__`/`__rmul__`/`__imul__`, mirroring
    /// `py_mul`'s own overflow discipline (`test_list.py`'s
    /// `test_overflow`/`test_list_resize_overflow` expect
    /// `(MemoryError, OverflowError)` for gigantic counts, empty for
    /// negatives).
    fn list_repeat_count(v: &PyObjectRef) -> PyResult<BigInt> {
        match &*v.borrow() {
            PyObject::Int(n) => Ok(n.clone()),
            _ => Err(PyError::type_error("can't multiply sequence by non-int")),
        }
    }
    fn repeat_list_new(v: Vec<PyObjectRef>, n: &BigInt) -> PyResult<Vec<PyObjectRef>> {
        if let Some(n) = n.to_usize() {
            let mut result = Vec::new();
            match v.len().checked_mul(n) {
                Some(total) if result.try_reserve_exact(total).is_ok() => {
                    for _ in 0..n {
                        result.extend(v.iter().cloned());
                    }
                    return Ok(result);
                }
                _ => return Err(PyError::memory_error("could not allocate list")),
            }
        } else if n.sign() == Sign::Minus {
            return Ok(Vec::new());
        }
        Err(PyError::memory_error("could not allocate list"))
    }
    fn repeat_list_in_place(list: &mut Vec<PyObjectRef>, n: &BigInt) -> PyResult<()> {
        if let Some(n) = n.to_usize() {
            let orig = list.clone();
            list.clear();
            for _ in 0..n {
                list.extend(orig.iter().cloned());
            }
            return Ok(());
        } else if n.sign() == Sign::Minus {
            list.clear();
            return Ok(());
        }
        Err(PyError::memory_error("could not allocate list"))
    }
    list_dict.insert_str(
        "__add__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__add__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() != 2 {
                    return Err(PyError::type_error("__add__() takes exactly one argument"));
                }
                match (list_items_of(&args[0]), list_items_of(&args[1])) {
                    (Some(mut v), Some(w)) => {
                        v.extend(w);
                        Ok(py_list(v))
                    }
                    _ => Err(PyError::type_error(
                        "can only concatenate list (not other) to list",
                    )),
                }
            },
        }),
    );
    list_dict.insert_str(
        "__mul__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__mul__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() != 2 {
                    return Err(PyError::type_error("__mul__() takes exactly one argument"));
                }
                match list_items_of(&args[0]) {
                    Some(v) => Ok(py_list(repeat_list_new(v, &list_repeat_count(&args[1])?)?)),
                    None => Err(PyError::type_error("can't multiply sequence by non-int")),
                }
            },
        }),
    );
    list_dict.insert_str(
        "__rmul__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__rmul__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() != 2 {
                    return Err(PyError::type_error("__rmul__() takes exactly one argument"));
                }
                // Same orientation as `__mul__`: the reflected dispatch
                // (`try_dunder_binop(b, a, "__rmul__")`) already passes the
                // list as `args[0]`, so `2 * [1]` and explicit
                // `list.__rmul__([1], 2)` share this shape.
                match list_items_of(&args[0]) {
                    Some(v) => Ok(py_list(repeat_list_new(v, &list_repeat_count(&args[1])?)?)),
                    None => Err(PyError::type_error("can't multiply sequence by non-int")),
                }
            },
        }),
    );
    // Rich comparisons via the real centralized machinery, on unwrapped
    // natives (same anti-recursion reasoning as `__add__` above).
    // Codes match COMPARE_OP: 0:< 1:<= 3:>= 4:> (`__eq__`/`__ne__` already
    // come from `object`, untouched).
    macro_rules! list_richcmp {
        ($name:literal, $op:literal) => {
            list_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: |args: &[PyObjectRef]| {
                        if args.len() != 2 {
                            return Err(PyError::type_error(concat!(
                                $name,
                                "() takes exactly one argument"
                            )));
                        }
                        let a = match list_items_of(&args[0]) {
                            Some(v) => py_list(v),
                            None => return Err(PyError::type_error("expected a list")),
                        };
                        let b = match list_items_of(&args[1]) {
                            Some(v) => py_list(v),
                            None => return Err(PyError::type_error("expected a list")),
                        };
                        crate::object::py_compare(&a, &b, $op)
                    },
                }),
            );
        };
    }
    list_richcmp!("__lt__", 0);
    list_richcmp!("__le__", 1);
    list_richcmp!("__ge__", 3);
    list_richcmp!("__gt__", 4);
    // `__doc__` as plain None (consistent with every other native type
    // today): the KEY is what `dir(list)` needs; access behavior is
    // unchanged (the `__doc__`→None fallback already produced None).
    list_dict.insert_str("__doc__", py_none());
    // `list.__hash__ = None` (real CPython's own convention for every
    // mutable builtin: `list`/`dict`/`set`/`bytearray`) — the actual
    // unhashability ENFORCEMENT already happens elsewhere (`hash()`
    // dispatches on the runtime value's own `PyObject` variant, not this
    // dict entry, so this doesn't change `hash([])`'s behavior at all: it
    // already correctly raised `TypeError: unhashable type: 'list'`).
    // What this DOES fix is every INTROSPECTION-based consumer of the
    // `__hash__ is None` idiom real CPython uses to mean "not hashable" —
    // real `Lib/_collections_abc.py`'s own `Hashable.__subclasshook__`
    // (`_check_methods`: "if B.__dict__[method] is None: return
    // NotImplemented") being the concrete trigger
    // (`test_collections.py`'s `TestOneTrickPonyABCs.test_Hashable`
    // asserted `bytearray(b'')` is NOT `isinstance(x, Hashable)` — without
    // a real `None` here, `_check_methods` found `__hash__` inherited from
    // `object` and concluded it MUST be hashable).
    list_dict.insert_str("__hash__", py_none());
    let list_type = PyObjectRef::new(PyObject::Type {
        name: "list".to_string(),
        dict: Box::new(str_map_to_typedict(list_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *list_type.borrow_mut() {
        *mro = vec![list_type.clone(), object_type.clone()];
    }
    builtins.insert_str("list", list_type.clone());
    crate::object::seed_primitive_type_cache("list", list_type);

    // `dict` — same shape as the others above. Unlike `list`/`str`
    // (nothing class-level-only), `dict` DOES have genuine class-level
    // methods previously reached via `bf_name == "dict" && name == "..."`
    // in `get_attribute_impl` (`attrs.rs`) — those become unreachable once
    // `dict` is a real `Type`, so `fromkeys`/`__setitem__`/`__getitem__`
    // move into `dict_dict` here instead.
    let mut dict_dict: HashMap<String, PyObjectRef> = HashMap::new();
    dict_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "dict".to_string(),
            func: builtin_dict,
        }),
    );
    dict_dict.insert_str(
        "fromkeys",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fromkeys".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("fromkeys() takes at least 1 argument"));
                }
                let keys = crate::object::collect_iterable(&args[0])?;
                let value = args.get(1).cloned().unwrap_or_else(py_none);
                let mut d = PyDict::new();
                for k in keys {
                    d.set(k, value.clone())?;
                }
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
            },
        }),
    );
    dict_dict.insert_str(
        "__setitem__",
        PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "__setitem__".to_string(),
            func: crate::object::builtin_dict_setitem as BuiltinFunc,
            self_obj: py_none(),
        }),
    );
    dict_dict.insert_str(
        "__getitem__",
        PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "__getitem__".to_string(),
            func: crate::object::builtin_dict_getitem as BuiltinFunc,
            self_obj: py_none(),
        }),
    );
    dict_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    dict_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_dict_repr as BuiltinFunc,
        }),
    );
    // See `list_dict`'s matching `__hash__ = None` for why.
    dict_dict.insert_str("__hash__", py_none());
    let dict_type = PyObjectRef::new(PyObject::Type {
        name: "dict".to_string(),
        dict: Box::new(str_map_to_typedict(dict_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *dict_type.borrow_mut() {
        *mro = vec![dict_type.clone(), object_type.clone()];
    }
    builtins.insert_str("dict", dict_type.clone());
    crate::object::seed_primitive_type_cache("dict", dict_type);

    // `tuple` — same shape as `list`/`str` (no class-level-only method).
    // Unlike those two, `tuple` was NOT previously in
    // `is_recognized_native_base_name`/`make_native_backing`/
    // `synthesize_native_init` at all — `class MyTuple(tuple): ...` was
    // silently broken before this migration added it there too (same class
    // of gap found and fixed for `float`; see the memory entry for this
    // migration effort).
    let mut tuple_dict: HashMap<String, PyObjectRef> = HashMap::new();
    tuple_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "tuple".to_string(),
            func: builtin_tuple,
        }),
    );
    tuple_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    tuple_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_tuple_repr as BuiltinFunc,
        }),
    );
    let tuple_type = PyObjectRef::new(PyObject::Type {
        name: "tuple".to_string(),
        dict: Box::new(str_map_to_typedict(tuple_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *tuple_type.borrow_mut() {
        *mro = vec![tuple_type.clone(), object_type.clone()];
    }
    builtins.insert_str("tuple", tuple_type.clone());
    crate::object::seed_primitive_type_cache("tuple", tuple_type);

    // `bytes` — same shape as `list`/`str`/`tuple` (no dunder-shaped
    // class-level method — `fromhex` is a plain classmethod-style
    // function, not a dunder, so it carries none of the ancestor-mro
    // hazards documented for `dict`'s migration).
    let mut bytes_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bytes_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bytes".to_string(),
            func: builtin_bytes,
        }),
    );
    bytes_dict.insert_str(
        "fromhex",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fromhex".to_string(),
            func: builtin_bytes_fromhex,
        }),
    );
    bytes_dict.insert_str(
        "maketrans",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "maketrans".to_string(),
            func: crate::object::bytes_maketrans_builtin,
        }),
    );
    bytes_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    bytes_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bytes_repr as BuiltinFunc,
        }),
    );
    let bytes_type = PyObjectRef::new(PyObject::Type {
        name: "bytes".to_string(),
        dict: Box::new(str_map_to_typedict(bytes_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bytes_type.borrow_mut() {
        *mro = vec![bytes_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bytes", bytes_type.clone());
    crate::object::seed_primitive_type_cache("bytes", bytes_type);

    // `set` — same shape, no class-level-only method at all.
    let mut set_dict: HashMap<String, PyObjectRef> = HashMap::new();
    set_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "set".to_string(),
            func: builtin_set,
        }),
    );
    set_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    set_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_set_repr as BuiltinFunc,
        }),
    );
    // See `list_dict`'s matching `__hash__ = None` for why.
    set_dict.insert_str("__hash__", py_none());
    let set_type = PyObjectRef::new(PyObject::Type {
        name: "set".to_string(),
        dict: Box::new(str_map_to_typedict(set_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *set_type.borrow_mut() {
        *mro = vec![set_type.clone(), object_type.clone()];
    }
    builtins.insert_str("set", set_type.clone());
    crate::object::seed_primitive_type_cache("set", set_type);

    // `complex` — same shape; `from_number` is a plain classmethod-style
    // function (not a dunder).
    let mut complex_dict: HashMap<String, PyObjectRef> = HashMap::new();
    complex_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "complex".to_string(),
            func: builtin_complex,
        }),
    );
    // `from_number` as a CLASSMETHOD (bound: args[0] is the calling type, so
    // `ComplexSubclass.from_number(3.14)` returns a ComplexSubclass instance
    // — test_from_number_subclass asserts `type(...) is cls`).
    fn complex_from_number_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "complex.from_number() takes exactly 1 argument",
            ));
        }
        let cls = args[0].clone();
        let value = &args[1];
        // Reject strings/bytes — from_number doesn't parse strings (unlike
        // complex(str)); test_from_number: from_number('3.14') must TypeError.
        {
            let b = value.borrow();
            if matches!(&*b, PyObject::Str(_) | PyObject::Bytes(_)) {
                return Err(PyError::type_error(
                    "complex() argument must be a string or a number",
                ));
            }
        }
        // Convert via complex() (handles complex pass-through, subclass
        // backing, __complex__/__float__/__index__).
        let cx = builtin_complex(&[value.clone()])?;
        // The base `complex` type → return the native value; a SUBCLASS →
        // wrap in an Instance of that class (native backing = cx).
        let is_base = match &*cls.borrow() {
            PyObject::Type { name, .. } => name == "complex",
            _ => false,
        };
        if is_base {
            return Ok(cx);
        }
        Ok(crate::object::make_subclass_instance(&cls, cx))
    }
    let from_number_fn = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "from_number".to_string(),
        func: complex_from_number_impl,
    });
    complex_dict.insert_str(
        "from_number",
        PyObjectRef::new(PyObject::ClassMethod { func: from_number_fn }),
    );
    complex_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    complex_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_complex_repr as BuiltinFunc,
        }),
    );
    // Rich comparisons. Complex supports ==/!= against numbers but is NOT
    // orderable: ordering dunders always return NotImplemented (the operator
    // dispatch turns two NotImplementeds into TypeError; test_richcompare
    // asserts the dunders THEMSELVES return NotImplemented).
    {
        use crate::object::{py_compare, py_div, py_not_implemented};
        // True iff `o` is a numeric value complex can compare for equality.
        fn is_num(o: &PyObjectRef) -> bool {
            matches!(
                &*o.borrow(),
                PyObject::Complex(..) | PyObject::Int(_) | PyObject::Float(_)
            ) || matches!(
                o,
                PyObjectRef::SmallInt(_)
                    | PyObjectRef::SmallFloat(_)
                    | PyObjectRef::SmallBool(_)
            )
        }
        complex_dict.insert_str(
            "__truediv__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__truediv__".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error(
                            "__truediv__() takes at least 2 arguments",
                        ));
                    }
                    py_div(&args[0], &args[1])
                },
            }),
        );

    // Complex parts for comparison: as_complex_parts, but a huge int (that
    // can't convert to f64) becomes ±inf so `complex == 1<<10000` is False
    // (not NotImplemented).
    fn cx_parts_for_cmp(o: &PyObjectRef) -> Option<(f64, f64)> {
        let borrowed = o.borrow();
        // Check Int FIRST (with exact-representability round-trip) —
        // `as_complex_parts` would round a huge int to f64, losing the
        // precision distinction test_richcompare_boundaries needs.
        if let PyObject::Int(n) = &*borrowed {
            return match n.to_f64() {
                Some(f) => {
                    let exact = format!("{:.0}", f)
                        .parse::<BigInt>()
                        .map(|b| b == *n)
                        .unwrap_or(false);
                    if exact {
                        Some((f, 0.0))
                    } else {
                        // Not exactly representable → sentinel that never
                        // equals a finite complex real (so == is False).
                        Some((f64::NAN, 0.0))
                    }
                }
                None => Some((f64::INFINITY, 0.0)),
            };
        }
        crate::object::as_complex_parts(&borrowed)
    }

        complex_dict.insert_str(
            "__eq__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__eq__".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("__eq__() takes at least 2 arguments"));
                    }
                    if !is_num(&args[1]) {
                        return Ok(py_not_implemented());
                    }
                    // Compare VALUES directly — NOT via py_compare, which for
                    // an Instance self (complex subclass) re-dispatches to this
                    // very __eq__ → infinite recursion (c2 == c crash).
                    match (cx_parts_for_cmp(&args[0]), cx_parts_for_cmp(&args[1])) {
                        (Some((ar, ai)), Some((br, bi))) => Ok(py_bool(ar == br && ai == bi)),
                        _ => Ok(py_not_implemented()),
                    }
                },
            }),
        );
        complex_dict.insert_str(
            "__ne__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__ne__".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("__ne__() takes at least 2 arguments"));
                    }
                    if !is_num(&args[1]) {
                        return Ok(py_not_implemented());
                    }
                    match (cx_parts_for_cmp(&args[0]), cx_parts_for_cmp(&args[1])) {
                        (Some((ar, ai)), Some((br, bi))) => Ok(py_bool(ar != br || ai != bi)),
                        _ => Ok(py_not_implemented()),
                    }
                },
            }),
        );
        // Ordering: always NotImplemented (complex is not orderable).
        complex_dict.insert_str(
            "__lt__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__lt__".to_string(),
                func: |_args| Ok(py_not_implemented()),
            }),
        );
        complex_dict.insert_str(
            "__le__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__le__".to_string(),
                func: |_args| Ok(py_not_implemented()),
            }),
        );
        complex_dict.insert_str(
            "__gt__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__gt__".to_string(),
                func: |_args| Ok(py_not_implemented()),
            }),
        );
        complex_dict.insert_str(
            "__ge__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__ge__".to_string(),
                func: |_args| Ok(py_not_implemented()),
            }),
        );
        complex_dict.insert_str(
            "__format__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__format__".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error(
                            "__format__ requires at least 1 argument",
                        ));
                    }
                    let spec = if args.len() > 1 {
                        args[1].str()
                    } else {
                        String::new()
                    };
                    let result = crate::vm::format_with_spec(&args[0], &spec)?;
                    Ok(py_str(&result))
                },
            }),
        );
    }
    let complex_type = PyObjectRef::new(PyObject::Type {
        name: "complex".to_string(),
        dict: Box::new(str_map_to_typedict(complex_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *complex_type.borrow_mut() {
        *mro = vec![complex_type.clone(), object_type.clone()];
    }
    builtins.insert_str("complex", complex_type.clone());
    crate::object::seed_primitive_type_cache("complex", complex_type);

    // `bytearray` — same shape, no class-level-only method.
    let mut bytearray_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bytearray_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bytearray".to_string(),
            func: builtin_bytearray,
        }),
    );
    bytearray_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    bytearray_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bytearray_repr as BuiltinFunc,
        }),
    );
    // See `list_dict`'s matching `__hash__ = None` for why.
    bytearray_dict.insert_str("__hash__", py_none());
    let bytearray_type = PyObjectRef::new(PyObject::Type {
        name: "bytearray".to_string(),
        dict: Box::new(str_map_to_typedict(bytearray_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bytearray_type.borrow_mut() {
        *mro = vec![bytearray_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bytearray", bytearray_type.clone());
    crate::object::seed_primitive_type_cache("bytearray", bytearray_type);

    // `frozenset` — same shape, no class-level-only method.
    let mut frozenset_dict: HashMap<String, PyObjectRef> = HashMap::new();
    frozenset_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "frozenset".to_string(),
            func: builtin_frozenset,
        }),
    );
    frozenset_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    frozenset_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_frozenset_repr as BuiltinFunc,
        }),
    );
    let frozenset_type = PyObjectRef::new(PyObject::Type {
        name: "frozenset".to_string(),
        dict: Box::new(str_map_to_typedict(frozenset_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *frozenset_type.borrow_mut() {
        *mro = vec![frozenset_type.clone(), object_type.clone()];
    }
    builtins.insert_str("frozenset", frozenset_type.clone());
    crate::object::seed_primitive_type_cache("frozenset", frozenset_type);

    // `NoneType` (`type(None)`) — NOT inserted into `builtins` (real
    // CPython doesn't expose a bare `NoneType` name either; it's reachable
    // as `types.NoneType`), just pre-seeded into the same
    // `PRIMITIVE_TYPE_CACHE` the OTHER types above register into, so
    // `type(None)` returns this real, `object`-derived Type instead of
    // falling through to `builtin_type_of`'s generic "empty placeholder
    // Type built by name" catch-all (which gives an empty `dict`/`mro` —
    // exposing NO dunders at all, `__hash__` included). Real CPython:
    // `None` IS hashable (`NoneType.__hash__` is a real, constant-
    // returning function) — real trigger: `test_collections.py`'s
    // `TestOneTrickPonyABCs.test_Hashable` asserts
    // `issubclass(type(None), Hashable)`, which needs `_check_methods` to
    // find a real (non-`None`, non-missing) `__hash__` somewhere in
    // `type(None).__mro__`.
    let mut none_type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    none_type_dict.insert_str(
        "__hash__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__hash__".to_string(),
            func: |args: &[PyObjectRef]| {
                let v = args
                    .first()
                    .ok_or_else(|| PyError::type_error("__hash__() missing self"))?;
                Ok(py_int(v.hash()? as i64))
            },
        }),
    );
    let none_type = PyObjectRef::new(PyObject::Type {
        name: "NoneType".to_string(),
        dict: Box::new(str_map_to_typedict(none_type_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *none_type.borrow_mut() {
        *mro = vec![none_type.clone(), object_type.clone()];
    }
    crate::object::seed_primitive_type_cache("NoneType", none_type);
}
