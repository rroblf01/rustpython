use crate::object::*;
use std::collections::HashMap;

pub fn create_copy_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! copy_func {
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

    copy_func!("copy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("copy() missing required argument"));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::None => Ok(py_none()),
            PyObject::Bool(b) => Ok(py_bool(*b)),
            PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) | PyObject::Bytes(_) => {
                Ok(obj.clone())
            }
            PyObject::Tuple(items) => {
                // `copy.copy(tuple)` returns the SAME tuple (test_copy:
                // `self.assertIs(copy.copy(x), x)`).
                Ok(obj.clone())
            }
            PyObject::List(items) => {
                let new_items: Vec<PyObjectRef> = items
                    .iter()
                    .map(|i| {
                        // Shallow copy: clone references
                        let b = i.borrow();
                        match &*b {
                            PyObject::None => py_none(),
                            PyObject::Bool(b) => py_bool(*b),
                            PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) => i.clone(),
                            _ => i.clone(),
                        }
                    })
                    .collect();
                Ok(py_list(new_items))
            }
            PyObject::Dict(dict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.items() {
                    let _ = new_dict.set(k, v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
            }
            PyObject::Set(s) => {
                let mut new_set = PySet::new();
                for item in s.to_vec() {
                    let _ = new_set.add(item);
                }
                Ok(PyObjectRef::new(PyObject::Set(new_set)))
            }
            PyObject::Deque { data, maxlen } => Ok(py_deque(data.clone(), *maxlen)),
            // A class transparently subclassing a native container
            // (`class NodeList(list): pass`, real CPython's own
            // `xml.dom.minicompat.NodeList`) with no explicit `__copy__`
            // fell straight to the generic `Ok(obj.clone())` below — an
            // `Rc` clone, the SAME object, not a real copy at all.
            // Confirmed via `test_xml_dom_minicompat.py`'s own `test_
            // nodelist_copy`/`test_nodelist_deepcopy` (`assertIsNot`/
            // `unexpectedly identical`). Shallow-copy the native backing
            // itself (mirroring the `PyObject::List`/`Dict`/`Set`/`Tuple`
            // arms just above) and wrap it in a NEW `Instance` of the same
            // class, instead of falling through to identity.
            PyObject::Instance { typ, dict } if crate::object::native_backing_of(obj).is_some() => {
                let native = crate::object::native_backing_of(obj).unwrap();
                let new_native = match &*native.borrow() {
                    PyObject::List(items) => py_list(items.clone()),
                    PyObject::Tuple(items) => PyObjectRef::imm(PyObject::Tuple(items.clone())),
                    PyObject::Dict(d) => {
                        let mut nd = PyDict::new();
                        for (k, v) in d.items() {
                            let _ = nd.set(k, v);
                        }
                        PyObjectRef::new(PyObject::Dict(Box::new(nd)))
                    }
                    PyObject::Set(s) => {
                        let mut ns = PySet::new();
                        for item in s.to_vec() {
                            let _ = ns.add(item);
                        }
                        PyObjectRef::new(PyObject::Set(ns))
                    }
                    other => PyObjectRef::new(other.clone()),
                };
                let mut new_dict = dict.clone();
                new_dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), new_native);
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: typ.clone(),
                    dict: new_dict,
                }))
            }
            _ => {
                // For instances and custom types, try __copy__
                if let Ok(copy_method) = borrowed.get_attribute("__copy__") {
                    drop(borrowed);
                    // `call_function_disposable` (NOT the bare
                    // `crate::object::call_function` in exceptions_ctor.rs,
                    // which only handles BuiltinFunction/Closure) — a
                    // user-defined `__copy__` is a Python Function and must
                    // route through a real VM (test_copy).
                    return crate::object::call_function_disposable(
                        &copy_method,
                        vec![obj.clone()],
                        vec![],
                    );
                }
                drop(borrowed);
                Ok(obj.clone())
            }
        }
    });

    // `copy.replace(obj, /, **changes)` (Python 3.13+) — was missing
    // entirely. Real CPython dispatches to `type(obj).__replace__(obj,
    // **changes)`, which no type in this codebase actually defines yet —
    // rather than adding the full generic `__replace__` protocol (a much
    // bigger, separate effort), this covers the two shapes real code
    // actually uses: a namedtuple's own `_replace` method (already
    // implemented, see this session's namedtuple work), and the general
    // `type(obj)(**{**vars(obj), **changes})` pattern that's exactly how
    // `types.SimpleNamespace.__replace__` and dataclasses' generated
    // `__replace__` are themselves defined in real CPython — so this
    // produces the SAME result for any plain-attribute-holding instance,
    // just without a real `__replace__` slot to dispatch through.
    copy_func!("replace", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "replace() missing required argument: 'obj'",
            ));
        }
        let obj = args[0].clone();
        let changes: Vec<(PyObjectRef, PyObjectRef)> = if args.len() > 1 {
            match &*args[1].borrow() {
                PyObject::Dict(d) => d.items(),
                _ => vec![],
            }
        } else {
            vec![]
        };
        let changes_kv: Vec<(String, PyObjectRef)> =
            changes.iter().map(|(k, v)| (k.str(), v.clone())).collect();

        // A namedtuple instance's own dict already holds `_fields` alongside
        // its field values (see `nt_replace`'s own construction), so the
        // generic Instance-merge path below reconstructs a namedtuple
        // correctly too — no need for a separate `_replace`-dispatch branch.
        let instance_parts: Option<(PyObjectRef, Vec<(String, PyObjectRef)>)> = match &*obj.borrow()
        {
            PyObject::Instance { typ, dict } => Some((
                typ.clone(),
                dict.iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            )),
            _ => None,
        };
        match instance_parts {
            // Build the replacement instance DIRECTLY (same `typ`, a fresh
            // dict merging the original's attributes with `changes`) rather
            // than round-tripping through `type(obj)(**kwargs)` — several
            // native "instance-shaped" types (`types.SimpleNamespace`
            // foremost) are constructed via a dedicated `BuiltinFunction` in
            // their owning module, NOT via their `Instance.typ` field (an
            // ad-hoc `Type` with empty `bases`/`mro`, used for `isinstance`/
            // repr only) — calling THAT `Type` as if it were the real
            // constructor silently built an empty instance, dropping every
            // attribute. Direct construction sidesteps that mismatch
            // entirely and matches what `SimpleNamespace.__replace__` and a
            // plain dataclass without `__post_init__` validation logic
            // actually do semantically anyway (new instance, replaced
            // attributes, no side effects).
            Some((cls, mut new_dict)) => {
                for (k, v) in &changes_kv {
                    match new_dict.iter_mut().find(|(existing, _)| existing == k) {
                        Some(entry) => entry.1 = v.clone(),
                        None => new_dict.push((k.clone(), v.clone())),
                    }
                }
                let mut attrs = crate::object::AttrMap::new();
                for (k, v) in new_dict {
                    attrs.insert(k, v);
                }
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: cls,
                    dict: attrs,
                }))
            }
            None => Err(PyError::type_error(format!(
                "replace() does not support {} objects",
                obj.borrow().type_name()
            ))),
        }
    });

    copy_func!("deepcopy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("deepcopy() missing required argument"));
        }
        let obj = &args[0];
        let memo = if args.len() > 1 {
            args[1].clone()
        } else {
            py_dict()
        };
        // Delegate entirely to `deepcopy_one` — this used to duplicate its
        // whole List/Tuple/Dict/`__deepcopy__` dispatch inline, with the
        // SAME "memoize after recursing instead of before" bug fixed there
        // (see its own doc comment): a self-referential dict/list passed
        // DIRECTLY to `copy.deepcopy(d)` recursed forever, because this
        // top-level call site's own copy of the logic never registered `d`
        // in `memo` before recursing into `d`'s own self-referencing value,
        // even after `deepcopy_one`'s NESTED recursion was fixed to do so
        // correctly. Confirmed via CPython's own
        // `test_copy.py::test_deepcopy_reflexive_dict`.
        crate::object::deepcopy_one(obj, &memo)
    });

    // Error class
    d.insert_str(
        "Error",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Error".to_string(),
            func: |args| {
                let msg = if !args.is_empty() {
                    args[0].str()
                } else {
                    "copy error".to_string()
                };
                Err(PyError::Exception(msg, py_none()))
            },
        }),
    );

    // `copy.__all__` — same fix, same reason, as `operator.__all__`
    // (`core.rs`) — missing entirely, breaking the module's own
    // `test___all__` sanity check at collection time.
    let all_names: Vec<PyObjectRef> = d
        .keys()
        .filter(|k| !k.starts_with('_'))
        .map(|k| py_str(k))
        .collect();
    d.insert_str("__all__", py_list(all_names));

    d
}
