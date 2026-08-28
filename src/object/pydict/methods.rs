// Split from src/object/pydict.rs — dict method helpers (instance, resolve, static methods, builtin setitem/getitem).
use super::*;
use crate::object::*;
use std::rc::Rc;

/// Helper: provide dict methods (items, keys, values, __iter__) for Instance objects
/// that inherit from dict but can't access the built-in dict methods.
pub(crate) fn instance_builtin_dict_method(
    name: &str,
    dict_snapshot: Vec<(String, PyObjectRef)>,
) -> Option<PyObjectRef> {
    let method_name = name.to_string();
    Some(PyObjectRef::new(PyObject::Closure(Rc::new(
        move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            match method_name.as_str() {
                "__iter__" => {
                    let keys: Vec<PyObjectRef> =
                        dict_snapshot.iter().map(|(k, _)| py_str(k)).collect();
                    Ok(PyObjectRef::new(PyObject::List(keys)))
                }
                "items" => {
                    let items: Vec<PyObjectRef> = dict_snapshot
                        .iter()
                        .map(|(k, v)| py_tuple(vec![py_str(k), v.clone()]))
                        .collect();
                    Ok(PyObjectRef::new(PyObject::List(items)))
                }
                "keys" => {
                    let keys: Vec<PyObjectRef> =
                        dict_snapshot.iter().map(|(k, _)| py_str(k)).collect();
                    Ok(PyObjectRef::new(PyObject::List(keys)))
                }
                "values" => {
                    let values: Vec<PyObjectRef> =
                        dict_snapshot.iter().map(|(_, v)| v.clone()).collect();
                    Ok(PyObjectRef::new(PyObject::List(values)))
                }
                _ => Err(PyError::type_error(format!(
                    "unsupported dict method: {}",
                    method_name
                ))),
            }
        },
    ))))
}

/// Resolves the REAL underlying `PyDict` data behind an "unbound-style" dict
/// method call (`dict.keys(some_dict_or_subclass_instance)`, the common
/// idiom a `dict` subclass uses to call the parent's real implementation
/// while overriding the same-named method itself — real trigger: CPython's
/// own `test_dict.py::test_dict_copy_order`'s `CustomReversedDict`).
///
/// These `dict_method_*` functions used to check `PyObject::Instance {
/// dict, .. }` and read the instance's OWN generic attribute `AttrMap` —
/// which is the WRONG storage entirely: a dict subclass instance's actual
/// key/value data lives in a native backing (a real `PyObject::Dict` under
/// `NATIVE_BACKING_KEY`), not its `__dict__`-equivalent attribute map (which
/// is empty unless the subclass itself sets extra attributes). This was
/// broken for BOTH forms of the call — a plain `dict` (not wrapped in any
/// `Instance` at all) AND a genuine subclass instance — confirmed via
/// direct repro (`dict.keys({'a': 1})` and `dict.keys(CustomDict(a=1))`
/// both raised `TypeError: keys() requires a dict-like instance`).
fn resolve_dict_like(obj: &PyObjectRef) -> Option<PyObjectRef> {
    if matches!(&*obj.borrow(), PyObject::Dict(_)) {
        return Some(obj.clone());
    }
    crate::object::native_backing_of(obj)
        .filter(|native| matches!(&*native.borrow(), PyObject::Dict(_)))
}

/// Static dict method: get
pub fn dict_method_get(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("get() requires at least 1 argument"));
    }
    match resolve_dict_like(&args[0]) {
        Some(d) => {
            if let PyObject::Dict(pd) = &*d.borrow() {
                Ok(pd.get(&args[1])?.unwrap_or_else(|| {
                    if args.len() > 2 {
                        args[2].clone()
                    } else {
                        py_none()
                    }
                }))
            } else {
                unreachable!()
            }
        }
        None => Err(PyError::type_error("get() requires a dict-like instance")),
    }
}

/// Static dict method: __iter__
pub fn dict_method_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__iter__ requires self"));
    }
    match resolve_dict_like(&args[0]) {
        Some(d) => {
            let keys = if let PyObject::Dict(pd) = &*d.borrow() {
                pd.keys()
            } else {
                unreachable!()
            };
            Ok(PyObjectRef::new(PyObject::ListIter {
                list: keys,
                index: 0,
            }))
        }
        None => Err(PyError::type_error(
            "__iter__ requires a dict-like instance",
        )),
    }
}

/// Static dict method: items
pub fn dict_method_items(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("items() requires self"));
    }
    // Handle native dicts directly
    if let Some(d) = resolve_dict_like(&args[0]) {
        if let PyObject::Dict(pd) = &*d.borrow() {
            let items = pd.items();
            return Ok(py_list(
                items
                    .into_iter()
                    .map(|(k, v)| py_tuple(vec![k, v]))
                    .collect(),
            ));
        }
    }
    // For Mapping-like objects (ThemeSection etc.), use __getitem__ + __iter__
    let obj = &args[0];
    let iter_fn = {
        let obj_borrowed = obj.borrow();
        obj_borrowed.get_attribute("__iter__")?
    };
    let iter_obj = crate::object::call_bound_method(iter_fn, obj.clone(), vec![])?;
    let mut keys_list = Vec::new();
    loop {
        let key = match crate::object::builtin_next(&[iter_obj.clone()]) {
            Ok(v) => v,
            Err(e) if crate::object::is_stop_iteration_error(&e) => break,
            Err(e) => return Err(e),
        };
        keys_list.push(key);
    }
    let getitem_fn = {
        let obj_borrowed = obj.borrow();
        obj_borrowed.get_attribute("__getitem__")?
    };
    let mut result = Vec::new();
    for k in keys_list {
        let v = crate::object::call_bound_method(getitem_fn.clone(), obj.clone(), vec![k.clone()])?;
        result.push(py_tuple(vec![k, v]));
    }
    Ok(py_list(result))
}

