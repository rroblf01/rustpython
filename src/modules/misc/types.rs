use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;
#[allow(unused_imports)]
use std::cell::RefCell;

thread_local! {
    static SIMPLE_NAMESPACE_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

fn build_simple_namespace_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    // Real CPython's `SimpleNamespace.__repr__` lists attributes SORTED by
    // name (`namespace(x=1, y=2)`, regardless of assignment order) —
    // confirmed against real Python behavior, not guessed.
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            if let PyObject::Instance { typ, dict } = &*args[0].borrow() {
                let cls_name = {
                    let tb = typ.borrow();
                    if let PyObject::Type { name, .. } = &*tb {
                        if name == "types.SimpleNamespace" {
                            "namespace".to_string()
                        } else {
                            // For subclasses, use subclass name (e.g. AdvancedNamespace)
                            // Strip "types." prefix if present
                            if name.starts_with("types.") {
                                name["types.".len()..].to_string()
                            } else {
                                name.clone()
                            }
                        }
                    } else {
                        "namespace".to_string()
                    }
                };
                let mut items: Vec<(String, PyObjectRef)> = dict
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect();
                items.sort_by(|a, b| a.0.cmp(&b.0));
                let body = items
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v.repr()))
                    .collect::<Vec<_>>()
                    .join(", ");
                Ok(py_str(&format!("{}({})", cls_name, body)))
            } else {
                Ok(py_str("namespace()"))
            }
        }),
    );
    // Real CPython compares two SimpleNamespaces by their `__dict__`s.
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if args.len() < 2 {
                return Ok(py_bool(false));
            }
            let a = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                Some(dict.clone())
            } else {
                None
            };
            let b = if let PyObject::Instance { dict, .. } = &*args[1].borrow() {
                Some(dict.clone())
            } else {
                None
            };
            match (a, b) {
                (Some(a), Some(b)) => {
                    if a.len() != b.len() {
                        return Ok(py_bool(false));
                    }
                    for (k, v) in a.iter() {
                        match b.get(k) {
                            Some(bv) if v.equals(bv)? => {}
                            _ => return Ok(py_bool(false)),
                        }
                    }
                    Ok(py_bool(true))
                }
                _ => Ok(py_bool(false)),
            }
        }),
    );
    // Make SimpleNamespace callable and subclassable
    type_dict.insert_str(
        "__call__",
        bf!("__call__", |args| {
            panic!("SimpleNamespace __call__ called");
            eprintln!("DEBUG SimpleNamespace __call__ args len {} last {}", args.len(), args.last().map(|a| a.borrow().type_name()).unwrap_or("none".to_string()));
            let typ = args[0].clone();
            let mut inst_dict = crate::object::AttrMap::new();
            // args[0] is the type, args[1..] are positional, last may be kwargs dict
            // For SimpleNamespace(a=1, b=2), args is [Type, Dict{"a":1, "b":2}]
            // For SimpleNamespace() with no args, args is [Type] with no Dict
            if args.len() > 1 {
                if let Some(last) = args.last() {
                    eprintln!("DEBUG last type {}", last.borrow().type_name());
                    if let PyObject::Dict(items) = &*last.borrow() {
                        eprintln!("DEBUG last is Dict with {} items", items.len());
                        for (k, v) in items.items() {
                            inst_dict.insert(k.str(), v.clone());
                        }
                    }
                }
            }
            eprintln!("DEBUG inst_dict len {}", inst_dict.len());
            Ok(PyObjectRef::new(PyObject::Instance {
                typ,
                dict: inst_dict,
            }))
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "types.SimpleNamespace".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn get_simple_namespace_type() -> PyObjectRef {
    let existing = SIMPLE_NAMESPACE_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_simple_namespace_type();
    SIMPLE_NAMESPACE_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

thread_local! {
    static UNION_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// `__args__` of a `types.UnionType` instance (`int | str`), if `obj` is one
/// — checked by the ad-hoc type's own NAME (`"types.UnionType"`, unique to
/// this constructor) rather than object identity, avoiding a recursive
/// `get_union_type()` call from inside `make_union`'s own flattening pass.
pub(crate) fn union_args(obj: &PyObjectRef) -> Option<Vec<PyObjectRef>> {
    if let PyObject::Instance { typ, dict } = &*obj.borrow() {
        if matches!(&*typ.borrow(), PyObject::Type { name, .. } if name == "types.UnionType") {
            if let Some(a) = dict.get("__args__") {
                if let PyObject::Tuple(items) = &*a.borrow() {
                    return Some(items.clone());
                }
            }
        }
    }
    None
}

/// Builds (or extends) a PEP 604 union (`int | str`, `int | str | None`).
/// Flattens nested unions and de-duplicates by value equality — matching
/// real CPython (`int | int == int`, `int | (str | int) == int | str`).
/// A single remaining member collapses to that member directly, not a
/// one-element union (`int | int` IS `int`, not `UnionType` wrapping it).
pub(crate) fn make_union(parts: Vec<PyObjectRef>) -> PyObjectRef {
    let mut members: Vec<PyObjectRef> = Vec::new();
    for part in parts {
        let flattened = union_args(&part).unwrap_or_else(|| vec![part]);
        for m in flattened {
            if !members
                .iter()
                .any(|existing| existing.is(&m) || existing.equals(&m).unwrap_or(false))
            {
                members.push(m);
            }
        }
    }
    if members.len() == 1 {
        return members.into_iter().next().unwrap();
    }
    let mut inst_dict = AttrMap::new();
    inst_dict.insert_str("__args__", py_tuple(members));
    PyObjectRef::new(PyObject::Instance {
        typ: get_union_type(),
        dict: inst_dict,
    })
}

fn union_member_repr(m: &PyObjectRef) -> String {
    match &*m.borrow() {
        PyObject::None => "None".to_string(),
        PyObject::Type { name, .. } => name.clone(),
        _ => m.repr(),
    }
}

fn build_union_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    type_dict.insert(
        "__repr__".to_string(),
        bf!("__repr__", |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__repr__ missing argument"));
            }
            let members = union_args(&args[0]).unwrap_or_default();
            let parts: Vec<String> = members.iter().map(union_member_repr).collect();
            Ok(py_str(&parts.join(" | ")))
        }),
    );
    // Order-independent membership comparison (real CPython: `int | str ==
    // str | int`) — NOT a positional/sequence comparison.
    type_dict.insert(
        "__eq__".to_string(),
        bf!("__eq__", |args| {
            if args.len() < 2 {
                return Ok(py_not_implemented());
            }
            let a = match union_args(&args[0]) {
                Some(a) => a,
                None => return Ok(py_not_implemented()),
            };
            let b = match union_args(&args[1]) {
                Some(b) => b,
                None => return Ok(py_not_implemented()),
            };
            if a.len() != b.len() {
                return Ok(py_bool(false));
            }
            for x in &a {
                if !b.iter().any(|y| x.equals(y).unwrap_or(false)) {
                    return Ok(py_bool(false));
                }
            }
            Ok(py_bool(true))
        }),
    );
    // Order-independent hash (XOR, matching the order-independent __eq__
    // above) so a union is usable as a dict key/set member consistently
    // regardless of the order its members were written in.
    type_dict.insert(
        "__hash__".to_string(),
        bf!("__hash__", |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__hash__ missing argument"));
            }
            let members = union_args(&args[0]).unwrap_or_default();
            let mut h: i64 = 0;
            for m in &members {
                h ^= m.hash()? as i64;
            }
            Ok(py_int(h))
        }),
    );
    type_dict.insert(
        "__or__".to_string(),
        bf!("__or__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__or__() missing argument"));
            }
            Ok(make_union(vec![args[0].clone(), args[1].clone()]))
        }),
    );
    type_dict.insert(
        "__ror__".to_string(),
        bf!("__ror__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__ror__() missing argument"));
            }
            Ok(make_union(vec![args[1].clone(), args[0].clone()]))
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "types.UnionType".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub(crate) fn get_union_type() -> PyObjectRef {
    let existing = UNION_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_union_type();
    UNION_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

// `types.GenericAlias` — real generic-alias instances for `list[int]`,
// `dict[str, str]` etc. Previously `__class_getitem__` returned a bare
// TUPLE `(cls, item)` and `types.GenericAlias` was a placeholder string,
// so `dict[str, str] | None` (real code: configparser.py's class-level
// annotations) raised "unsupported operand types for |". An alias is an
// Instance of a shared GenericAlias type holding origin + args, with the
// union/equality/repr/attribute surface real code touches.
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

pub(crate) fn get_generic_alias_type() -> PyObjectRef {
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

pub(crate) fn make_generic_alias(origin: PyObjectRef, args: Vec<PyObjectRef>) -> PyObjectRef {
    let mut inst_dict = AttrMap::new();
    inst_dict.insert_str("__origin__", origin);
    inst_dict.insert_str("__args__", py_tuple(args));
    PyObjectRef::new(PyObject::Instance {
        typ: get_generic_alias_type(),
        dict: inst_dict,
    })
}

pub fn create_types_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! t_func {
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

    t_func!("FunctionType", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("FunctionType() requires an argument"));
        }
        Ok(args[0].clone())
    });
    // Real `types.DynamicClassAttribute` differs from plain `property` only
    // in a narrow metaclass-interop edge case (raising `AttributeError` on
    // class-level access so a metaclass's own `__getattr__` can take over —
    // `enum.py`'s own `Enum.name`/`Enum.value` use this internally). Aliased
    // to `property` directly rather than modeling that edge case: covers
    // the overwhelming majority of real usage (structural
    // getter/setter/deleter behavior), and unblocks the `ImportError:
    // cannot import name 'DynamicClassAttribute' from 'types'` that
    // otherwise hits any code merely importing it.
    t_func!("DynamicClassAttribute", builtin_property);
    t_func!("LambdaType", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("LambdaType() requires an argument"));
        }
        Ok(args[0].clone())
    });
    // Unlike `FunctionType`/`LambdaType` above (pure isinstance-check
    // helpers — real code essentially never CALLS them, since functions can
    // only be built by `def`/`lambda`), `types.MethodType(function,
    // instance)` genuinely IS a common real-world constructor — manually
    // binding a plain function to an instance, without going through a
    // class's own attribute lookup (e.g. dynamic method injection, certain
    // metaprogramming/proxy patterns). The passthrough-`args[0].clone()`
    // shape silently discarded the `instance` argument entirely, returning
    // the UNBOUND function — calling the result then called the function
    // with one fewer argument than it expects (self never supplied),
    // corrupting positional argument binding downstream (confirmed via a
    // repro: `types.MethodType(f, obj)(x)` raised `NameError` inside `f`
    // for its own `x` parameter, since `x` silently filled `self`'s slot
    // instead). Fixed to build a real `PyObject::BoundMethod`, the same
    // representation this interpreter already uses for `obj.method`
    // attribute access.
    d.insert_str(
        "MethodType",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "MethodType".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("MethodType() requires 2 arguments"));
                }
                Ok(PyObjectRef::new(PyObject::BoundMethod {
                    func: args[0].clone(),
                    self_obj: args[1].clone(),
                }))
            },
        }),
    );
    t_func!("BuiltinFunctionType", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "BuiltinFunctionType() requires an argument",
            ));
        }
        Ok(args[0].clone())
    });
    // Unlike its neighbors above (`FunctionType`/`LambdaType`/`MethodType`,
    // all pure isinstance-check helpers that only ever see an ALREADY-
    // EXISTING instance of their kind passed back in), `types.ModuleType`
    // is genuinely CONSTRUCTIBLE in real Python — `types.ModuleType(name)`
    // creates a brand-new, empty module object with that name (the exact
    // mechanism CPython's own `importlib` uses internally, and a common
    // idiom for building "fake modules" — real trigger: CPython's own
    // `test_call.py`). The passthrough-`args[0].clone()` shape used here
    // used to just return the NAME STRING unchanged, silently masquerading
    // as a module — any subsequent `.attr = value` on it then tried to
    // `borrow_mut()` an inline `PyObjectRef::SmallStr`, panicking
    // ("borrow_mut on non-mutable value") instead of setting a real module
    // attribute.
    d.insert_str(
        "ModuleType",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "ModuleType".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "module.__init__() takes at least 1 argument (0 given)",
                    ));
                }
                let name = args[0].str();
                let module = crate::object::create_module(&name, HashMap::new());
                if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                    dict.insert_str("__name__", crate::object::py_str(&name));
                    dict.insert_str(
                        "__doc__",
                        if args.len() > 1 {
                            args[1].clone()
                        } else {
                            crate::object::py_none()
                        },
                    );
                }
                Ok(module)
            },
        }),
    );
    t_func!("NoneType", |_| Ok(py_none()));
    t_func!("GeneratorType", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("GeneratorType() requires an argument"));
        }
        Ok(args[0].clone())
    });
    t_func!("CoroutineType", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("CoroutineType() requires an argument"));
        }
        Ok(args[0].clone())
    });
    t_func!("AsyncGeneratorType", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "AsyncGeneratorType() requires an argument",
            ));
        }
        Ok(args[0].clone())
    });
    // Real `types.SimpleNamespace(**kwargs)` creates an object exposing
    // each keyword as an ATTRIBUTE (`ns.x`), with a `namespace(x=1, y=2)`
    // repr and by-value equality — NOT a plain dict (a plain `PyObject::
    // Dict` doesn't support attribute-style access at all, so `ns.x` used
    // to raise `AttributeError: 'dict' object has no attribute 'x'`, a
    // real, common idiom broken outright). Kwargs arrive as a single
    // trailing packed dict per this project's own calling convention (see
    // e.g. `dict(mapping, key=val)`'s handling elsewhere) — real
    // `SimpleNamespace` takes no positional arguments at all, so the ONLY
    // arg ever present here is that trailing kwargs dict, if any.
    d.insert_str("SimpleNamespace", get_simple_namespace_type());
    // `types.UnionType` — the runtime type of `int | str` (PEP 604). Only
    // exposed as a name here (real code mostly just needs `isinstance(x,
    // types.UnionType)` or the name to exist for introspection/`__all__`
    // checks) — the actual construction happens via `__or__`/`__ror__` on
    // every `Type` object (see `attrs.rs`), not by calling this directly
    // (real `UnionType` isn't constructible by calling it either).
    d.insert_str("UnionType", get_union_type());
    // `@types.coroutine` — real CPython marks the generator function so its
    // resulting generator gets coroutine-like `__await__`/`send`/`throw`
    // behavior. This interpreter's own generator objects already expose
    // `__await__`/`__iter__` unconditionally (see `object.rs`'s Generator
    // attribute-access arm), so the decorator itself only needs to be a
    // transparent passthrough — real trigger: CPython's own `test.support`,
    // `@types.coroutine\ndef async_yield(v): return (yield v)`.
    t_func!("coroutine", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("coroutine() requires an argument"));
        }
        Ok(args[0].clone())
    });
    {
        // A real (minimal) Type, not a bare placeholder string — needed so
        // `CodeType.__init__` resolves to something attribute-accessible
        // (real trigger: `unittest/mock.py`'s own module-level
        // `inspect.signature(partial(CodeType.__init__, None))`, which
        // otherwise raises `AttributeError` — on a plain str — before ever
        // reaching the `try/except ValueError:` guarding that line).
        let mut code_type_dict = HashMap::new();
        code_type_dict.insert_str(
            "__init__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        let code_type = PyObjectRef::new(PyObject::Type {
            name: "code".to_string(),
            dict: Box::new(str_map_to_typedict(code_type_dict)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str("CodeType", code_type);
    }
    // `types.TracebackType(next, frame, lasti, lineno)` — a real Type whose
    // __init__ validates its 4 arguments and stores them on the instance
    // (readable as tb_next/tb_frame/tb_lasti/tb_lineno via the normal
    // Instance attribute path). Real trigger: CPython's own `test_raise.py`
    // TestTracebackType tests, which construct and attribute-check one.
    {
        let mut tb_type_dict = HashMap::new();
        tb_type_dict.insert_str(
            "__init__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if args.len() != 5 {
                        return Err(PyError::type_error(format!(
                            "TracebackType() takes 4 arguments ({} given)",
                            args.len().saturating_sub(1)
                        )));
                    }
                    let (next, frame, lasti, lineno) = (&args[1], &args[2], &args[3], &args[4]);
                    if !matches!(&*next.borrow(), PyObject::None)
                        && !matches!(&*next.borrow(), PyObject::Instance { .. })
                    {
                        return Err(PyError::type_error(
                            "TracebackType.__init__(): tb_next must be a traceback or None",
                        ));
                    }
                    if !matches!(&*frame.borrow(), PyObject::Instance { .. }) {
                        return Err(PyError::type_error(
                            "TracebackType.__init__(): frame must be a frame object",
                        ));
                    }
                    if lasti.as_i64().is_none() {
                        return Err(PyError::type_error(
                            "TracebackType.__init__(): lasti must be an integer",
                        ));
                    }
                    if lineno.as_i64().is_none() {
                        return Err(PyError::type_error(
                            "TracebackType.__init__(): lineno must be an integer",
                        ));
                    }
                    if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                        dict.insert_str("tb_next", next.clone());
                        dict.insert_str("tb_frame", frame.clone());
                        dict.insert_str("tb_lasti", lasti.clone());
                        dict.insert_str("tb_lineno", lineno.clone());
                    }
                    Ok(py_none())
                },
            }),
        );
        let tb_type = PyObjectRef::new(PyObject::Type {
            name: "TracebackType".to_string(),
            dict: Box::new(str_map_to_typedict(tb_type_dict)),
            bases: vec![],
            mro: vec![],
        });
        // CPython's traceback objects reject `del tb.tb_next` and validate
        // `tb.tb_next = <value>` (must be a traceback or None; must not create
        // a cycle). test_raise::TestTracebackType::test_attrs asserts all of
        // this on real tracebacks.
        if let PyObject::Type { dict, .. } = &mut *tb_type.borrow_mut() {
            let mut setattr_dict: HashMap<String, PyObjectRef> = HashMap::new();
            setattr_dict.insert_str(
                "__setattr__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__setattr__".to_string(),
                    func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        let name = match args.get(1) {
                            Some(a) => match &*a.borrow() {
                                PyObject::Str(s) => s.to_string(),
                                _ => return Ok(py_none()),
                            },
                            None => return Ok(py_none()),
                        };
                        if name == "tb_next" {
                            let value = args.get(2).cloned().unwrap_or_else(py_none);
                            if !matches!(&*value.borrow(), PyObject::None) {
                                if !matches!(&*value.borrow(), PyObject::Instance { .. }) {
                                    return Err(PyError::type_error(
                                        "tb_next must be a traceback or None",
                                    ));
                                }
                                let self_obj = &args[0];
                                let mut cur = value.clone();
                                loop {
                                    if cur.is(self_obj) {
                                        return Err(PyError::value_error("cannot create cycles"));
                                    }
                                    let nxt = cur
                                        .borrow()
                                        .get_attribute("tb_next")
                                        .unwrap_or_else(|_| py_none());
                                    if matches!(&*nxt.borrow(), PyObject::None) {
                                        break;
                                    }
                                    cur = nxt;
                                }
                            }
                        }
                        Ok(py_none())
                    },
                }),
            );
            setattr_dict.insert_str(
                "__delattr__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__delattr__".to_string(),
                    func: |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                        Err(PyError::type_error("read-only attribute"))
                    },
                }),
            );
            for (k, v) in setattr_dict {
                dict.insert_str(&k, v);
            }
        }
        d.insert_str("TracebackType", tb_type);
    }
    d.insert_str("CellType", py_str("cell"));
    // `types.MappingProxyType(dict)` — a read-only view of a mapping. Only
    // a placeholder ("mappingproxy") string before, so `types.
    // MappingProxyType({})` (real trigger: CPython's own `test_hmac.py`,
    // a default arg unpacked via `**`) blew up with "'str' object is not
    // callable". Implemented as a callable that wraps the given dict in an
    // Instance exposing `keys`/`__iter__`/`__getitem__`/`get`/`__len__`/
    // `items`/`__contains__`; the dict stays shared with the caller (a true
    // view: mutations through the original dict are visible).
    d.insert_str(
        "MappingProxyType",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "MappingProxyType".to_string(),
            func: |args| {
                if args.len() != 1 {
                    return Err(PyError::type_error(
                        "mappingproxy() takes exactly one argument",
                    ));
                }
                let src = args[0].clone();
                let is_dict = matches!(&*src.borrow(), PyObject::Dict(_));
                let inner: PyObjectRef = if is_dict {
                    src.clone()
                } else {
                    // Non-dict mapping: materialize a snapshot dict via items().
                    let mut items = Vec::new();
                    if let Ok(it) = builtin_iter(&[src]) {
                        loop {
                            match builtin_next(&[it.clone()]) {
                                Ok(v) => {
                                    if let PyObject::Tuple(vals) = &*v.borrow() {
                                        if vals.len() == 2 {
                                            items.push((vals[0].clone(), vals[1].clone()));
                                        }
                                    }
                                }
                                Err(PyError::StopIteration) => break,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    let mut d = crate::object::PyDict::new();
                    for (k, v) in items {
                        let _ = d.set(k, v);
                    }
                    PyObjectRef::new(PyObject::Dict(Box::new(d)))
                };
                let mut dict = crate::object::AttrMap::new();
                let typ = PyObjectRef::new(PyObject::Type {
                    name: "mappingproxy".to_string(),
                    dict: Box::new(str_map_to_typedict({
                        let mut td = HashMap::new();
                        // Each method captures `inner` directly rather than
                        // relying on self: attribute-call (`m.get(k)`) passes a
                        // bare Closure with NO self, while the dunder/subscript
                        // paths (`m[k]`, `len(m)`) prepend it — so reading the
                        // key as the LAST arg works for both shapes.
                        let key_arg = |args: &[PyObjectRef]| args.last().cloned();
                        for (name, field) in [
                            ("keys", "keys"),
                            ("values", "values"),
                            ("items", "items"),
                            ("__len__", "len"),
                            ("__iter__", "keys"),
                        ] {
                            let inner = inner.clone();
                            let field = field.to_string();
                            td.insert_str(
                                name,
                                PyObjectRef::new(PyObject::Closure(Rc::new(
                                    move |_args: &[PyObjectRef]| {
                                        if let PyObject::Dict(d) = &*inner.borrow() {
                                            match field.as_str() {
                                                "keys" => {
                                                    Ok(py_list(d.keys().iter().cloned().collect()))
                                                }
                                                "values" => Ok(py_list(
                                                    d.values().iter().cloned().collect(),
                                                )),
                                                "items" => Ok(py_list(
                                                    d.items()
                                                        .into_iter()
                                                        .map(|(k, v)| py_tuple(vec![k, v]))
                                                        .collect(),
                                                )),
                                                "len" => Ok(py_int(d.len() as i64)),
                                                _ => Err(PyError::runtime_error(
                                                    "unhandled mappingproxy field",
                                                )),
                                            }
                                        } else {
                                            Err(PyError::type_error(
                                                "mappingproxy wrapping a non-dict",
                                            ))
                                        }
                                    },
                                ))),
                            );
                        }
                        // `mappingproxy({...})` repr — a BuiltinFunction (not a
                        // capturing Closure, which pprint's _dispatch hashing
                        // can't key) reading the backing from the instance
                        // (test_pprint::test_mapping_proxy).
                        td.insert_str(
                            "__repr__",
                            PyObjectRef::new(PyObject::BuiltinFunction {
                                name: "__repr__".to_string(),
                                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    let backing = crate::object::native_backing_of(&args[0])
                                        .ok_or_else(|| {
                                            PyError::runtime_error("mappingproxy has no backing")
                                        })?;
                                    Ok(py_str(&format!("mappingproxy({})", backing.repr())))
                                },
                            }),
                        );
                        for (name, field) in [
                            ("get", "get"),
                            ("__getitem__", "getitem"),
                            ("__contains__", "contains"),
                        ] {
                            let inner = inner.clone();
                            let field = field.to_string();
                            td.insert_str(
                                name,
                                PyObjectRef::new(PyObject::Closure(Rc::new(
                                    move |args: &[PyObjectRef]| {
                                        let k = key_arg(args).ok_or_else(|| {
                                            PyError::type_error(format!(
                                                "{}() missing key argument",
                                                field
                                            ))
                                        })?;
                                        if let PyObject::Dict(d) = &*inner.borrow() {
                                            match field.as_str() {
                                                "contains" => {
                                                    Ok(py_bool(d.contains(&k).unwrap_or(false)))
                                                }
                                                "get" => {
                                                    let key =
                                                        args.first().cloned().ok_or_else(|| {
                                                            PyError::type_error("get() missing key")
                                                        })?;
                                                    match d.get(&key).ok().flatten() {
                                                        Some(v) => Ok(v),
                                                        None => {
                                                            Ok(args.get(1).cloned().unwrap_or_else(
                                                                || PyObjectRef::new(PyObject::None),
                                                            ))
                                                        }
                                                    }
                                                }
                                                "getitem" => match d.get(&k).ok().flatten() {
                                                    Some(v) => Ok(v),
                                                    None => Err(PyError::key_error(k.repr())),
                                                },
                                                _ => Err(PyError::runtime_error(
                                                    "unhandled mappingproxy field",
                                                )),
                                            }
                                        } else {
                                            Err(PyError::type_error(
                                                "mappingproxy wrapping a non-dict",
                                            ))
                                        }
                                    },
                                ))),
                            );
                        }
                        td
                    })),
                    bases: vec![],
                    mro: vec![],
                });
                dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), inner.clone());
                Ok(PyObjectRef::new(PyObject::Instance { typ, dict }))
            },
        }),
    );
    // GenericAlias — used for generic type annotations like list[int], dict[str, int]
    d.insert_str("GenericAlias", get_generic_alias_type());

    d
}
