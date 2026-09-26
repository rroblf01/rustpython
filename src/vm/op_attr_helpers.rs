use crate::object::*;
use std::rc::Rc;

/// Try to resolve attribute via native backing for Instance that subclasses
/// a native container (list/dict/str/deque etc.). Returns Some if found via
/// native backing, None otherwise.
pub fn try_native_backing(dict: &AttrMap, typ: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    let native = dict.get(NATIVE_BACKING_KEY)?;
    // A deque subclass's `__copy__`/`copy()` must return a NEW instance of the SAME subclass
    if matches!(&*native.borrow(), PyObject::Deque { .. }) && (name == "__copy__" || name == "copy")
    {
        let typ_clone = typ.clone();
        let new_native = {
            let b = native.borrow();
            if let PyObject::Deque { data, maxlen } = &*b {
                py_deque(data.clone(), *maxlen)
            } else {
                unreachable!()
            }
        };
        return Some(PyObjectRef::new(PyObject::Closure(Rc::new(
            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                let mut new_dict = AttrMap::new();
                new_dict.insert(NATIVE_BACKING_KEY.to_string(), new_native.clone());
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: typ_clone.clone(),
                    dict: new_dict,
                }))
            },
        ))));
    }
    // A dict SUBCLASS's `__getitem__` must keep the INSTANCE as the
    // receiver, not the native backing: rebinding `self` to the backing
    // loses the subclass, so the `__missing__` hook (collections.Counter,
    // urllib's `_Quoter`) became unreachable through `d.__getitem__(k)`
    // even though the plain `d[k]` still worked. Route those names
    // through the instance's own `__getitem__` (looked up on its type, so
    // a user override still wins) so `py_getitem` sees the instance and
    // can call `__missing__`.
    if (name == "__getitem__" || name == "__missing__")
        && matches!(&*native.borrow(), PyObject::Dict(_))
    {
        // A user-defined `__getitem__` on the subclass wins (it is found
        // even with the native-dict "escape hatch" skipped, since it lives
        // in the subclass's own dict).
        if let Some(f) = crate::object::lookup_dunder_via_mro(typ, "__getitem__") {
            return Some(f);
        }
        // Otherwise hand back a bound lookup that keeps the INSTANCE as the
        // receiver, so `py_getitem` sees the subclass and can run its
        // `__missing__` (collections.Counter, urllib's `_Quoter`, …).
        // Returning the native dict's own method here bound `self` to the
        // backing and raised a raw KeyError instead.
        let owned_typ = typ.clone();
        let owned_dict = dict.clone();
        return Some(PyObjectRef::new(PyObject::Closure(Rc::new(
            move |call_args: &[PyObjectRef]| {
                let instance = PyObjectRef::new(PyObject::Instance {
                    typ: owned_typ.clone(),
                    dict: owned_dict.clone(),
                });
                crate::object::py_getitem(&instance, &call_args[0])
            },
        ))));
    }
    let val = native.borrow().get_attribute(name).ok()?;
    let rebound = match &*val.borrow() {
        PyObject::BuiltinMethod { name: n, func, .. } => {
            Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: n.clone(),
                func: *func,
                self_obj: native.clone(),
            }))
        }
        _ => None,
    };
    Some(rebound.unwrap_or(val))
}

/// PEP 3134 traceback/chaining fallback for exception subclasses
pub fn try_exception_attributes(
    typ: &PyObjectRef,
    name: &str,
    obj: &PyObjectRef,
) -> Option<PyObjectRef> {
    if matches!(
        name,
        "with_traceback"
            | "add_note"
            | "__traceback__"
            | "__context__"
            | "__cause__"
            | "__suppress_context__"
            | "__notes__"
    ) && find_exception_base_name(typ).is_some()
    {
        Some(match name {
            "with_traceback" => PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "with_traceback".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error(
                            "with_traceback() takes exactly one argument",
                        ));
                    }
                    Ok(args[0].clone())
                },
                self_obj: obj.clone(),
            }),
            "add_note" => PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "add_note".to_string(),
                func: |_args| Ok(py_none()),
                self_obj: obj.clone(),
            }),
            "__context__" | "__traceback__" | "__cause__" => py_none(),
            "__suppress_context__" => py_bool(false),
            "__notes__" => py_list(vec![]),
            _ => unreachable!(),
        })
    } else {
        None
    }
}

/// Fallback for dict methods on dict-derived instances (e.g. Counter, defaultdict subclasses)
///
/// Only for classes that actually inherit from `dict`. Applied to EVERY
/// instance it shadowed a real `__getattr__`: `MagicMock().get` came back as
/// the dict `get` builtin instead of an auto-created child mock, which is
/// what broke `test_getpass`'s `environ.get.return_value = ...` (and any
/// other mock attribute named `get`/`keys`/`items`/`values`).
pub fn try_dict_methods(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    if !is_dict_derived_class(obj) {
        return None;
    }
    if name == "__iter__" || name == "items" || name == "keys" || name == "values" || name == "get"
    {
        let func: BuiltinFunc = match name {
            "__iter__" => dict_method_iter,
            "items" => dict_method_items,
            "keys" => dict_method_keys,
            "values" => dict_method_values,
            "get" => dict_method_get,
            _ => return None,
        };
        Some(PyObjectRef::imm(PyObject::BuiltinMethod {
            name: name.to_string(),
            func,
            self_obj: obj.clone(),
        }))
    } else {
        None
    }
}

/// Does this object's class inherit from `dict` (directly or through its MRO)?
fn is_dict_derived_class(obj: &PyObjectRef) -> bool {
    let typ = match &*obj.borrow() {
        PyObject::Instance { typ, .. } => typ.clone(),
        PyObject::Type { .. } => obj.clone(),
        _ => return false,
    };
    let mro = match &*typ.borrow() {
        PyObject::Type { mro, .. } => mro.clone(),
        _ => return false,
    };
    mro.iter()
        .skip(1)
        .any(|base| matches!(&*base.borrow(), PyObject::Type { name, .. } if name == "dict"))
}
