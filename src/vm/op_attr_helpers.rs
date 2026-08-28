use crate::object::*;
use std::rc::Rc;

/// Try to resolve attribute via native backing for Instance that subclasses
/// a native container (list/dict/str/deque etc.). Returns Some if found via
/// native backing, None otherwise.
pub fn try_native_backing(
    dict: &AttrMap,
    typ: &PyObjectRef,
    name: &str,
) -> Option<PyObjectRef> {
    let native = dict.get(NATIVE_BACKING_KEY)?;
    // A deque subclass's `__copy__`/`copy()` must return a NEW instance of the SAME subclass
    if matches!(&*native.borrow(), PyObject::Deque { .. }) && (name == "__copy__" || name == "copy") {
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
                new_dict.insert(
                    NATIVE_BACKING_KEY.to_string(),
                    new_native.clone(),
                );
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: typ_clone.clone(),
                    dict: new_dict,
                }))
            },
        ))));
    }
    let val = native.borrow().get_attribute(name).ok()?;
    let rebound = match &*val.borrow() {
        PyObject::BuiltinMethod { name: n, func, .. } => Some(PyObjectRef::imm(PyObject::BuiltinMethod {
            name: n.clone(),
            func: *func,
            self_obj: native.clone(),
        })),
        _ => None,
    };
    Some(rebound.unwrap_or(val))
}

/// Fallback for dict methods on dict-derived instances (e.g. Counter, defaultdict subclasses)
pub fn try_dict_methods(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    if name == "__iter__"
        || name == "items"
        || name == "keys"
        || name == "values"
        || name == "get"
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
