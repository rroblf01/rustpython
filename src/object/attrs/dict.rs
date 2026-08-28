// Auto-extracted from src/object/attrs/mod.rs lines 2810-3398
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Dict(_d) => {
                match name {
                    "keys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "keys".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_keys",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*d {
                                let keys: Vec<PyObjectRef> = g
                                    .borrow()
                                    .keys()
                                    .map(|k| py_str(crate::interner::lookup_str(*k)))
                                    .collect();
                                Ok(py_list(keys))
                            } else {
                                Err(PyError::runtime_error("keys on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "values" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "values".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_values",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*d {
                                let values: Vec<PyObjectRef> =
                                    g.borrow().values().cloned().collect();
                                Ok(py_list(values))
                            } else {
                                Err(PyError::runtime_error("values on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "items" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "items".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(_dict) = &*d {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_items",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*d {
                                let items: Vec<PyObjectRef> = g
                                    .borrow()
                                    .iter()
                                    .map(|(k, v)| {
                                        py_tuple(vec![
                                            py_str(crate::interner::lookup_str(*k)),
                                            v.clone(),
                                        ])
                                    })
                                    .collect();
                                Ok(py_list(items))
                            } else {
                                Err(PyError::runtime_error("items on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("get() takes at least 1 argument"));
                            }
                            let dict = &*args[0].borrow();
                            if let PyObject::Dict(d) = dict {
                                Ok(d.get(&args[1])?.unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else if let PyObject::Globals(g) = dict {
                                let key = match &*args[1].borrow() {
                                    PyObject::Str(s) => crate::interner::intern(s.as_str()),
                                    _ => return Ok(py_none()),
                                };
                                Ok(g.borrow().get(&key).cloned().unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else {
                                Err(PyError::runtime_error("get on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("pop() takes at least 1 argument"));
                            }
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                match d.remove(&args[1]) {
                                    Ok(val) => Ok(val),
                                    Err(_) if args.len() > 2 => Ok(args[2].clone()),
                                    Err(e) => Err(e),
                                }
                            } else {
                                Err(PyError::runtime_error("pop on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "popitem" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "popitem".to_string(),
                        func: |args| {
                            // Real `dict.popitem()` takes NO arguments at
                            // all (unlike `OrderedDict.popitem(last=True)`,
                            // a genuinely different method on a different
                            // type) — this silently accepted and ignored
                            // any extra positional argument instead of
                            // raising, confirmed via CPython's own
                            // `test_dict.py`/`mapping_tests.py::
                            // test_popitem`, which explicitly checks
                            // `assertRaises(TypeError, d.popitem, 42)`.
                            if args.len() > 1 {
                                return Err(PyError::type_error(format!(
                                    "dict.popitem() takes no arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                let items = d.items();
                                if items.is_empty() {
                                    return Err(PyError::key_error(
                                        "popitem(): dictionary is empty",
                                    ));
                                }
                                let (k, v) = items.into_iter().last().unwrap();
                                d.remove(&k)?;
                                Ok(py_tuple(vec![k, v]))
                            } else {
                                Err(PyError::runtime_error("popitem on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                d.clear();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("clear on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "update".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "update() takes at least 1 argument",
                                ));
                            }
                            let self_obj = args[0].clone();
                            // Matches CPython's real dict.update(): accepts another
                            // dict, any mapping-protocol object (has .keys()), or an
                            // iterable of (key, value) pairs. A trailing kwargs dict
                            // (from `d.update(x, k=v)`) is just another entry here.
                            for other in &args[1..] {
                                let is_dict = matches!(&*other.borrow(), PyObject::Dict(_));
                                if is_dict {
                                    let items = if let PyObject::Dict(other_dict) = &*other.borrow()
                                    {
                                        other_dict.items()
                                    } else {
                                        unreachable!()
                                    };
                                    if let PyObject::Dict(d) = &mut *self_obj.borrow_mut() {
                                        for (k, v) in items {
                                            d.set(k, v)?;
                                        }
                                    }
                                    continue;
                                }
                                // A native-backed dict subclass (Counter, defaultdict,
                                // or any `class Foo(dict): ...`) — read straight off
                                // the native backing rather than resolving `keys`.
                                if let Some(native) = native_backing_of(other) {
                                    if let PyObject::Dict(other_dict) = &*native.borrow() {
                                        let items = other_dict.items();
                                        if let PyObject::Dict(d) = &mut *self_obj.borrow_mut() {
                                            for (k, v) in items {
                                                d.set(k, v)?;
                                            }
                                        }
                                        continue;
                                    }
                                }
                                let keys_fn = match &*other.borrow() {
                                    PyObject::Instance { typ, .. } => {
                                        lookup_dunder_via_mro(typ, "keys")
                                    }
                                    _ => None,
                                };
                                if let Some(keys_fn) = keys_fn {
                                    let keys_obj =
                                        call_bound_method(keys_fn, other.clone(), vec![])?;
                                    let it = builtin_iter(&[keys_obj])?;
                                    loop {
                                        match builtin_next(&[it.clone()]) {
                                            Ok(k) => {
                                                let v = py_getitem(other, &k)?;
                                                if let PyObject::Dict(d) =
                                                    &mut *self_obj.borrow_mut()
                                                {
                                                    d.set(k, v)?;
                                                }
                                            }
                                            Err(PyError::StopIteration) => break,
                                            Err(e) => return Err(e),
                                        }
                                    }
                                } else {
                                    let it = builtin_iter(&[other.clone()])?;
                                    loop {
                                        match builtin_next(&[it.clone()]) {
                                            Ok(pair) => {
                                                let (k, v) = match &*pair.borrow() {
                                                    PyObject::Tuple(items) | PyObject::List(items) if items.len() == 2 => {
                                                        (items[0].clone(), items[1].clone())
                                                    }
                                                    _ => return Err(PyError::type_error("cannot convert update sequence element to a sequence")),
                                                };
                                                if let PyObject::Dict(d) =
                                                    &mut *self_obj.borrow_mut()
                                                {
                                                    d.set(k, v)?;
                                                }
                                            }
                                            Err(PyError::StopIteration) => break,
                                            Err(e) => return Err(e),
                                        }
                                    }
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "setdefault" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setdefault".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "setdefault() takes at least 1 argument",
                                ));
                            }
                            let key = args[1].clone();
                            let default = if args.len() > 2 {
                                args[2].clone()
                            } else {
                                py_none()
                            };
                            // Routed through `pydict_safe_get_or_insert` — see
                            // `pydict_safe_set`'s doc comment (subscript.rs)
                            // for why this must never hold `args[0]`'s own
                            // mutable borrow across a colliding key's
                            // `.equals()` call (real CPython test:
                            // `test_dict.py`'s `test_clear_at_lookup`, which
                            // exercises this exact method).
                            //
                            // Globals receivers are Imm and wrap their own
                            // RefCell'd map — the safe-or-insert helper
                            // requires a Mut Dict wrapper and would panic.
                            if matches!(&*args[0].borrow(), PyObject::Globals(_)) {
                                let sid = crate::interner::intern(&key.str());
                                let g = match &*args[0].borrow() {
                                    PyObject::Globals(g) => g.clone(),
                                    _ => unreachable!(),
                                };
                                if let Some(existing) = g.borrow().get(&sid) {
                                    return Ok(existing.clone());
                                }
                                g.borrow_mut().insert(sid, default.clone());
                                return Ok(default);
                            }
                            pydict_safe_get_or_insert(&args[0], key, default)
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                let mut new_dict = PyDict::new();
                                for (k, v) in dict.items() {
                                    new_dict.set(k, v)?;
                                }
                                Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
                            } else {
                                Err(PyError::runtime_error("copy on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "fromkeys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "fromkeys".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "fromkeys() takes at least 1 argument",
                                ));
                            }
                            let mut new_dict = PyDict::new();
                            let val = if args.len() > 2 {
                                args[2].clone()
                            } else {
                                py_none()
                            };
                            let it = builtin_iter(&[args[1].clone()])?;
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(k) => {
                                        new_dict.set(k, val.clone())?;
                                    }
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(py_int(72 + (d.len() as i64) * 16))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-dict"))
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
                            if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(py_bool(d.contains(&args[1])?))
                            } else {
                                Err(PyError::runtime_error("__contains__ on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `some_dict.__getitem__`/`__setitem__`/`__delitem__` as
                    // a bound-method REFERENCE (not called directly) — real
                    // code uses this idiom to grab a fast lookup callable
                    // (real trigger: CPython 3.14's own `_colorize.py`,
                    // `super().__setattr__('_name_to_value',
                    // name_to_value.__getitem__)`), same class of gap as
                    // `frozenset.__contains__` found earlier this session.
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
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                d.remove(&args[1])?;
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("__delitem__ on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "move_to_end" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "move_to_end".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "move_to_end() needs a key argument",
                                ));
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__or__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__or__".to_string(),
                        func: |args| {
                            // Reachable two ways with two different argument
                            // shapes: a normal bound call (`d.__or__(x)`,
                            // rebound to `[self, other]` by the usual
                            // attribute-access path) and `py_bit_or`'s
                            // `try_dunder_binop` (`{} | d2`), which — like
                            // every other native dunder called that way —
                            // goes through `call_bound_method`'s
                            // placeholder-prepending `BuiltinMethod` arm,
                            // delivering `[None, self, other]` instead. This
                            // used to only handle the 2-arg shape, so `dict |
                            // dict` (real PEP 584 syntax) misread the
                            // placeholder as `self`, failing with a
                            // nonsensical "non-dict" error despite both
                            // operands being genuine dicts.
                            let (self_idx, other_idx) = if args.len() >= 3 {
                                (1, 2)
                            } else if args.len() == 2 {
                                (0, 1)
                            } else {
                                return Err(PyError::type_error(
                                    "__or__() takes exactly one argument",
                                ));
                            };
                            // Accept dict-subclass instances on either side
                            // (defaultdict etc.) by falling back to their
                            // keys()/__getitem__ mapping protocol.
                            fn dict_like_items(o: &PyObjectRef) -> Option<Vec<(PyObjectRef, PyObjectRef)>> {
                                let b = o.borrow();
                                if let PyObject::Dict(dd) = &*b {
                                    return Some(dd.items());
                                }
                                if b.get_attribute("keys").is_ok() {
                                    return None; // handled by caller via update-style path
                                }
                                None
                            }
                            // Reflected priority: a dict-subclass Instance
                            // (defaultdict etc.) with __ror__ must win over
                            // this native implementation.
                            if let PyObject::Instance { typ, .. } =
                                &*args[other_idx].borrow()
                            {
                                if crate::object::lookup_dunder_via_mro(typ, "__ror__")
                                    .is_some()
                                {
                                    let ror = crate::object::lookup_dunder_via_mro(
                                        typ,
                                        "__ror__",
                                    )
                                    .unwrap();
                                    return crate::object::call_bound_method(
                                        ror,
                                        args[other_idx].clone(),
                                        vec![args[self_idx].clone()],
                                    );
                                }
                            }
                            // Accept list/tuple of (k, v) pairs too:
                            // `defaultdict |= [(1,'a'), ...]` is real CPython
                            // dict.__ior__ semantics (in-place update).
                            let other_is_pairs = matches!(
                                &*args[other_idx].borrow(),
                                PyObject::List(_) | PyObject::Tuple(_)
                            );
                            let other_ok = matches!(&*args[other_idx].borrow(), PyObject::Dict(_))
                                || args[other_idx].borrow().get_attribute("keys").is_ok()
                                || other_is_pairs;
                            if !other_ok {
                                return Err(PyError::type_error(
                                    "__or__() argument must be a dict",
                                ));
                            }
                            // Build result: start from self's mapping view.
                            let self_obj_clone = args[self_idx].clone();
                            let mut new_dict = PyDict::new();
                            if let PyObject::Dict(dict) = &*self_obj_clone.borrow() {
                                for (k, v) in dict.items() {
                                    new_dict.set(k.clone(), v.clone())?;
                                }
                            } else {
                                // dict subclass instance: read via its own
                                // mapping protocol (keys + getitem).
                                let keys_m = self_obj_clone.borrow().get_attribute("keys")?;
                                let keys_it = crate::object::builtin_iter(&[crate::object::call_bound_method(keys_m, self_obj_clone.clone(), vec![])?])?;
                                loop {
                                    match crate::object::builtin_next(&[keys_it.clone()]) {
                                        Ok(k) => {
                                            let v = crate::object::py_getitem(&self_obj_clone, &k)?;
                                            new_dict.set(k, v)?;
                                        }
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                            }
                            // Then merge other: prefer its items for dup keys.
                            if let PyObject::Dict(other_dict) = &*args[other_idx].borrow() {
                                for (k, v) in other_dict.items() {
                                    new_dict.set(k.clone(), v.clone())?;
                                }
                            } else {
                                let keys_m = args[other_idx].borrow().get_attribute("keys")?;
                                let keys_it = crate::object::builtin_iter(&[crate::object::call_bound_method(keys_m, args[other_idx].clone(), vec![])?])?;
                                loop {
                                    match crate::object::builtin_next(&[keys_it.clone()]) {
                                        Ok(k) => {
                                            let v = crate::object::py_getitem(&args[other_idx], &k)?;
                                            new_dict.set(k, v)?;
                                        }
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                            }
                            let _ = dict_like_items; // (kept for reference)
                            Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => {
                        let len = _d.len() as i64;
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_int(len))
                            },
                        ))))
                    }
                    "__iter__" => {
                        let keys: Vec<PyObjectRef> = _d.keys();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_list(keys.clone()))
                            },
                        ))))
                    }
                    "__contains__" => {
                        let d_clone = _d.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                if args.is_empty() {
                                    return Err(PyError::type_error("__contains__() takes exactly 1 argument"));
                                }
                                Ok(py_bool(d_clone.contains(&args[0])?))
                            },
                        ))))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'dict' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
