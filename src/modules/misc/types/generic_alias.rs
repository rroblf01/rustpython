use crate::object::*;
use std::collections::HashMap;

use super::union::make_union;

thread_local! {
    static GENERIC_ALIAS_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

fn generic_alias_origin(inst: &PyObjectRef) -> Option<PyObjectRef> {
    let obj = inst.borrow();
    if let PyObject::Instance { dict, .. } = &*obj {
        dict.get_str("__origin__").cloned()
    } else {
        None
    }
}

fn generic_alias_args(inst: &PyObjectRef) -> Vec<PyObjectRef> {
    let obj = inst.borrow();
    if let PyObject::Instance { dict, .. } = &*obj {
        if let Some(a) = dict.get_str("__args__") {
            if let PyObject::Tuple(t) = &*a.borrow() {
                return t.clone();
            }
        }
    }
    vec![]
}

fn build_generic_alias_type() -> PyObjectRef {
    let mut td: HashMap<String, PyObjectRef> = HashMap::new();
    // __or__ / __ror__ (PEP 604: `list[int] | None` / `dict[str,str] | None`)
    // (BuiltinFunction funcs are fn pointers, so no captured closures.)
    td.insert_str(
        "__or__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__or__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 2 {
                    return Err(PyError::type_error("missing argument"));
                }
                Ok(make_union(vec![args[0].clone(), args[1].clone()]))
            },
        }),
    );
    td.insert_str(
        "__ror__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__ror__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 2 {
                    return Err(PyError::type_error("missing argument"));
                }
                Ok(make_union(vec![args[1].clone(), args[0].clone()]))
            },
        }),
    );
    // __getitem__ (nested generics: `list[int][str]` — rare, but real)
    td.insert_str(
        "__getitem__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getitem__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 2 {
                    return Err(PyError::type_error("missing argument"));
                }
                let origin = generic_alias_origin(&args[0]);
                let base = generic_alias_args(&args[0]);
                let mut new_args = base;
                new_args.push(args[1].clone());
                match origin {
                    Some(o) => Ok(make_generic_alias(o, new_args)),
                    None => Err(PyError::type_error("GenericAlias has no origin")),
                }
            },
        }),
    );
    td.insert_str(
        "__eq__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__eq__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 2 {
                    return Ok(py_bool(false));
                }
                let other = &args[1];
                let ob = other.borrow();
                if let PyObject::Instance { .. } = &*ob {
                    let my_origin = generic_alias_origin(&args[0]);
                    let my_args = generic_alias_args(&args[0]);
                    let oo = generic_alias_origin(other);
                    let oa = generic_alias_args(other);
                    drop(ob);
                    let same_origin = match (my_origin, oo) {
                        (Some(a), Some(b)) => a.is(&b) || a.equals(&b).unwrap_or(false),
                        _ => false,
                    };
                    let same_args = my_args.len() == oa.len()
                        && my_args
                            .iter()
                            .zip(oa.iter())
                            .all(|(x, y)| x.is(y) || x.equals(y).unwrap_or(false));
                    return Ok(py_bool(same_origin && same_args));
                }
                Ok(py_bool(false))
            },
        }),
    );
    td.insert_str(
        "__hash__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__hash__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error("missing self"));
                }
                Ok(py_int(args[0].hash()? as i64))
            },
        }),
    );
    td.insert_str(
        "__copy__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__copy__".to_string(),
            func: |args: &[PyObjectRef]| Ok(args[0].clone()),
        }),
    );
    td.insert_str(
        "__origin__",
        PyObjectRef::new(PyObject::Property(Box::new(crate::object::PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__origin__".to_string(),
                func: |args: &[PyObjectRef]| {
                    generic_alias_origin(
                        args.first()
                            .ok_or_else(|| PyError::type_error("missing self"))?,
                    )
                    .ok_or_else(|| PyError::type_error("GenericAlias has no origin"))
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    td.insert_str(
        "__args__",
        PyObjectRef::new(PyObject::Property(Box::new(crate::object::PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__args__".to_string(),
                func: |args: &[PyObjectRef]| {
                    Ok(py_tuple(generic_alias_args(
                        args.first()
                            .ok_or_else(|| PyError::type_error("missing self"))?,
                    )))
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    // repr: `list[int]`, `dict[str, str]`
    td.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |args: &[PyObjectRef]| {
                let origin = generic_alias_origin(
                    args.first()
                        .ok_or_else(|| PyError::type_error("missing self"))?,
                )
                .ok_or_else(|| PyError::type_error("GenericAlias has no origin"))?;
                let origin_repr = match &*origin.borrow() {
                    PyObject::Type { name, .. } => name.clone(),
                    _ => origin.borrow().repr(),
                };
                let arg_reprs: Vec<String> = generic_alias_args(&args[0])
                    .iter()
                    .map(|a| a.borrow().repr())
                    .collect();
                Ok(py_str(&format!(
                    "{}[{}]",
                    origin_repr,
                    arg_reprs.join(", ")
                )))
            },
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "types.GenericAlias".to_string(),
        dict: Box::new(str_map_to_typedict(td)),
        bases: vec![],
        mro: vec![],
    })
}

pub fn get_generic_alias_type() -> PyObjectRef {
    let existing = GENERIC_ALIAS_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_generic_alias_type();
    GENERIC_ALIAS_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

pub fn make_generic_alias(origin: PyObjectRef, args: Vec<PyObjectRef>) -> PyObjectRef {
    let mut inst_dict = AttrMap::new();
    inst_dict.insert_str("__origin__", origin);
    inst_dict.insert_str("__args__", py_tuple(args));
    PyObjectRef::new(PyObject::Instance {
        typ: get_generic_alias_type(),
        dict: inst_dict,
    })
}