/// Static dict method: keys
pub fn dict_method_keys(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("keys() requires self"));
    }
    match resolve_dict_like(&args[0]) {
        Some(d) => {
            let keys = if let PyObject::Dict(pd) = &*d.borrow() {
                pd.keys()
            } else {
                unreachable!()
            };
            Ok(py_list(keys))
        }
        None => Err(PyError::type_error("keys() requires a dict-like instance")),
    }
}

/// Static dict method: values
pub fn dict_method_values(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("values() requires self"));
    }
    match resolve_dict_like(&args[0]) {
        Some(d) => {
            let values = if let PyObject::Dict(pd) = &*d.borrow() {
                pd.values()
            } else {
                unreachable!()
            };
            Ok(py_list(values))
        }
        None => Err(PyError::type_error(
            "values() requires a dict-like instance",
        )),
    }
}

/// dict.__setitem__ function: allows dict.__setitem__(instance, key, value)
pub fn builtin_dict_setitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Handle both calling conventions:
    // - Direct: [instance, key, value] (3 args)
    // - Via BuiltinMethod: [py_none(), instance, key, value] (4 args)
    let instance = if args.len() >= 4 {
        &args[1]
    } else if args.len() >= 3 {
        &args[0]
    } else {
        return Err(PyError::type_error(
            "dict.__setitem__() requires at least 2 arguments",
        ));
    };
    let key = if args.len() >= 4 {
        args[2].str()
    } else if args.len() >= 3 {
        args[1].str()
    } else {
        return Err(PyError::type_error(
            "dict.__setitem__() requires at least 2 arguments",
        ));
    };
    let value = if args.len() >= 4 {
        args[3].clone()
    } else if args.len() >= 3 {
        args[2].clone()
    } else {
        return Err(PyError::type_error(
            "dict.__setitem__() requires at least 2 arguments",
        ));
    };
    // A real dict subclass instance (e.g. `class _EnumDict(dict): ...`,
    // used to give enum.EnumType.__prepare__'s namespace object a place to
    // track member-definition order) has its actual dict *contents* in its
    // native backing, not its own attribute storage — `dict.__setitem__`
    // must write there so a later `classdict[key]` subscript read (which
    // goes through the native backing via py_getitem) actually sees it.
    // Only fall back to treating the instance's own attribute dict as "the
    // dict" when there's no native backing at all.
    if let Some(native) = native_backing_of(instance) {
        py_setitem(&native, &py_str(&key), value)?;
        return Ok(py_none());
    }
    let mut obj = instance.borrow_mut();
    if let PyObject::Instance { dict, .. } = &mut *obj {
        dict.insert(key, value);
    } else if let PyObject::Dict(pd) = &mut *obj {
        pd.set(py_str(&key), value).ok();
    } else {
        drop(obj);
        // Fall back to py_setitem for non-Instance types
        py_setitem(
            instance,
            &args[if args.len() >= 4 { 2 } else { 1 }],
            args[if args.len() >= 4 { 3 } else { 2 }].clone(),
        )?;
    }
    Ok(py_none())
}

/// dict.__getitem__ function: allows dict.__getitem__(instance, key)
pub fn builtin_dict_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Handle both calling conventions:
    // - Direct: [instance, key] (2 args)
    // - Via BuiltinMethod: [py_none(), instance, key] (3 args)
    let instance = if args.len() >= 3 {
        &args[1]
    } else if args.len() >= 2 {
        &args[0]
    } else {
        return Err(PyError::type_error(
            "dict.__getitem__() requires at least 1 argument",
        ));
    };
    let key_ref = if args.len() >= 3 {
        &args[2]
    } else if args.len() >= 2 {
        &args[1]
    } else {
        return Err(PyError::type_error(
            "dict.__getitem__() requires at least 1 argument",
        ));
    };
    let key = key_ref.str();
    // Check for __missing__ first (dict subclass support, e.g. Counter).
    // `get_attribute` already returns a properly SELF-BOUND method for an
    // ordinary Python-defined `__missing__` (a `BoundMethod`/equivalent
    // with `instance` baked in) — passing `instance.clone()` again here on
    // top of that used to double up `self`, e.g. `Counter.__missing__(self,
    // key)` receiving `(instance, instance, key)` and raising a `TypeError`
    // that this whole call then silently swallowed via `.ok()`, making the
    // `__missing__` lookup appear to fail even when it was genuinely
    // defined. Only `key_ref` needs to be passed explicitly.
    let missing_result = instance
        .borrow()
        .get_attribute("__missing__")
        .ok()
        .and_then(|missing| crate::object::call_function(&missing, vec![key_ref.clone()]).ok());
    if let Some(val) = missing_result {
        return Ok(val);
    }
    // See builtin_dict_setitem's matching comment: a real dict subclass's
    // actual contents live in its native backing, not its attribute dict.
    if let Some(native) = native_backing_of(instance) {
        return py_getitem(&native, key_ref);
    }
    // Directly read from the Instance's dict, bypassing py_getitem (which would recurse)
    let obj = instance.borrow();
    if let PyObject::Instance { dict, .. } = &*obj {
        let val = dict
            .get(&key)
            .cloned()
            .ok_or_else(|| PyError::key_error(format!("'{}'", key)))?;
        drop(obj);
        Ok(val)
    } else if let PyObject::Dict(pd) = &*obj {
        let val = pd.get(key_ref)?.unwrap_or_else(py_none);
        drop(obj);
        Ok(val)
    } else {
        drop(obj);
        // Fall back to py_getitem for non-Instance/Dict types
        py_getitem(instance, key_ref)
    }
}
