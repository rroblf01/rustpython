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
    // See `generic_alias_native_ctor`'s own doc comment: makes
    // `GenericAlias(origin, args)` — called directly, e.g. via real
    // `Lib/_collections_abc.py`'s `classmethod(GenericAlias)` — build a
    // properly populated instance instead of falling through to default
    // (empty-dict) object construction.
    td.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "GenericAlias".to_string(),
            func: generic_alias_native_ctor,
        }),
    );
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
    // PEP 560: `class Foo(list[int]): ...` — `list[int]` is a
    // `GenericAlias` INSTANCE, not a class, so real class creation
    // substitutes it with `self.__origin__` (`list`) before actually
    // building the class. See `handle_build_class`'s own `__mro_entries__`
    // resolution step (`vm/call_class.rs`) for the caller side.
    td.insert_str(
        "__mro_entries__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__mro_entries__".to_string(),
            func: |args: &[PyObjectRef]| {
                let origin = generic_alias_origin(
                    args.first()
                        .ok_or_else(|| PyError::type_error("missing self"))?,
                )
                .ok_or_else(|| PyError::type_error("GenericAlias has no origin"))?;
                Ok(py_tuple(vec![origin]))
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
    // Plain `__new__` (as opposed to the `NATIVE_VALUE_CTOR_KEY` fast path
    // registered above, used for a DIRECT `GenericAlias(origin, args)`
    // call): needed so a real Python subclass can reach it via
    // `super().__new__(cls, origin, args)` — real trigger: real
    // `Lib/_collections_abc.py`'s own `class _CallableGenericAlias
    // (GenericAlias): def __new__(cls, origin, args): ... return
    // super().__new__(cls, origin, args)` (backing `Callable[[int], str]`
    // -style subscripting). `super()` resolves dunder lookups through the
    // MRO, not through this convenience-call convention, so both need to
    // exist side by side with their own (different) calling conventions —
    // `cls` explicit here, implicit (always this exact type) there.
    td.insert_str(
        "__new__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__new__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.len() < 3 {
                    return Err(PyError::type_error(
                        "GenericAlias.__new__ requires (cls, origin, args)",
                    ));
                }
                let cls = args[0].clone();
                let origin = args[1].clone();
                let item_args = match &*args[2].borrow() {
                    PyObject::Tuple(t) => t.clone(),
                    _ => vec![args[2].clone()],
                };
                let mut inst_dict = AttrMap::new();
                inst_dict.insert_str("__origin__", origin);
                inst_dict.insert_str("__args__", py_tuple(item_args));
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: cls,
                    dict: inst_dict,
                }))
            },
        }),
    );
    let typ = PyObjectRef::new(PyObject::Type {
        name: "types.GenericAlias".to_string(),
        dict: Box::new(str_map_to_typedict(td)),
        bases: vec![],
        mro: vec![],
    });
    // A non-empty `mro` containing at least itself — several general
    // mechanisms (`isinstance`/`issubclass`'s mro-walk arms, `super()`
    // resolution for a real Python subclass) key off a type's own `mro`
    // and silently no-op/fail for an empty one, the same recurring
    // ad-hoc-native-`Type` gap documented on several OTHER native types in
    // this codebase (`Fraction`, `namedtuple`-generated classes, ...).
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
        if let Some(object_type) = crate::object::get_primitive_type("object") {
            if !object_type.is(&typ) {
                mro.push(object_type);
            }
        }
    }
    typ
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

/// `types.GenericAlias(origin, args)` called directly as a constructor —
/// real trigger: real `Lib/_collections_abc.py`'s
/// `__class_getitem__ = classmethod(GenericAlias)` (assigned to EVERY ABC
/// there), so `Mapping[str, str]` ultimately calls `GenericAlias(Mapping,
/// (str, str))`. Without a real native constructor wired up here (this
/// type used to be buildable only via the internal `make_generic_alias`
/// helper, called directly from Rust — e.g. `list[int]`'s own subscript
/// handler — never through an ordinary Python-level call), calling it
/// fell through to plain default object construction: a `PyObject::
/// Instance` with an EMPTY dict, no `__origin__`/`__args__` ever set, so
/// `Mapping[str, str].__origin__` — and, more importantly, subclassing it
/// via `__mro_entries__` — silently produced a broken, originless alias.
/// `args` may be a single item (`GenericAlias(list, int)`) or a tuple
/// (`GenericAlias(dict, (str, str))`) — real CPython normalizes both the
/// same way.
fn generic_alias_native_ctor(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "GenericAlias() takes 2 positional arguments (origin, args)",
        ));
    }
    let origin = args[0].clone();
    let item_args = match &*args[1].borrow() {
        PyObject::Tuple(t) => t.clone(),
        _ => vec![args[1].clone()],
    };
    Ok(make_generic_alias(origin, item_args))
}
