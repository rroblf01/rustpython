use crate::modules::create_collections_abc_dict;
use crate::object::*;
use std::collections::HashMap;

pub fn create_json_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! json_func {
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

    json_func!("dumps", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("dumps() missing required argument"));
        }
        let indent = if args.len() > 1 {
            let v = args[1].as_i64().unwrap_or(-1);
            if v >= 0 {
                Some(v as usize)
            } else {
                None
            }
        } else {
            None
        };
        let sort_keys = if args.len() > 2 {
            args[2].truthy()
        } else {
            false
        };
        json_encode_full(&args[0], indent, sort_keys, 0)
    });

    json_func!("loads", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("loads() missing required argument"));
        }
        let s = args[0].str();
        json_decode(&s)
    });

    d
}

// Real `json.JSONEncoder` (subclassable, `default()` override point) is
// implemented as real Python source instead — see json_extra.py and
// VirtualMachine::install_source_defined_stdlib.
pub const JSON_EXTRA_SOURCE: &str = include_str!("json_extra.py");

/// `collections.OrderedDict(source)` — builds an OrderedDict instance (a
/// real type with its own repr) from a dict or an iterable of (k, v) pairs.
thread_local! {
    static OD_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        const { std::cell::RefCell::new(None) };
}
pub fn ordered_dict_new(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let typ = OD_TYPE
        .with(|c| c.borrow().clone())
        .ok_or_else(|| PyError::runtime_error("OrderedDict type not initialized"))?;
    let dict = crate::object::py_dict();
    if std::env::var("RPY_DEBUG_OD").is_ok() {
        eprintln!("OD new: nargs={}", args.len());
    }
    if !args.is_empty() {
        let source = &args[0];
        let borrowed = source.borrow();
        if let PyObject::Dict(d) = &*borrowed {
            for (k, v) in d.items() {
                if let PyObject::Dict(ref mut target) = &mut *dict.borrow_mut() {
                    let _ = target.set(k, v);
                }
            }
        } else {
            // Any iterable of (k, v) pairs.
            drop(borrowed);
            let it = crate::object::builtin_iter(&[args[0].clone()])?;
            loop {
                match crate::object::builtin_next(&[it.clone()]) {
                    Ok(pair) => {
                        if let PyObject::Tuple(vals) = &*pair.borrow() {
                            if vals.len() == 2 {
                                if let PyObject::Dict(ref mut target) = &mut *dict.borrow_mut() {
                                    let _ = target.set(vals[0].clone(), vals[1].clone());
                                }
                            }
                        }
                    }
                    Err(crate::object::PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
    }
    let backing = match &*dict.borrow() {
        PyObject::Dict(d) => PyObjectRef::new(PyObject::Dict(d.clone())),
        _ => unreachable!(),
    };
    let mut attrs = crate::object::AttrMap::new();
    attrs.insert(crate::object::NATIVE_BACKING_KEY.to_string(), backing);
    Ok(PyObjectRef::new(PyObject::Instance { typ, dict: attrs }))
}

pub fn create_collections_dict(object_type: PyObjectRef) -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! coll_func {
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

    // `deque` — a REAL subclassable native type (migrated from the plain
    // `BuiltinFunction` alias-to-`List` that used to be registered here,
    // which couldn't support `class X(deque)`, `d.appendleft()` (the
    // recurring `test_shlex` failure), bounded `maxlen` truncation, or
    // round-tripping through pickle). Registered exactly like `list`/
    // `int`/etc. in `create_builtins`: a `PyObject::Type` whose dict holds
    // the native constructor under `NATIVE_VALUE_CTOR_KEY` (so `deque(x)`
    // dispatches to `builtin_deque` and returns a raw `PyObject::Deque`,
    // never an `Instance`) plus `__init__` (`native_base_init_builtin`, so
    // `d.__init__(iterable)` / `deque.__init__(subclass_instance, ...)`
    // repopulate the backing) — `default_build_class`'s native-base
    // detection then makes `class X(deque): ...` a first-class subclassable
    // type like any other native container.
    let mut deque_dict: HashMap<String, PyObjectRef> = HashMap::new();
    deque_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "deque".to_string(),
            func: crate::object::builtin_deque,
        }),
    );
    deque_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    deque_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_deque_repr as crate::object::BuiltinFunc,
        }),
    );
    let deque_type = PyObjectRef::new(PyObject::Type {
        name: "deque".to_string(),
        dict: Box::new(str_map_to_typedict(deque_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    // `deque.__module__ == 'collections'` (reprlib's dispatch keyed on it).
    if let PyObject::Type { dict, .. } = &mut *deque_type.borrow_mut() {
        dict.insert_str("__module__", py_str("collections"));
    }
    if let PyObject::Type { mro, .. } = &mut *deque_type.borrow_mut() {
        *mro = vec![deque_type.clone(), object_type.clone()];
    }
    crate::object::seed_primitive_type_cache("deque", deque_type.clone());
    d.insert("deque".to_string(), deque_type);

    // OrderedDict: remembers insertion order — a real type (not a plain
    // dict) with its own `OrderedDict()`/`OrderedDict([(k, v), ...])` repr
    // (test_pprint::test_ordered_dict dispatches on its __repr__) and
    // dict-like behavior via a native dict backing. The type is cached in
    // the same thread_local `ordered_dict_new` reads.
    {
        let mut od_dict: HashMap<String, PyObjectRef> = HashMap::new();
        od_dict.insert_str(
            "__repr__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if args.is_empty() {
                        return Ok(py_str("OrderedDict()"));
                    }
                    let backing = crate::object::native_backing_of(&args[0])
                        .ok_or_else(|| PyError::runtime_error("OrderedDict.__repr__ on non-OD"))?;
                    let items: Vec<String> = {
                        let b = backing.borrow();
                        match &*b {
                            PyObject::Dict(d) => d
                                .items()
                                .iter()
                                .map(|(k, v)| format!("({}, {})", k.repr(), v.repr()))
                                .collect(),
                            _ => Vec::new(),
                        }
                    };
                    if items.is_empty() {
                        Ok(py_str("OrderedDict()"))
                    } else {
                        Ok(py_str(&format!("OrderedDict([{}])", items.join(", "))))
                    }
                },
            }),
        );
        od_dict.insert_str(
            "__module__",
            PyObjectRef::new(PyObject::Str(compact_str::CompactString::from(
                "collections",
            ))),
        );
        // Dict-backed: the type-construct path builds an Instance carrying a
        // native dict backing, so native_backing_of/len/getitem/etc. work.
        od_dict.insert_str(crate::object::NATIVE_BASE_MARKER, py_str("dict"));
        // `OrderedDict(source)` — the TYPE is the callable (so
        // `OrderedDict.__repr__`/`type(od).__repr__` are the same object,
        // which pprint's dispatch requires). __init__ populates the native
        // dict backing from a dict or iterable of (k, v) pairs.
        od_dict.insert_str(
            "__init__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if args.len() < 2 || matches!(&*args[1].borrow(), PyObject::None) {
                        return Ok(py_none());
                    }
                    let source = &args[1];
                    let borrowed = source.borrow();
                    let mut entries: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
                    if let PyObject::Dict(d) = &*borrowed {
                        for (k, v) in d.items() {
                            entries.push((k, v));
                        }
                    } else {
                        drop(borrowed);
                        let it = crate::object::builtin_iter(&[args[1].clone()])?;
                        loop {
                            match crate::object::builtin_next(&[it.clone()]) {
                                Ok(pair) => {
                                    if let PyObject::Tuple(vals) = &*pair.borrow() {
                                        if vals.len() == 2 {
                                            entries.push((vals[0].clone(), vals[1].clone()));
                                        }
                                    }
                                }
                                Err(crate::object::PyError::StopIteration) => break,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    if let Some(backing) = crate::object::native_backing_of(&args[0]) {
                        if let PyObject::Dict(d) = &mut *backing.borrow_mut() {
                            for (k, v) in entries {
                                let _ = d.set(k, v);
                            }
                        }
                    }
                    Ok(py_none())
                },
            }),
        );
        // dict-like behavior via the native dict backing.
        fn od_backing(args: &[PyObjectRef]) -> PyResult<crate::object::PyObjectRef> {
            crate::object::native_backing_of(&args[0])
                .ok_or_else(|| PyError::runtime_error("OrderedDict operation on non-OD"))
        }
        od_dict.insert_str(
            "__len__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__len__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    let b = od_backing(args)?;
                    let inner = b.borrow();
                    if let PyObject::Dict(d) = &*inner {
                        Ok(py_int(d.len() as i64))
                    } else {
                        Err(PyError::runtime_error("OrderedDict has no dict"))
                    }
                },
            }),
        );
        od_dict.insert_str(
            "__getitem__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__getitem__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    let key = args.get(1).cloned().unwrap_or_else(py_none);
                    let b = od_backing(args)?;
                    let inner = b.borrow();
                    if let PyObject::Dict(d) = &*inner {
                        match d.get(&key) {
                            Ok(Some(v)) => Ok(v),
                            _ => Err(PyError::KeyError(key.str())),
                        }
                    } else {
                        Err(PyError::runtime_error("OrderedDict has no dict"))
                    }
                },
            }),
        );
        od_dict.insert_str(
            "__contains__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__contains__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    let key = args.get(1).cloned().unwrap_or_else(py_none);
                    let b = od_backing(args)?;
                    let inner = b.borrow();
                    if let PyObject::Dict(d) = &*inner {
                        match d.get(&key) {
                            Ok(Some(_)) => Ok(py_bool(true)),
                            _ => Ok(py_bool(false)),
                        }
                    } else {
                        Ok(py_bool(false))
                    }
                },
            }),
        );
        // iteration / keys / items / values / get via the dict-backing
        // fallback already wired into LOAD_ATTR for dict-derived instances.
        let od_type = PyObjectRef::new(PyObject::Type {
            name: "OrderedDict".to_string(),
            dict: Box::new(str_map_to_typedict(od_dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *od_type.borrow_mut() {
            *mro = vec![od_type.clone()];
        }
        OD_TYPE.with(|c| *c.borrow_mut() = Some(od_type.clone()));
        crate::object::seed_primitive_type_cache("OrderedDict", od_type.clone());
        d.insert("OrderedDict".to_string(), od_type.clone());
    }

    // namedtuple: factory function — creates simple types with named fields
    coll_func!("namedtuple", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "namedtuple() needs at least 2 arguments",
            ));
        }
        let typename = args[0].str();
        // Real `namedtuple(typename, field_names)` accepts `field_names` as
        // EITHER a single string (space- and/or comma-separated: `"x y"`,
        // `"x, y"`) OR an iterable of per-field strings (`['x', 'y']`,
        // `('x', 'y')`) — only the string form was handled before (an
        // unconditional `.str()` on `args[1]`, so a real list/tuple
        // silently stringified to its OWN repr, e.g. `"['x', 'y']"`, which
        // then got whitespace-split into garbage field names like
        // `"['x',"`/`"'y']"`). This corrupted EVERY namedtuple constructed
        // with the list/tuple form — not just `_replace`/`copy.replace`,
        // which is what surfaced it (even a fresh instance's own `repr()`
        // and attribute access were broken).
        let fields: Vec<String> = match &*args[1].borrow() {
            PyObject::List(items) | PyObject::Tuple(items) => {
                items.iter().map(|i| i.str()).collect()
            }
            _ => {
                let field_str = args[1].str();
                field_str
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect()
            }
        };
        if fields.is_empty() {
            return Err(PyError::type_error(
                "namedtuple() requires at least 1 field name",
            ));
        }
        let n = fields.len();
        let f_clone = fields.clone();
        let tn_clone = typename.clone();
        // __init__: called by Type handler after creating empty Instance
        let init_fn = move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if args.len() < 1 {
                return Err(PyError::type_error("__init__ missing self"));
            }
            let self_obj = &args[0];
            let pos_args = &args[1..];
            if pos_args.len() != n {
                return Err(PyError::type_error(format!(
                    "{} expects {} arguments, got {}",
                    tn_clone,
                    n,
                    pos_args.len()
                )));
            }
            // Set field values as attributes on self
            for (i, f) in f_clone.iter().enumerate() {
                self_obj
                    .borrow_mut()
                    .set_attribute(f, pos_args[i].clone())
                    .ok();
            }
            self_obj
                .borrow_mut()
                .set_attribute(
                    "_fields",
                    PyObjectRef::new(PyObject::List(f_clone.iter().map(|f| py_str(f)).collect())),
                )
                .ok();
            Ok(py_none())
        };
        let init_obj = PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(init_fn)));
        let mut type_dict = HashMap::new();
        type_dict.insert_str("__init__", init_obj);

        // A real `namedtuple` instance IS a tuple (subclasses `tuple` in
        // real CPython) — comparing/hashing/iterating/indexing it must
        // behave exactly like the equivalent plain tuple of field values,
        // and `repr()` must show `TypeName(field=val, ...)`, not the
        // generic `<TypeName object>` fallback. All of this was missing
        // entirely (fields were stored as plain instance attributes with
        // no tuple-like behavior at all) — found via CPython's own
        // `urllib/robotparser.py`, whose `RequestRate = namedtuple(...)`
        // instances need to compare equal by value.
        //
        // Implemented as plain `BuiltinFunction`s (bare `fn` pointers, NOT
        // `PyObject::Closure`) that read the field-NAME list back off
        // `self._fields` (set by `__init__` above) at call time, rather
        // than capturing it — deliberately avoiding `Closure` for these,
        // since a `Closure` found via a shared TYPE's dict is NOT
        // auto-bound with `self` (unlike `Function`/`BuiltinFunction`):
        // `Closure` is also used elsewhere in this codebase (`io.BytesIO`'s
        // `read`/`write`/`seek`) for a PER-INSTANCE closure that captures
        // its own state and explicitly expects NO `self` prepended, so the
        // auto-binding rule can't apply to `Closure` unconditionally.
        // `BuiltinFunction` has no such ambiguity — it always auto-binds.
        fn nt_fields(self_obj: &PyObjectRef) -> PyResult<Vec<String>> {
            match self_obj.borrow().get_attribute("_fields")?.borrow().clone() {
                PyObject::List(items) => Ok(items.iter().map(|v| v.str()).collect()),
                _ => Err(PyError::type_error("not a namedtuple instance")),
            }
        }
        fn nt_field_values(self_obj: &PyObjectRef) -> PyResult<Vec<PyObjectRef>> {
            nt_fields(self_obj)?
                .iter()
                .map(|f| self_obj.borrow().get_attribute(f))
                .collect()
        }
        fn nt_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            let typename = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
                if let PyObject::Type { name, .. } = &*typ.borrow() {
                    name.clone()
                } else {
                    "namedtuple".to_string()
                }
            } else {
                "namedtuple".to_string()
            };
            let fields = nt_fields(&args[0])?;
            let vals = nt_field_values(&args[0])?;
            let parts: Vec<String> = fields
                .iter()
                .zip(vals.iter())
                .map(|(f, v)| format!("{}={}", f, v.repr()))
                .collect();
            Ok(py_str(&format!("{}({})", typename, parts.join(", "))))
        }
        fn nt_eq(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.len() < 2 {
                return Ok(py_bool(false));
            }
            let a = nt_field_values(&args[0])?;
            let b_tuple = match &*args[1].borrow() {
                PyObject::Tuple(t) => Some(t.clone()),
                PyObject::Instance { dict, .. } if dict.get_str("_fields").is_some() => {
                    Some(nt_field_values(&args[1])?)
                }
                _ => None,
            };
            match b_tuple {
                Some(b) if b.len() == a.len() => {
                    for (x, y) in a.iter().zip(b.iter()) {
                        if !x.equals(y)? {
                            return Ok(py_bool(false));
                        }
                    }
                    Ok(py_bool(true))
                }
                _ => Ok(py_not_implemented()),
            }
        }
        fn nt_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            builtin_iter(&[py_tuple(nt_field_values(&args[0])?)])
        }
        fn nt_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.len() < 2 {
                return Err(PyError::type_error("expected an index"));
            }
            py_getitem(&py_tuple(nt_field_values(&args[0])?), &args[1])
        }
        fn nt_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            Ok(py_int(nt_fields(&args[0])?.len() as i64))
        }
        fn nt_hash(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            Ok(py_int(py_tuple(nt_field_values(&args[0])?).hash()? as i64))
        }
        fn nt_asdict(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            let fields = nt_fields(&args[0])?;
            let vals = nt_field_values(&args[0])?;
            let mut d = crate::object::PyDict::new();
            for (f, v) in fields.iter().zip(vals.into_iter()) {
                d.set(py_str(f), v)?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
        }
        fn nt_replace(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            let self_obj = &args[0];
            let typ = if let PyObject::Instance { typ, .. } = &*self_obj.borrow() {
                typ.clone()
            } else {
                return Err(PyError::type_error("_replace on non-namedtuple"));
            };
            let fields = nt_fields(self_obj)?;
            let overrides: Vec<(String, PyObjectRef)> = if args.len() > 1 {
                match &*args[1].borrow() {
                    PyObject::Dict(d) => d.items().into_iter().map(|(k, v)| (k.str(), v)).collect(),
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            let mut new_dict = AttrMap::new();
            for f in &fields {
                let v = overrides
                    .iter()
                    .find(|(k, _)| k == f)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(self_obj.borrow().get_attribute(f)?);
                new_dict.insert_str(f, v);
            }
            new_dict.insert_str(
                "_fields",
                PyObjectRef::new(PyObject::List(fields.iter().map(|f| py_str(f)).collect())),
            );
            Ok(PyObjectRef::new(PyObject::Instance {
                typ,
                dict: new_dict,
            }))
        }

        fn nt_lt(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.len() < 2 {
                return Ok(py_bool(false));
            }
            let a_vals = nt_field_values(&args[0])?;
            let b_vals = match &*args[1].borrow() {
                PyObject::Tuple(t) => t.clone(),
                PyObject::Instance { dict, .. } if dict.get_str("_fields").is_some() => nt_field_values(&args[1])?,
                _ => return Ok(py_not_implemented()),
            };
            Ok(py_bool(crate::object::py_compare(&py_tuple(a_vals), &py_tuple(b_vals), 0)?.truthy()))
        }
        fn nt_le(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.len() < 2 {
                return Ok(py_bool(false));
            }
            let a_vals = nt_field_values(&args[0])?;
            let b_vals = match &*args[1].borrow() {
                PyObject::Tuple(t) => t.clone(),
                PyObject::Instance { dict, .. } if dict.get_str("_fields").is_some() => nt_field_values(&args[1])?,
                _ => return Ok(py_not_implemented()),
            };
            Ok(py_bool(crate::object::py_compare(&py_tuple(a_vals), &py_tuple(b_vals), 1)?.truthy()))
        }
        fn nt_gt(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.len() < 2 {
                return Ok(py_bool(false));
            }
            let a_vals = nt_field_values(&args[0])?;
            let b_vals = match &*args[1].borrow() {
                PyObject::Tuple(t) => t.clone(),
                PyObject::Instance { dict, .. } if dict.get_str("_fields").is_some() => nt_field_values(&args[1])?,
                _ => return Ok(py_not_implemented()),
            };
            Ok(py_bool(crate::object::py_compare(&py_tuple(a_vals), &py_tuple(b_vals), 4)?.truthy()))
        }
        fn nt_ge(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.len() < 2 {
                return Ok(py_bool(false));
            }
            let a_vals = nt_field_values(&args[0])?;
            let b_vals = match &*args[1].borrow() {
                PyObject::Tuple(t) => t.clone(),
                PyObject::Instance { dict, .. } if dict.get_str("_fields").is_some() => nt_field_values(&args[1])?,
                _ => return Ok(py_not_implemented()),
            };
            Ok(py_bool(crate::object::py_compare(&py_tuple(a_vals), &py_tuple(b_vals), 3)?.truthy()))
        }
        fn nt_ne(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            let eq = nt_eq(args)?;
            if crate::object::is_not_implemented(&eq) {
                return Ok(eq);
            }
            Ok(py_bool(!eq.truthy()))
        }
        macro_rules! nt_method {
            ($name:expr, $f:expr) => {
                type_dict.insert_str(
                    $name,
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: $name.to_string(),
                        func: $f,
                    }),
                );
            };
        }
        nt_method!("__repr__", nt_repr);
        nt_method!("__eq__", nt_eq);
        nt_method!("__lt__", nt_lt);
        nt_method!("__le__", nt_le);
        nt_method!("__gt__", nt_gt);
        nt_method!("__ge__", nt_ge);
        nt_method!("__ne__", nt_ne);
        nt_method!("__iter__", nt_iter);
        nt_method!("__getitem__", nt_getitem);
        nt_method!("__len__", nt_len);
        nt_method!("__hash__", nt_hash);
        nt_method!("_asdict", nt_asdict);
        nt_method!("_replace", nt_replace);
        // `_make(iterable)` — classmethod building a fresh instance from a
        // positional-value iterable (`Match._make([a, b, size])`; CPython's
        // difflib.py uses it via `_Match.make` in SequenceMatcher.get_matching_blocks).
        {
            let make_fields = fields.clone();
            let make_type_dict = type_dict.clone();
            let make_typename = typename.clone();
            type_dict.insert_str(
                "_make",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                    let source = if args.is_empty() {
                        return Err(PyError::type_error("_make() missing required argument"));
                    } else {
                        args[0].clone()
                    };
                    let vals: Vec<PyObjectRef> =
                        if matches!(&*source.borrow(), PyObject::Tuple(_) | PyObject::List(_)) {
                            match &*source.borrow() {
                                PyObject::Tuple(t) => t.clone(),
                                PyObject::List(l) => l.clone(),
                                _ => unreachable!(),
                            }
                        } else {
                            let mut out = Vec::new();
                            let it = crate::object::builtin_iter(&[source])?;
                            let itb = it.borrow();
                            if let PyObject::Instance { .. } = &*itb {
                                loop {
                                    match crate::object::call_bound_method(
                                        itb.get_attribute("__next__")?,
                                        it.clone(),
                                        vec![],
                                    ) {
                                        Ok(v) => out.push(v),
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                            }
                            out
                        };
                    if vals.len() != make_fields.len() {
                        return Err(PyError::type_error(format!(
                            "{}._make() takes exactly {} arguments, got {}",
                            make_typename,
                            make_fields.len(),
                            vals.len()
                        )));
                    }
                    let mut new_dict = AttrMap::new();
                    new_dict.insert_str(
                        "_fields",
                        PyObjectRef::new(PyObject::List(
                            make_fields.iter().map(|f| py_str(f)).collect(),
                        )),
                    );
                    for (f, v) in make_fields.iter().zip(vals.into_iter()) {
                        new_dict.insert_str(f, v);
                    }
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: PyObjectRef::new(PyObject::Type {
                            name: make_typename.clone(),
                            dict: Box::new(str_map_to_typedict(make_type_dict.clone())),
                            bases: vec![],
                            mro: vec![],
                        }),
                        dict: new_dict,
                    }))
                }))),
            );
        }

        // Add field names as class-level attributes (for __doc__ setting support)
        for f in &fields {
            type_dict.insert(
                f.clone(),
                PyObjectRef::new(PyObject::Instance {
                    typ: PyObjectRef::new(PyObject::Type {
                        name: "member_descriptor".to_string(),
                        dict: Box::new(str_map_to_typedict(HashMap::new())),
                        bases: vec![],
                        mro: vec![],
                    }),
                    dict: AttrMap::new(),
                }),
            );
        }
        Ok(PyObjectRef::new(PyObject::Type {
            name: typename,
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        }))
    });

    // collections.abc submodule (Iterable, Hashable, etc.)
    d.insert_str(
        "abc",
        create_module("collections.abc", create_collections_abc_dict()),
    );

    d
}

/// `lru_cache`/`cache` source — see VirtualMachine::install_source_defined_stdlib.
pub const FUNCTOOLS_EXTRA_SOURCE: &str = include_str!("functools_extra.py");

/// UserList/UserDict/UserString source (like CPython's own collections.py).
/// Compiled and run once, post-construction, against the real VM — see
/// `VirtualMachine::install_collections_user_types` in vm.rs. Composition
/// over self.data works correctly for real subclassing (unlike inheriting
/// from the native list/dict/str types directly, which isn't supported).
pub const COLLECTIONS_USER_TYPES_SOURCE: &str = include_str!("collections_user_types.py");

pub fn create_functools_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ft_func {
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

    ft_func!("reduce", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("reduce() takes at least 2 arguments"));
        }
        let func = args[0].clone();
        let iterable = &args[1];
        let it = builtin_iter(&[iterable.clone()])?;
        // With an explicit `initial` (3rd positional arg), that value is
        // the starting accumulator and EVERY element of the iterable gets
        // folded in — the previous implementation always pulled the first
        // element via `next()` as `acc` regardless of whether `initial` was
        // given, silently DROPPING the initial value (and the first real
        // element never got a chance to be folded against it) whenever the
        // iterable was non-empty. Only fell back to `initial` for a truly
        // EMPTY iterable, which is a much narrower case than real Python's
        // `reduce(func, iterable, initial)` semantics. Real trigger:
        // CPython's own `Lib/statistics.py`, `reduce(_coerce, types, int)`.
        let has_initial = args.len() >= 3;
        let mut acc = if has_initial {
            args[2].clone()
        } else {
            match builtin_next(&[it.clone()]) {
                Ok(v) => v,
                Err(PyError::StopIteration) => {
                    return Err(PyError::type_error(
                        "reduce() of empty sequence with no initial value",
                    ));
                }
                Err(e) => return Err(e),
            }
        };
        loop {
            match builtin_next(&[it.clone()]) {
                Ok(v) => {
                    let result = builtin_call(&func, &[acc, v])?;
                    acc = result;
                }
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(acc)
    });

    // total_ordering: class decorator that fills in missing comparison methods
    ft_func!("total_ordering", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "total_ordering requires a class argument",
            ));
        }
        let cls = args[0].clone();
        // Collect available comparison methods
        let _has_le = cls.borrow().get_attribute("__le__").is_ok();
        let _has_lt = cls.borrow().get_attribute("__lt__").is_ok();
        let _has_ge = cls.borrow().get_attribute("__ge__").is_ok();
        let _has_gt = cls.borrow().get_attribute("__gt__").is_ok();
        let _has_eq = cls.borrow().get_attribute("__eq__").is_ok();
        // Basic stub: this doesn't implement all the methods, just returns the class
        // A real implementation would need to add __le__/__lt__/__ge__/__gt__/__eq__/__ne__
        Ok(cls)
    });

    // cached_property: descriptor that caches property value on first access
    ft_func!("cached_property", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "cached_property requires a function argument",
            ));
        }
        Ok(args[0].clone())
    });

    ft_func!("partial", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("partial() takes at least 1 argument"));
        }
        let func = args[0].clone();
        let partial_args: Vec<PyObjectRef> = args[1..].to_vec();
        Ok(PyObjectRef::new(PyObject::Partial {
            func,
            args: partial_args,
            dict: crate::object::AttrMap::new(),
        }))
    });

    // partialmethod: real semantics auto-bind `self` as the first argument
    // via the descriptor protocol when accessed on an instance. We don't
    // implement that binding here — this just pre-binds the given args like
    // partial() — so `descriptor.__get__`-based access won't insert self.
    // Direct calls (e.g. `SomeClass.attr(instance, ...)`) still work.
    ft_func!("partialmethod", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "partialmethod() takes at least 1 argument",
            ));
        }
        let func = args[0].clone();
        let partial_args: Vec<PyObjectRef> = args[1..].to_vec();
        Ok(PyObjectRef::new(PyObject::Partial {
            func,
            args: partial_args,
            dict: crate::object::AttrMap::new(),
        }))
    });

    ft_func!("update_wrapper", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "update_wrapper() requires at least 2 arguments",
            ));
        }
        let wrapper = args[0].clone();
        let wrapped = args[1].clone();
        let attrs = [
            "__module__",
            "__name__",
            "__qualname__",
            "__doc__",
            "__annotations__",
            "__dict__",
        ];
        for attr in &attrs {
            if let Ok(val) = wrapped.borrow().get_attribute(attr) {
                let _ = wrapper.borrow_mut().set_attribute(attr, val);
            }
        }
        let _ = wrapper
            .borrow_mut()
            .set_attribute("__wrapped__", wrapped.clone());
        for attr in &["__defaults__", "__kwdefaults__", "__code__", "__globals__"] {
            if let Ok(val) = wrapped.borrow().get_attribute(attr) {
                let _ = wrapper.borrow_mut().set_attribute(attr, val);
            }
        }
        Ok(wrapper)
    });
    // `functools.WRAPPER_ASSIGNMENTS`/`WRAPPER_UPDATES` — the attribute
    // lists `functools.wraps` copies (test_reprlib imports the former).
    d.insert_str(
        "WRAPPER_ASSIGNMENTS",
        py_tuple(vec![
            py_str("__module__"),
            py_str("__name__"),
            py_str("__qualname__"),
            py_str("__annotations__"),
            py_str("__doc__"),
        ]),
    );
    d.insert_str("WRAPPER_UPDATES", py_tuple(vec![py_str("__dict__")]));
    ft_func!("wraps", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("wraps() requires at least 1 argument"));
        }
        let wrapped = args[0].clone();
        let wrapped_clone = wrapped.clone();
        let decorator = move |inner_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if inner_args.is_empty() {
                return Err(PyError::type_error("wraps() decorator requires 1 argument"));
            }
            let wrapper_fn = inner_args[0].clone();
            let attrs = [
                "__module__",
                "__name__",
                "__qualname__",
                "__doc__",
                "__annotations__",
                "__dict__",
            ];
            for attr in &attrs {
                if let Ok(val) = wrapped_clone.borrow().get_attribute(attr) {
                    let _ = wrapper_fn.borrow_mut().set_attribute(attr, val);
                }
            }
            let _ = wrapper_fn
                .borrow_mut()
                .set_attribute("__wrapped__", wrapped_clone.clone());
            Ok(wrapper_fn)
        };
        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(decorator))))
    });
    // lru_cache/cache: real implementations installed as Python source —
    // see VirtualMachine::new_with_args's install_source_defined_stdlib
    // call and functools_extra.py. A wrapper needs to support the
    // descriptor protocol (__get__, for correct method binding) and expose
    // cache_clear()/cache_info(), neither of which a bare Rust closure can
    // hold (PyObject::Closure has no attribute storage).

    // singledispatch: generic function dispatcher
    // Used by pkgutil, among others
    ft_func!("singledispatch", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "singledispatch() requires at least 1 argument",
            ));
        }
        let func = args[0].clone();
        let registry = Rc::new(std::cell::RefCell::new(std::collections::HashMap::<
            String,
            PyObjectRef,
        >::new()));
        {
            let mut reg = registry.borrow_mut();
            reg.insert_str("object", func.clone());
        }
        let func_name = func.borrow().get_attribute("__name__").ok();
        let registry_clone = registry.clone();
        let dispatch_func = move |call_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if call_args.is_empty() {
                return Err(PyError::type_error(
                    "singledispatch requires at least 1 argument",
                ));
            }
            let first_arg = &call_args[0];
            let arg_type = first_arg.borrow().type_name();
            let reg = registry_clone.borrow();
            let impl_func = reg
                .get(&arg_type)
                .or_else(|| reg.get("object"))
                .cloned()
                .ok_or_else(|| PyError::runtime_error("singledispatch: no implementation found"))?;
            builtin_call(&impl_func, call_args)
        };
        // Use Instance with __call__ so set_attribute works (Closure doesn't support attribute setting)
        let mut call_type_dict = HashMap::new();
        let dispatch_rc = Rc::new(dispatch_func);
        call_type_dict.insert_str(
            "__call__",
            PyObjectRef::new(PyObject::Closure(Rc::new(
                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> { dispatch_rc(args) },
            ))),
        );
        let dispatcher = PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "singledispatch".to_string(),
                dict: Box::new(str_map_to_typedict(call_type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(), // attributes like .register, .registry go here
        });
        {
            let mut py_registry = PyDict::new();
            let reg = registry.borrow();
            for (type_name, impl_func) in reg.iter() {
                py_registry.set(py_str(type_name), impl_func.clone()).ok();
            }
            let _ = dispatcher.borrow_mut().set_attribute(
                "registry",
                PyObjectRef::new(PyObject::Dict(Box::new(py_registry))),
            );
        }
        let reg_register = registry.clone();
        let _dispatch_clone = dispatcher.clone();
        let register_method = move |m_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if m_args.is_empty() {
                return Err(PyError::type_error(
                    "register() requires at least 1 argument",
                ));
            }
            let typ_arg = m_args[0].clone();
            let type_name = typ_arg.borrow().type_name();
            let type_key = if type_name == "type" {
                typ_arg
                    .borrow()
                    .get_attribute("__name__")
                    .map(|n| n.str())
                    .unwrap_or_else(|_| type_name.clone())
            } else {
                type_name.clone()
            };
            if m_args.len() >= 2 {
                reg_register
                    .borrow_mut()
                    .insert(type_key, m_args[1].clone());
                Ok(py_none())
            } else {
                let reg_register_clone = reg_register.clone();
                let decorator = move |d_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if d_args.is_empty() {
                        return Err(PyError::type_error(
                            "register decorator requires a function argument",
                        ));
                    }
                    reg_register_clone
                        .borrow_mut()
                        .insert(type_key.clone(), d_args[0].clone());
                    Ok(d_args[0].clone())
                };
                Ok(PyObjectRef::new(PyObject::Closure(Rc::new(decorator))))
            }
        };
        let _ = dispatcher.borrow_mut().set_attribute(
            "register",
            PyObjectRef::new(PyObject::Closure(Rc::new(register_method))),
        );
        if let Some(name) = func_name {
            let _ = dispatcher.borrow_mut().set_attribute("__name__", name);
        }
        let _ = dispatcher.borrow_mut().set_attribute("__wrapped__", func);
        Ok(dispatcher)
    });

    // cmp_to_key: convert old-style comparison function to a key class for sorted()/min()/max()
    ft_func!("cmp_to_key", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "cmp_to_key requires at least 1 argument",
            ));
        }
        let mycmp = args[0].clone();
        let mycmp_for_factory = mycmp.clone();
        // Return a callable that acts as the key class
        let key_factory = move |k_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if k_args.is_empty() {
                return Err(PyError::type_error(
                    "cmp_to_key() key factory missing required argument",
                ));
            }
            let obj = k_args[0].clone();
            let mycmp_rc = std::rc::Rc::new(mycmp_for_factory.clone());
            let obj_rc = std::rc::Rc::new(obj);

            // __lt__(self, other): mycmp(self.obj, other.obj) < 0
            let lt_mycmp = mycmp_rc.clone();
            let lt_obj = obj_rc.clone();
            let lt = move |lt_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if lt_args.len() < 2 {
                    return Err(PyError::type_error("__lt__ requires 2 arguments"));
                }
                // `other` is a Kobj wrapper — compare its `.obj`, not the
                // wrapper itself (real CPython's cmp_to_key: mycmp(self.obj,
                // other.obj)).
                let other_obj = lt_args[1]
                    .borrow()
                    .get_attribute("obj")
                    .unwrap_or_else(|_| lt_args[1].clone());
                let cmp_result = builtin_call(&lt_mycmp, &[(*lt_obj).clone(), other_obj])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n < 0)))
            };

            // __le__(self, other): mycmp(self.obj, other.obj) <= 0
            let le_mycmp = mycmp_rc.clone();
            let le_obj = obj_rc.clone();
            let le = move |le_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if le_args.len() < 2 {
                    return Err(PyError::type_error("__le__ requires 2 arguments"));
                }
                let other_obj = le_args[1]
                    .borrow()
                    .get_attribute("obj")
                    .unwrap_or_else(|_| le_args[1].clone());
                let cmp_result = builtin_call(&le_mycmp, &[(*le_obj).clone(), other_obj])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n <= 0)))
            };

            // __gt__(self, other): mycmp(self.obj, other.obj) > 0
            let gt_mycmp = mycmp_rc.clone();
            let gt_obj = obj_rc.clone();
            let gt = move |gt_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if gt_args.len() < 2 {
                    return Err(PyError::type_error("__gt__ requires 2 arguments"));
                }
                let other_obj = gt_args[1]
                    .borrow()
                    .get_attribute("obj")
                    .unwrap_or_else(|_| gt_args[1].clone());
                let cmp_result = builtin_call(&gt_mycmp, &[(*gt_obj).clone(), other_obj])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n > 0)))
            };

            // __ge__(self, other): mycmp(self.obj, other.obj) >= 0
            let ge_mycmp = mycmp_rc.clone();
            let ge_obj = obj_rc.clone();
            let ge = move |ge_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if ge_args.len() < 2 {
                    return Err(PyError::type_error("__ge__ requires 2 arguments"));
                }
                let other_obj = ge_args[1]
                    .borrow()
                    .get_attribute("obj")
                    .unwrap_or_else(|_| ge_args[1].clone());
                let cmp_result = builtin_call(&ge_mycmp, &[(*ge_obj).clone(), other_obj])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n >= 0)))
            };

            // __eq__(self, other): mycmp(self.obj, other.obj) == 0
            let eq_mycmp = mycmp_rc.clone();
            let eq_obj = obj_rc.clone();
            let eq = move |eq_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if eq_args.len() < 2 {
                    return Err(PyError::type_error("__eq__ requires 2 arguments"));
                }
                let other_obj = eq_args[1]
                    .borrow()
                    .get_attribute("obj")
                    .unwrap_or_else(|_| eq_args[1].clone());
                let cmp_result = builtin_call(&eq_mycmp, &[(*eq_obj).clone(), other_obj])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n == 0)))
            };

            // __ne__(self, other): mycmp(self.obj, other.obj) != 0
            let ne_mycmp = mycmp_rc.clone();
            let ne_obj = obj_rc.clone();
            let ne = move |ne_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if ne_args.len() < 2 {
                    return Err(PyError::type_error("__ne__ requires 2 arguments"));
                }
                let other_obj = ne_args[1]
                    .borrow()
                    .get_attribute("obj")
                    .unwrap_or_else(|_| ne_args[1].clone());
                let cmp_result = builtin_call(&ne_mycmp, &[(*ne_obj).clone(), other_obj])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n != 0)))
            };

            // __hash__: cmp_to_key objects are unhashable (comparison may not be consistent)
            let hash_err = |_: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                Err(PyError::type_error(
                    "comparison function yields unhashable object",
                ))
            };

            let mut type_dict = std::collections::HashMap::new();
            type_dict.insert_str(
                "__lt__",
                PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(lt))),
            );
            type_dict.insert_str(
                "__le__",
                PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(le))),
            );
            type_dict.insert_str(
                "__gt__",
                PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(gt))),
            );
            type_dict.insert_str(
                "__ge__",
                PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(ge))),
            );
            type_dict.insert_str(
                "__eq__",
                PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(eq))),
            );
            type_dict.insert_str(
                "__ne__",
                PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(ne))),
            );
            type_dict.insert_str(
                "__hash__",
                PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(hash_err))),
            );

            let key_obj = PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "cmp_to_key".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: AttrMap::new(),
            });
            let _ = key_obj
                .borrow_mut()
                .set_attribute("obj", obj_rc.as_ref().clone());
            Ok(key_obj)
        };
        Ok(PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
            key_factory,
        ))))
    });

    d
}

pub fn create_itertools_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! it_func {
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

    // chain is represented as a callable Instance (not a bare
    // BuiltinFunction) so it can also expose `chain.from_iterable(...)` —
    // BuiltinFunction has no attribute storage at all (set_attribute has no
    // arm for it), so a plain function couldn't hold a from_iterable
    // sibling method the way real itertools.chain does.
    {
        let mut chain_type_dict = HashMap::new();
        chain_type_dict.insert_str(
            "__call__",
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    // vm.call_function's `__call__` dispatch always prepends self
                    // (matching a real Python `__call__(self, *args)` method) before
                    // calling whatever `__call__` resolves to — unlike attribute
                    // access via LOAD_ATTR, which does NOT auto-bind a bare Closure.
                    // args[0] here is the chain instance itself; skip it.
                    let mut items = Vec::new();
                    for arg in args.iter().skip(1) {
                        if let Ok(it) = builtin_iter(&[arg.clone()]) {
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => items.push(v),
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                    Ok(py_list(items))
                },
            ))),
        );
        chain_type_dict.insert_str(
            "from_iterable",
            PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if args.is_empty() {
                        return Err(PyError::type_error("from_iterable() missing argument"));
                    }
                    let mut items = Vec::new();
                    if let Ok(outer_it) = builtin_iter(&[args[0].clone()]) {
                        loop {
                            match builtin_next(&[outer_it.clone()]) {
                                Ok(inner) => {
                                    if let Ok(inner_it) = builtin_iter(&[inner]) {
                                        loop {
                                            match builtin_next(&[inner_it.clone()]) {
                                                Ok(v) => items.push(v),
                                                Err(PyError::StopIteration) => break,
                                                Err(e) => return Err(e),
                                            }
                                        }
                                    }
                                }
                                Err(PyError::StopIteration) => break,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    Ok(py_list(items))
                },
            ))),
        );
        let chain_type = PyObjectRef::new(PyObject::Type {
            name: "chain".to_string(),
            dict: Box::new(str_map_to_typedict(chain_type_dict)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str(
            "chain",
            PyObjectRef::new(PyObject::Instance {
                typ: chain_type,
                dict: AttrMap::new(),
            }),
        );
    }

    it_func!("count", |args| {
        let start = if args.len() > 0 {
            if let Some(n) = args[0].as_i64() {
                n
            } else {
                0i64
            }
        } else {
            0i64
        };
        let step = if args.len() > 1 {
            if let Some(n) = args[1].as_i64() {
                n
            } else {
                1i64
            }
        } else {
            1i64
        };
        let mut current = start;
        let mut items = Vec::new();
        for _ in 0..10000 {
            items.push(py_int(current));
            current += step;
        }
        Ok(py_list(items))
    });

    // `itertools.cycle(iterable)` was missing entirely — unlike this
    // file's other itertools functions (`count`/`repeat`/etc.), which
    // approximate "infinite" by eagerly materializing a large-but-bounded
    // number of items, `cycle` gets a REAL lazy iterator (`PyObject::
    // CycleIter`, `object.rs`) since eager materialization is simply
    // impossible for something with no natural cutoff at all — real code
    // commonly relies on `cycle()` running genuinely forever (e.g. paired
    // with `itertools.islice` to take just the first N, or driven by an
    // external `break`).
    it_func!("cycle", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("cycle() missing required argument"));
        }
        let it = builtin_iter(&[args[0].clone()])?;
        let mut items = Vec::new();
        loop {
            match builtin_next(&[it.clone()]) {
                Ok(v) => items.push(v),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(PyObjectRef::new(PyObject::CycleIter { items, index: 0 }))
    });

    it_func!("product", |args| {
        let mut args: Vec<PyObjectRef> = args.to_vec();
        let mut repeat = 1;
        if let Some(last) = args.last().map(|a| a.clone()) {
            let is_dict = matches!(&*last.borrow(), PyObject::Dict(_));
            if is_dict {
                if let PyObject::Dict(dict) = &*last.borrow() {
                    if let Ok(Some(r)) = dict.get(&crate::object::py_str("repeat")) {
                        repeat = r
                            .as_i64()
                            .ok_or_else(|| PyError::type_error("repeat must be int"))?
                            as usize;
                    }
                }
                args.pop();
            }
        }
        if args.is_empty() || repeat == 0 {
            return Ok(py_list(vec![py_tuple(vec![])]));
        }
        let mut pools: Vec<Vec<PyObjectRef>> = Vec::new();
        for _ in 0..repeat {
            for arg in &args {
                let mut pool = Vec::new();
                if let Ok(it) = builtin_iter(&[arg.clone()]) {
                    loop {
                        match builtin_next(&[it.clone()]) {
                            Ok(v) => pool.push(v),
                            Err(PyError::StopIteration) => break,
                            Err(e) => return Err(e),
                        }
                    }
                }
                pools.push(pool);
            }
        }
        let mut result = vec![vec![]];
        for pool in &pools {
            let mut new_result = Vec::new();
            for prefix in &result {
                for item in pool {
                    let mut new_prefix = prefix.clone();
                    new_prefix.push(item.clone());
                    new_result.push(new_prefix);
                }
            }
            result = new_result;
        }
        Ok(py_list(result.into_iter().map(|v| py_tuple(v)).collect()))
    });

    it_func!("combinations", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("combinations() missing argument"));
        }
        let mut pool = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => pool.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let n = pool.len();
        let r = if args.len() > 1 {
            args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("r must be int"))? as usize
        } else {
            n
        };
        let mut result = Vec::new();
        if r <= n {
            let mut indices: Vec<usize> = (0..r).collect();
            loop {
                result.push(py_tuple(indices.iter().map(|&i| pool[i].clone()).collect()));
                let mut i = r;
                loop {
                    if i == 0 {
                        return Ok(py_list(result));
                    }
                    i -= 1;
                    if indices[i] != i + n - r {
                        break;
                    }
                    if i == 0 {
                        return Ok(py_list(result));
                    }
                }
                indices[i] += 1;
                for j in i + 1..r {
                    indices[j] = indices[j - 1] + 1;
                }
            }
        }
        Ok(py_list(result))
    });

    it_func!("combinations_with_replacement", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "combinations_with_replacement() missing argument",
            ));
        }
        let mut pool = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => pool.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let n = pool.len();
        let r = if args.len() > 1 {
            args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("r must be int"))? as usize
        } else {
            n
        };
        let mut result = Vec::new();
        if n > 0 || r == 0 {
            let mut indices = vec![0usize; r];
            loop {
                result.push(py_tuple(indices.iter().map(|&i| pool[i].clone()).collect()));
                let mut i_opt = None;
                for i in (0..r).rev() {
                    if indices[i] != n - 1 {
                        i_opt = Some(i);
                        break;
                    }
                }
                match i_opt {
                    None => break,
                    Some(i) => {
                        let v = indices[i] + 1;
                        for j in i..r {
                            indices[j] = v;
                        }
                    }
                }
            }
        }
        Ok(py_list(result))
    });

    it_func!("permutations", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("permutations() missing argument"));
        }
        let mut pool = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => pool.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let n = pool.len();
        let r = if args.len() > 1 && !matches!(&*args[1].borrow(), PyObject::None) {
            args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("r must be int"))? as usize
        } else {
            n
        };
        let mut result = Vec::new();
        if r <= n {
            let mut indices: Vec<usize> = (0..n).collect();
            let mut cycles: Vec<usize> = (0..r).map(|i| n - i).collect();
            result.push(py_tuple(
                indices[0..r].iter().map(|&i| pool[i].clone()).collect(),
            ));
            'outer: loop {
                let mut i = r;
                loop {
                    if i == 0 {
                        break 'outer;
                    }
                    i -= 1;
                    cycles[i] -= 1;
                    if cycles[i] == 0 {
                        let first = indices[i];
                        for k in i..n - 1 {
                            indices[k] = indices[k + 1];
                        }
                        indices[n - 1] = first;
                        cycles[i] = n - i;
                    } else {
                        let j = n - cycles[i];
                        indices.swap(i, j);
                        result.push(py_tuple(
                            indices[0..r].iter().map(|&i| pool[i].clone()).collect(),
                        ));
                        continue 'outer;
                    }
                    if i == 0 {
                        break 'outer;
                    }
                }
            }
        }
        Ok(py_list(result))
    });

    it_func!("repeat", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("repeat() missing argument"));
        }
        let obj = args[0].clone();
        // `None` distinguishes "no count given" (real infinite repeat) from
        // an explicit `times=0` (a real, valid call meaning "repeat zero
        // times" — an empty iterator) — these used to collapse onto the
        // same `0` sentinel, so `itertools.repeat(x, 0)` wrongly produced
        // 1000 items instead of none.
        let times: Option<usize> = if args.len() > 1 {
            let n = args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("times must be int"))?;
            Some(n.max(0) as usize)
        } else {
            None
        };
        // Cap materialization regardless of the requested count — this
        // itertools implementation is eager (builds a real list), not a
        // true lazy iterator, so an astronomically large explicit count
        // (a common real-world test pattern like `repeat(x, sys.maxsize)`
        // combined with `islice` to only ever pull a few items, relying on
        // real itertools' laziness to never actually materialize the rest)
        // would otherwise try to allocate a vector sized by that count
        // directly, crashing with a Rust allocator "capacity overflow"
        // panic instead of a graceful Python-level result. Real trigger:
        // CPython's own `test_itertools.py`.
        const MAX_MATERIALIZED: usize = 100_000;
        let n = times.unwrap_or(1000).min(MAX_MATERIALIZED);
        let mut items = Vec::with_capacity(n);
        for _ in 0..n {
            items.push(obj.clone());
        }
        Ok(py_list(items))
    });

    // `islice(iterable, [start,] stop[, step])` — its ENTIRE reason to
    // exist in real Python is slicing a bound out of a POTENTIALLY INFINITE
    // iterator (`itertools.count()`, `itertools.cycle()`, a hand-written
    // infinite generator) without ever materializing it in full. The
    // previous implementation eagerly drained the WHOLE input into a `Vec`
    // BEFORE looking at `start`/`stop`/`step` at all — hung forever on any
    // genuinely infinite source (confirmed via the simplest repro,
    // `list(itertools.islice(itertools.cycle('ab'), 5))`). Fixed to pull at
    // most `stop` items from the source lazily, stopping as soon as enough
    // have been read — matching real `islice`'s whole purpose. A `stop`
    // of `None` (real Python's "take everything from `start` onward," only
    // meaningful for a source that eventually ends on its own) still drains
    // to real exhaustion, same as before — that's correct there, not a bug.
    it_func!("islice", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("islice() missing arguments"));
        }
        let (start, stop, step) = match args.len() {
            1 => return Err(PyError::type_error("islice() missing stop argument")),
            2 => (
                0i64,
                if matches!(&*args[1].borrow(), PyObject::None) {
                    None
                } else {
                    Some(args[1].as_i64().unwrap_or(0))
                },
                1i64,
            ),
            _ => {
                let start = args[1].as_i64().unwrap_or(0);
                let stop = if matches!(&*args[2].borrow(), PyObject::None) {
                    None
                } else {
                    Some(args[2].as_i64().unwrap_or(0))
                };
                let step = if args.len() > 3 {
                    args[3].as_i64().unwrap_or(1)
                } else {
                    1
                };
                (start, stop, step)
            }
        };
        let start = start.max(0);
        let step = step.max(1);
        let it = builtin_iter(&[args[0].clone()])?;
        let mut result = Vec::new();
        let mut i: i64 = 0;
        loop {
            if let Some(stop_v) = stop {
                if i >= stop_v {
                    break;
                }
            }
            match builtin_next(&[it.clone()]) {
                Ok(v) => {
                    if i >= start && (i - start) % step == 0 {
                        result.push(v);
                    }
                    i += 1;
                }
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(py_list(result))
    });

    it_func!("tee", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("tee() missing argument"));
        }
        let n = if args.len() > 1 {
            args[1].as_i64().unwrap_or(2) as usize
        } else {
            2
        };
        let mut items = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => items.push(v),
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let mut tees = Vec::with_capacity(n);
        for _ in 0..n {
            let it = builtin_iter(&[py_list(items.clone())])?;
            tees.push(it);
        }
        Ok(py_tuple(tees))
    });

    it_func!("zip_longest", |args| {
        let mut fillvalue = py_none();
        let mut iterables = args;
        if let Some(last) = iterables.last() {
            if let PyObject::Dict(d) = &*last.borrow() {
                if let Ok(Some(v)) = d.get(&py_str("fillvalue")) {
                    fillvalue = v;
                }
                iterables = &iterables[..iterables.len() - 1];
            }
        }
        let mut lists: Vec<Vec<PyObjectRef>> = Vec::new();
        for arg in iterables {
            let mut items = Vec::new();
            if let Ok(it) = builtin_iter(&[arg.clone()]) {
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(v) => items.push(v),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            lists.push(items);
        }
        let max_len = lists.iter().map(|l| l.len()).max().unwrap_or(0);
        let mut result = Vec::with_capacity(max_len);
        for i in 0..max_len {
            let row: Vec<PyObjectRef> = lists
                .iter()
                .map(|l| l.get(i).cloned().unwrap_or_else(|| fillvalue.clone()))
                .collect();
            result.push(py_tuple(row));
        }
        Ok(py_list(result))
    });

    it_func!("accumulate", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("accumulate() missing argument"));
        }
        let mut items = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            let mut total: Option<i64> = None;
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => {
                        if let Some(n) = v.as_i64() {
                            total = Some(total.unwrap_or(0) + n);
                            items.push(py_int(total.unwrap()));
                        } else {
                            items.push(v);
                        }
                    }
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(py_list(items))
    });

    // groupby(iterable, key=None) — groups consecutive elements sharing the
    // same key. Constructs a real, lazy `PyObject::GroupByIter` (see its
    // own doc comment in `object.rs` for why this MUST be lazy — an
    // earlier eager version crashed on CPython's own
    // `test_groupby_reentrant_eq_does_not_crash`, gh-143543); the actual
    // per-`next()` state machine lives in `builtin_next`'s dedicated
    // `GroupByIter` handling.
    it_func!("groupby", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("groupby() missing argument"));
        }
        // The key function may arrive positionally (args[1]) or as a
        // trailing kwargs dict (`key=...`) per this project's established
        // calling convention (see e.g. `str.format`'s own doc comment).
        let mut key_func: Option<PyObjectRef> = None;
        if args.len() > 1 {
            let last = &args[args.len() - 1];
            if let PyObject::Dict(d) = &*last.borrow() {
                if let Ok(Some(k)) = d.get(&py_str("key")) {
                    if !matches!(&*k.borrow(), PyObject::None) {
                        key_func = Some(k);
                    }
                }
            } else if !matches!(&*last.borrow(), PyObject::None) {
                key_func = Some(last.clone());
            }
        }
        let source = builtin_iter(&[args[0].clone()])?;
        Ok(PyObjectRef::new(PyObject::GroupByIter {
            source,
            key_func,
            pending: None,
            exhausted: false,
        }))
    });

    // filterfalse(func, iterable) — filter elements where func is False
    it_func!("filterfalse", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("filterfalse() requires 2 arguments"));
        }
        let predicate = if matches!(&*args[0].borrow(), PyObject::None) {
            None
        } else {
            Some(args[0].clone())
        };
        let iterable = args[1].clone();
        let mut result = Vec::new();
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(item) => {
                    let should_keep = match &predicate {
                        Some(f) => {
                            let callable = PyObjectRef::imm(PyObject::BoundMethod {
                                func: f.clone(),
                                self_obj: py_none(),
                            });
                            let mut vm = crate::vm::VirtualMachine::new();
                            match vm.call_function(callable, vec![item.clone()], vec![]) {
                                Ok(val) => !val.truthy(),
                                Err(_) => true,
                            }
                        }
                        None => !item.truthy(),
                    };
                    if should_keep {
                        result.push(item);
                    }
                }
                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(py_list(result))
    });

    d
}

pub fn create_statistics_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! stat_func {
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

    // Extract numeric values from ANY iterable (not just a literal `list` —
    // real `statistics` functions accept tuples, generators, `range`, etc.)
    // via `collect_iterable`, converting each element through `builtin_float`
    // (the same general `__float__`-dispatch machinery `float()` itself
    // uses) rather than hand-matching only `PyObject::Int`/`Float` — this
    // means `Fraction`/`Decimal`/`bool`/any custom class implementing
    // `__float__` all work, not just plain int/float literals. Previously
    // EVERY statistics function required a literal `list` argument (raising
    // "argument must be a list" for a tuple, generator, or `Fraction`-
    // containing sequence) — found via CPython's own `test_statistics.py`,
    // whose shared `NumericTestCase`-style mixin tests exercise exactly
    // these argument shapes across `TestMean`/`TestMedian`/`TestStdev`/etc.
    fn stat_extract_nums(data: &PyObjectRef) -> PyResult<Vec<f64>> {
        let items = crate::object::collect_iterable(data)?;
        if items.is_empty() {
            return Err(PyError::ValueError("argument is empty".to_string()));
        }
        let mut nums: Vec<f64> = Vec::with_capacity(items.len());
        for item in &items {
            let f = builtin_float(std::slice::from_ref(item))
                .map_err(|_| PyError::type_error("argument must contain numbers"))?;
            let borrowed = f.borrow();
            match &*borrowed {
                PyObject::Float(v) => nums.push(*v),
                _ => return Err(PyError::type_error("argument must contain numbers")),
            }
        }
        Ok(nums)
    }

    stat_func!("mean", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("mean() missing required argument"));
        }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => PyError::ValueError("mean() argument is empty".to_string()),
            PyError::TypeError(_) => PyError::type_error("mean() argument must contain numbers"),
            other => other,
        })?;
        let n = nums.len() as f64;
        let sum: f64 = nums.iter().sum();
        Ok(py_float(sum / n))
    });

    stat_func!("median", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("median() missing required argument"));
        }
        let mut nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => PyError::ValueError("median() argument is empty".to_string()),
            PyError::TypeError(_) => PyError::type_error("median() argument must contain numbers"),
            other => other,
        })?;
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        if n % 2 == 0 {
            Ok(py_float((nums[n / 2 - 1] + nums[n / 2]) / 2.0))
        } else {
            Ok(py_float(nums[n / 2]))
        }
    });

    stat_func!("stdev", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("stdev() missing required argument"));
        }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::TypeError(_) => PyError::type_error("stdev() argument must contain numbers"),
            other => other,
        })?;
        if nums.len() < 2 {
            return Err(PyError::ValueError(
                "stdev() requires at least 2 data points".to_string(),
            ));
        }
        let n = nums.len() as f64;
        let sum: f64 = nums.iter().sum();
        let mean = sum / n;
        let variance: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
        Ok(py_float(variance.sqrt()))
    });

    // `statistics.harmonic_mean` was missing entirely — the harmonic mean
    // is `n / sum(1/x for x in data)`, undefined (real CPython raises
    // `StatisticsError`, mapped to `ValueError` here matching the other
    // stats functions' convention) if any element is zero.
    stat_func!("harmonic_mean", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "harmonic_mean() missing required argument",
            ));
        }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => {
                PyError::ValueError("harmonic_mean() argument is empty".to_string())
            }
            PyError::TypeError(_) => {
                PyError::type_error("harmonic_mean() argument must contain numbers")
            }
            other => other,
        })?;
        if nums.iter().any(|&x| x < 0.0) {
            return Err(PyError::ValueError(
                "harmonic_mean() does not support negative values".to_string(),
            ));
        }
        if nums.iter().any(|&x| x == 0.0) {
            return Ok(py_float(0.0));
        }
        let n = nums.len() as f64;
        let recip_sum: f64 = nums.iter().map(|x| 1.0 / x).sum();
        Ok(py_float(n / recip_sum))
    });

    stat_func!("mode", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("mode() missing required argument"));
        }
        let items = crate::object::collect_iterable(&args[0])?;
        if items.is_empty() {
            return Err(PyError::ValueError("mode() argument is empty".to_string()));
        }
        let mut counts = std::collections::HashMap::new();
        let mut max_count = 0i64;
        let mut modes: Vec<PyObjectRef> = Vec::new();
        for item in &items {
            let hash = item.hash()?;
            let entry = counts.entry(hash).or_insert((0i64, item.clone()));
            entry.0 += 1;
        }
        // Find the max count
        for (_, (count, ref item)) in &counts {
            if *count > max_count {
                max_count = *count;
                modes.clear();
                modes.push(item.clone());
            } else if *count == max_count {
                modes.push(item.clone());
            }
        }
        if modes.len() == 1 {
            Ok(modes[0].clone())
        } else {
            Ok(py_list(modes))
        }
    });

    stat_func!("median_low", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "median_low() missing required argument",
            ));
        }
        let mut nums = stat_extract_nums(&args[0])?;
        if nums.is_empty() {
            return Err(PyError::ValueError(
                "median_low() argument is empty".to_string(),
            ));
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        Ok(py_float(nums[(n - 1) / 2]))
    });

    stat_func!("median_high", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "median_high() missing required argument",
            ));
        }
        let mut nums = stat_extract_nums(&args[0])?;
        if nums.is_empty() {
            return Err(PyError::ValueError(
                "median_high() argument is empty".to_string(),
            ));
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        Ok(py_float(nums[n / 2]))
    });

    // `statistics.__all__` — same fix, same reason, as `operator.__all__`
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

// ===================== Real decimal.Decimal =====================
//
// Arbitrary-precision decimal arithmetic per (a practical subset of) IBM's
// General Decimal Arithmetic Specification, the same spec CPython's own
// `decimal` module follows. A Decimal value is sign/coefficient/exponent
// (or one of the special states NaN/sNaN/Infinity); the coefficient is a
// `BigInt` so precision is genuinely unbounded, matching real semantics
// (unlike the previous stub, which just wrapped the constructor argument in
// a string with no arithmetic at all).
//
// Scope: construction (str/int/float/Decimal/tuple), correct string
// formatting, +-*/ (with context precision/rounding), //, %, **  for integer
// exponents, unary -/+/abs, comparisons, a usable (if approximate) hash,
// quantize/normalize/as_tuple/is_*, and a Context type with
// getcontext/setcontext/localcontext. Not implemented: exp/ln/log10/sqrt,
// non-integer power, signal traps/flags (rounding happens silently, as if
// no traps were enabled — only InvalidOperation/DivisionByZero on truly
// undefined operations actually raise).

#[derive(Clone, PartialEq, Debug)]
enum DecSpecial {
    Finite,
    QNaN,
    SNaN,
    Infinity,
}

#[derive(Clone, Debug)]
struct DecValue {
    special: DecSpecial,
    sign: bool,                // true = negative
    coeff: num_bigint::BigInt, // non-negative significand; 0 for NaN/Infinity
    exp: i64,                  // meaningless for NaN/Infinity
}

impl DecValue {
    fn zero() -> Self {
        DecValue {
            special: DecSpecial::Finite,
            sign: false,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        }
    }
    fn nan() -> Self {
        DecValue {
            special: DecSpecial::QNaN,
            sign: false,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        }
    }
    fn infinity(sign: bool) -> Self {
        DecValue {
            special: DecSpecial::Infinity,
            sign,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        }
    }
    fn is_zero(&self) -> bool {
        self.special == DecSpecial::Finite && num_traits::Zero::is_zero(&self.coeff)
    }
    fn is_nan(&self) -> bool {
        matches!(self.special, DecSpecial::QNaN | DecSpecial::SNaN)
    }
}

fn parse_decimal_str(raw: &str) -> Option<DecValue> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let mut sign = false;
    let rest = if let Some(r) = s.strip_prefix('+') {
        r
    } else if let Some(r) = s.strip_prefix('-') {
        sign = true;
        r
    } else {
        s
    };
    if rest.is_empty() {
        return None;
    }
    let rest_lower = rest.to_ascii_lowercase();
    if rest_lower == "inf" || rest_lower == "infinity" {
        return Some(DecValue::infinity(sign));
    }
    if let Some(digits_part) = rest_lower.strip_prefix("snan") {
        let coeff = if digits_part.is_empty() {
            num_bigint::BigInt::from(0)
        } else {
            num_bigint::BigInt::parse_bytes(digits_part.as_bytes(), 10)?
        };
        return Some(DecValue {
            special: DecSpecial::SNaN,
            sign,
            coeff,
            exp: 0,
        });
    }
    if let Some(digits_part) = rest_lower.strip_prefix("nan") {
        let coeff = if digits_part.is_empty() {
            num_bigint::BigInt::from(0)
        } else {
            num_bigint::BigInt::parse_bytes(digits_part.as_bytes(), 10)?
        };
        return Some(DecValue {
            special: DecSpecial::QNaN,
            sign,
            coeff,
            exp: 0,
        });
    }
    let (mantissa_part, exp_part) = match rest.find(['e', 'E']) {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
        None => (rest, None),
    };
    if mantissa_part.is_empty() {
        return None;
    }
    let (int_part, frac_part) = match mantissa_part.find('.') {
        Some(idx) => (&mantissa_part[..idx], &mantissa_part[idx + 1..]),
        None => (mantissa_part, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits_str = format!("{}{}", int_part, frac_part);
    let digits_str = if digits_str.is_empty() {
        "0".to_string()
    } else {
        digits_str
    };
    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10)?;
    let mut exp: i64 = -(frac_part.len() as i64);
    if let Some(exp_str) = exp_part {
        let exp_str = exp_str.trim();
        if exp_str.is_empty() {
            return None;
        }
        let extra: i64 = exp_str.parse().ok()?;
        exp += extra;
    }
    Some(DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff,
        exp,
    })
}

/// Largest `k` such that `b^k` divides `n` (for prime `b`).
fn factor_power_of(n: &num_bigint::BigUint, b: u8) -> u32 {
    let mut v = n.clone();
    let mut k = 0u32;
    while &v % num_bigint::BigUint::from(b) == num_bigint::BigUint::from(0u8) {
        v /= num_bigint::BigUint::from(b);
        k += 1;
    }
    k
}

fn decval_from_f64(f: f64) -> DecValue {
    float_to_decvalue(f)
}

/// The EXACT decimal value of an f64 (CPython's `Decimal(float)` and
/// `Decimal.from_float(f)` both produce the exact binary value, not the
/// shortest repr): `m * 2**e` written as `m * 5**k / 10**k`, normalized by
/// removing trailing factors of 10.
fn float_to_decvalue(f: f64) -> DecValue {
    if f.is_nan() {
        return DecValue::nan();
    }
    if f.is_infinite() {
        return DecValue::infinity(f < 0.0);
    }
    if f == 0.0 {
        return DecValue {
            special: DecSpecial::Finite,
            sign: f.is_sign_negative(),
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        };
    }
    let bits = f.to_bits();
    let sign = (bits >> 63) != 0;
    let biased = ((bits >> 52) & 0x7ff) as i64;
    let mantissa = bits & 0x000f_ffff_ffff_ffff;
    let (m, e) = if biased == 0 {
        (mantissa, -1074i64)
    } else {
        ((1u64 << 52) | mantissa, biased - 1023 - 52)
    };
    let coeff0 = num_bigint::BigInt::from(m);
    let (mut coeff, mut exp) = if e >= 0 {
        (coeff0 << (e as u32), 0i64)
    } else {
        let k = (-e) as u32;
        (coeff0 * num_bigint::BigInt::from(5u32).pow(k), -(k as i64))
    };
    let ten = num_bigint::BigInt::from(10);
    while coeff != num_bigint::BigInt::zero() && (&coeff % &ten).is_zero() {
        coeff /= &ten;
        exp += 1;
    }
    DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff,
        exp,
    }
}

fn ten_pow(n: i64) -> num_bigint::BigInt {
    if n <= 0 {
        return num_bigint::BigInt::from(1);
    }
    num_bigint::BigInt::from(10).pow(n as u32)
}

fn digit_count(coeff: &num_bigint::BigInt) -> usize {
    if num_traits::Zero::is_zero(coeff) {
        return 1;
    }
    coeff.to_string().len()
}

/// CPython's decimal-to-string algorithm (IBM spec `to-scientific-string`):
/// plain notation when the exponent is small enough, scientific otherwise.
fn format_decvalue(v: &DecValue) -> String {
    let sign_str = if v.sign { "-" } else { "" };
    match v.special {
        DecSpecial::Infinity => return format!("{}Infinity", sign_str),
        DecSpecial::QNaN => {
            let digits = if num_traits::Zero::is_zero(&v.coeff) {
                String::new()
            } else {
                v.coeff.to_string()
            };
            return format!("{}NaN{}", sign_str, digits);
        }
        DecSpecial::SNaN => {
            let digits = if num_traits::Zero::is_zero(&v.coeff) {
                String::new()
            } else {
                v.coeff.to_string()
            };
            return format!("{}sNaN{}", sign_str, digits);
        }
        DecSpecial::Finite => {}
    }
    let digits = if num_traits::Zero::is_zero(&v.coeff) {
        "0".to_string()
    } else {
        v.coeff.to_string()
    };
    let leading = digits.len() as i64;
    let adjusted_exp = v.exp + leading - 1;
    if v.exp <= 0 && adjusted_exp >= -6 {
        let body = if v.exp == 0 {
            digits
        } else if leading <= -v.exp {
            format!("0.{}{}", "0".repeat((-v.exp - leading) as usize), digits)
        } else {
            let split = (leading + v.exp) as usize;
            format!("{}.{}", &digits[..split], &digits[split..])
        };
        format!("{}{}", sign_str, body)
    } else {
        let body = if leading == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        let exp_sign = if adjusted_exp >= 0 { "+" } else { "-" };
        format!("{}{}E{}{}", sign_str, body, exp_sign, adjusted_exp.abs())
    }
}

thread_local! {
    static DECIMAL_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static DECIMAL_CONTEXT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static DECIMAL_CURRENT_CONTEXT: std::cell::RefCell<(usize, String)> = std::cell::RefCell::new((28, "ROUND_HALF_EVEN".to_string()));
    static FRACTION_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// The plain `Fraction` type (not a subclass) — Fraction arithmetic always
/// returns plain `Fraction` instances, matching CPython.
pub(crate) fn get_fraction_type() -> PyObjectRef {
    FRACTION_TYPE.with(|c| c.borrow().clone().unwrap())
}

fn current_decimal_context() -> (usize, String) {
    DECIMAL_CURRENT_CONTEXT.with(|c| c.borrow().clone())
}

const DEC_SIGN_KEY: &str = "_sign";
const DEC_COEFF_KEY: &str = "_coeff";
const DEC_EXP_KEY: &str = "_exp";
const DEC_SPECIAL_KEY: &str = "_special";

fn special_to_str(s: &DecSpecial) -> &'static str {
    match s {
        DecSpecial::Finite => "",
        DecSpecial::QNaN => "n",
        DecSpecial::SNaN => "N",
        DecSpecial::Infinity => "F",
    }
}
fn special_from_str(s: &str) -> DecSpecial {
    match s {
        "n" => DecSpecial::QNaN,
        "N" => DecSpecial::SNaN,
        "F" => DecSpecial::Infinity,
        _ => DecSpecial::Finite,
    }
}

fn decval_to_instance(v: &DecValue) -> PyObjectRef {
    let typ = get_decimal_type();
    let mut dict = AttrMap::new();
    dict.insert(DEC_SIGN_KEY.to_string(), py_bool(v.sign));
    dict.insert(DEC_COEFF_KEY.to_string(), py_int(v.coeff.clone()));
    dict.insert(DEC_EXP_KEY.to_string(), py_int(v.exp));
    dict.insert(
        DEC_SPECIAL_KEY.to_string(),
        py_str(special_to_str(&v.special)),
    );
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn instance_to_decval(obj: &PyObjectRef) -> Option<DecValue> {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        let sign = dict.get(DEC_SIGN_KEY)?.truthy();
        let coeff = match &*dict.get(DEC_COEFF_KEY)?.borrow() {
            PyObject::Int(i) => i.clone(),
            _ => return None,
        };
        let exp = dict.get(DEC_EXP_KEY)?.as_i64().unwrap_or(0);
        let special = special_from_str(&dict.get(DEC_SPECIAL_KEY)?.str());
        Some(DecValue {
            special,
            sign,
            coeff,
            exp,
        })
    } else {
        None
    }
}

/// Coerce a constructor argument (str/int/float/Decimal/tuple) into a DecValue.
fn decval_from_pyobject(v: &PyObjectRef) -> PyResult<DecValue> {
    if let Some(existing) = instance_to_decval(v) {
        return Ok(existing);
    }
    match &*v.borrow() {
        PyObject::Str(s) => parse_decimal_str(s).ok_or_else(|| {
            PyError::Exception(
                "InvalidOperation".to_string(),
                PyObjectRef::new(PyObject::Exception {
                    typ: "InvalidOperation".to_string(),
                    args: vec![py_str(&format!("invalid literal for Decimal: '{}'", s))],
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }),
            )
        }),
        PyObject::Int(i) => {
            let sign = num_traits::Signed::is_negative(i);
            Ok(DecValue {
                special: DecSpecial::Finite,
                sign,
                coeff: num_traits::Signed::abs(i),
                exp: 0,
            })
        }
        PyObject::Bool(b) => Ok(DecValue {
            special: DecSpecial::Finite,
            sign: false,
            coeff: num_bigint::BigInt::from(if *b { 1 } else { 0 }),
            exp: 0,
        }),
        PyObject::Float(f) => Ok(decval_from_f64(*f)),
        PyObject::Tuple(parts) => {
            if parts.len() != 3 {
                return Err(PyError::value_error(
                    "argument must be a sequence of length 3",
                ));
            }
            let sign = parts[0].as_i64().unwrap_or(0) != 0;
            let digit_items: Vec<PyObjectRef> = match &*parts[1].borrow() {
                PyObject::Tuple(d) | PyObject::List(d) => d.clone(),
                _ => return Err(PyError::value_error("digits must be a sequence of ints")),
            };
            let mut digits_str = String::new();
            for d in &digit_items {
                digits_str.push_str(&d.as_i64().unwrap_or(0).to_string());
            }
            if digits_str.is_empty() {
                digits_str.push('0');
            }
            match &*parts[2].borrow() {
                PyObject::Str(s) if s == "F" => Ok(DecValue::infinity(sign)),
                PyObject::Str(s) if s == "n" || s == "N" => {
                    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10)
                        .unwrap_or_default();
                    Ok(DecValue {
                        special: special_from_str(s),
                        sign,
                        coeff,
                        exp: 0,
                    })
                }
                _ => {
                    let exp = parts[2].as_i64().unwrap_or(0);
                    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10)
                        .unwrap_or_default();
                    Ok(DecValue {
                        special: DecSpecial::Finite,
                        sign,
                        coeff,
                        exp,
                    })
                }
            }
        }
        PyObject::None => Ok(DecValue::zero()),
        // A `fractions.Fraction` operand (e.g. `Decimal('1') < Fraction(1,2)` /
        // `Decimal('1001.0') == Fraction(2002, 2)` — CPython's numeric tower
        // converts the Fraction to a Decimal for the comparison) — was
        // hitting the `_ =>` "unsupported type" TypeError below.
        _ => {
            if let Some((num, den)) = frac_instance_num_den(v) {
                let (sign, coeff) = if num.sign() == num_bigint::Sign::Minus {
                    (true, (-num.clone()).to_biguint().unwrap_or_default())
                } else {
                    (false, num.to_biguint().unwrap_or_default())
                };
                let den_b = den.to_biguint().unwrap_or_default();
                // value = coeff/den_b; express exactly as X * 10**e by
                // factoring den_b = 2**twos * 5**fives and clearing the
                // extra 2s/5s against a power of 10:
                //   fives >= twos: X = coeff * 2**(fives-twos), e = -fives
                //   twos  >  fives: X = coeff * 5**(twos-fives), e = -twos
                let (twos, fives) = (factor_power_of(&den_b, 2), factor_power_of(&den_b, 5));
                let mut den_rem = den_b;
                for _ in 0..twos {
                    den_rem /= 2u8;
                }
                for _ in 0..fives {
                    den_rem /= 5u8;
                }
                // den_rem must be 1 now (any 2s/5s removed); remaining
                // factors make it non-terminating — approximate via float.
                if den_rem == num_bigint::BigUint::from(1u8) {
                    let (scaled, exp) = if fives >= twos {
                        (
                            coeff * num_bigint::BigUint::from(2u8).pow((fives - twos) as u32),
                            -(fives as i64),
                        )
                    } else {
                        (
                            coeff * num_bigint::BigUint::from(5u8).pow((twos - fives) as u32),
                            -(twos as i64),
                        )
                    };
                    Ok(DecValue {
                        special: DecSpecial::Finite,
                        sign,
                        coeff: scaled.into(),
                        exp,
                    })
                } else {
                    Ok(decval_from_f64(frac_to_f64(&num, &den)))
                }
            } else {
                Err(PyError::type_error(
                    "conversion from unsupported type to Decimal",
                ))
            }
        }
    }
}

fn round_decvalue(v: &DecValue, precision: usize, rounding: &str) -> DecValue {
    if v.special != DecSpecial::Finite {
        return v.clone();
    }
    let ndigits = digit_count(&v.coeff);
    if ndigits <= precision {
        return v.clone();
    }
    let drop = ndigits - precision;
    let divisor = ten_pow(drop as i64);
    let q = &v.coeff / &divisor;
    let r = &v.coeff % &divisor;
    let new_exp = v.exp + drop as i64;
    let twice_r = &r * num_bigint::BigInt::from(2);
    let round_up = match rounding {
        "ROUND_HALF_UP" => twice_r >= divisor,
        "ROUND_HALF_DOWN" => twice_r > divisor,
        "ROUND_HALF_EVEN" => {
            use std::cmp::Ordering;
            match twice_r.cmp(&divisor) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => (&q % 2) != num_bigint::BigInt::from(0),
            }
        }
        "ROUND_UP" => !num_traits::Zero::is_zero(&r),
        "ROUND_DOWN" => false,
        "ROUND_CEILING" => !num_traits::Zero::is_zero(&r) && !v.sign,
        "ROUND_FLOOR" => !num_traits::Zero::is_zero(&r) && v.sign,
        "ROUND_05UP" => {
            !num_traits::Zero::is_zero(&r) && {
                let last = &q % 10;
                last == num_bigint::BigInt::from(0) || last == num_bigint::BigInt::from(5)
            }
        }
        _ => {
            use std::cmp::Ordering;
            match twice_r.cmp(&divisor) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => (&q % 2) != num_bigint::BigInt::from(0),
            }
        }
    };
    let final_q = if round_up { q + 1 } else { q };
    DecValue {
        special: DecSpecial::Finite,
        sign: v.sign,
        coeff: final_q,
        exp: new_exp,
    }
}

fn round_to_context(v: DecValue) -> DecValue {
    let (precision, rounding) = current_decimal_context();
    round_decvalue(&v, precision, &rounding)
}

fn decval_align(a: &DecValue, b: &DecValue) -> (num_bigint::BigInt, num_bigint::BigInt, i64) {
    let exp = a.exp.min(b.exp);
    let a_scaled = &a.coeff * ten_pow(a.exp - exp);
    let b_scaled = &b.coeff * ten_pow(b.exp - exp);
    (a_scaled, b_scaled, exp)
}

fn decimal_invalid_op(msg: &str) -> PyError {
    PyError::Exception(
        "InvalidOperation".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "InvalidOperation".to_string(),
            args: vec![py_str(msg)],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}
fn decimal_division_by_zero(msg: &str) -> PyError {
    PyError::Exception(
        "DivisionByZero".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "DivisionByZero".to_string(),
            args: vec![py_str(msg)],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

fn decimal_add(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.is_nan() || b.is_nan() {
        let src = if a.is_nan() { a } else { b };
        return Ok(DecValue {
            special: DecSpecial::QNaN,
            sign: src.sign,
            coeff: src.coeff.clone(),
            exp: 0,
        });
    }
    if a.special == DecSpecial::Infinity || b.special == DecSpecial::Infinity {
        if a.special == DecSpecial::Infinity
            && b.special == DecSpecial::Infinity
            && a.sign != b.sign
        {
            return Err(decimal_invalid_op("(+Infinity) + (-Infinity)"));
        }
        return Ok(DecValue::infinity(if a.special == DecSpecial::Infinity {
            a.sign
        } else {
            b.sign
        }));
    }
    let (as_, bs, exp) = decval_align(a, b);
    let sum = (if a.sign { -as_ } else { as_ }) + (if b.sign { -bs } else { bs });
    let sign = num_traits::Signed::is_negative(&sum);
    let result = DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff: num_traits::Signed::abs(&sum),
        exp,
    };
    Ok(round_to_context(result))
}

fn decimal_negate(v: &DecValue) -> DecValue {
    let mut r = v.clone();
    if r.special == DecSpecial::Finite || r.special == DecSpecial::Infinity {
        r.sign = !r.sign;
    }
    r
}

fn decimal_sub(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    decimal_add(a, &decimal_negate(b))
}

fn decimal_mul(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.is_nan() || b.is_nan() {
        let src = if a.is_nan() { a } else { b };
        return Ok(DecValue {
            special: DecSpecial::QNaN,
            sign: src.sign,
            coeff: src.coeff.clone(),
            exp: 0,
        });
    }
    let sign = a.sign != b.sign;
    if a.special == DecSpecial::Infinity || b.special == DecSpecial::Infinity {
        if a.is_zero() || b.is_zero() {
            return Err(decimal_invalid_op("(+/-Infinity) * 0"));
        }
        return Ok(DecValue::infinity(sign));
    }
    let result = DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff: &a.coeff * &b.coeff,
        exp: a.exp + b.exp,
    };
    Ok(round_to_context(result))
}

fn decimal_div(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.is_nan() || b.is_nan() {
        let src = if a.is_nan() { a } else { b };
        return Ok(DecValue {
            special: DecSpecial::QNaN,
            sign: src.sign,
            coeff: src.coeff.clone(),
            exp: 0,
        });
    }
    let sign = a.sign != b.sign;
    if a.special == DecSpecial::Infinity && b.special == DecSpecial::Infinity {
        return Err(decimal_invalid_op("(+/-Infinity) / (+/-Infinity)"));
    }
    if a.special == DecSpecial::Infinity {
        return Ok(DecValue::infinity(sign));
    }
    if b.special == DecSpecial::Infinity {
        return Ok(DecValue {
            special: DecSpecial::Finite,
            sign,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        });
    }
    if b.is_zero() {
        if a.is_zero() {
            return Err(decimal_invalid_op("0 / 0"));
        }
        return Err(decimal_division_by_zero("division by zero"));
    }
    if a.is_zero() {
        return Ok(round_to_context(DecValue {
            special: DecSpecial::Finite,
            sign,
            coeff: num_bigint::BigInt::from(0),
            exp: a.exp - b.exp,
        }));
    }
    let (precision, rounding) = current_decimal_context();
    // Scale the numerator so the integer quotient carries `precision` extra
    // guard digits, then round back down to context precision — simplest
    // correct-enough way to get a faithfully-rounded quotient without
    // implementing the spec's exact ideal-exponent bookkeeping.
    let guard = precision as i64 + digit_count(&a.coeff) as i64 + 2;
    let scaled_num = &a.coeff * ten_pow(guard);
    let raw_q = &scaled_num / &b.coeff;
    let raw_r = &scaled_num % &b.coeff;
    let raw_exp = a.exp - b.exp - guard;
    let mut result = DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff: raw_q,
        exp: raw_exp,
    };
    if !num_traits::Zero::is_zero(&raw_r) {
        // Inexact — nudge the last kept digit if a straightforward rounding
        // of the truncated remainder would change it (half-up on the guard
        // digits is precise enough given the wide guard margin above).
        if &raw_r * 2 >= b.coeff {
            result.coeff += 1;
        }
    }
    Ok(round_decvalue(&result, precision, &rounding))
}

fn decimal_compare(a: &DecValue, b: &DecValue) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if a.is_nan() || b.is_nan() {
        return None;
    }
    match (&a.special, &b.special) {
        (DecSpecial::Infinity, DecSpecial::Infinity) => {
            return Some(if a.sign == b.sign {
                Ordering::Equal
            } else if a.sign {
                Ordering::Less
            } else {
                Ordering::Greater
            });
        }
        (DecSpecial::Infinity, _) => {
            return Some(if a.sign {
                Ordering::Less
            } else {
                Ordering::Greater
            })
        }
        (_, DecSpecial::Infinity) => {
            return Some(if b.sign {
                Ordering::Greater
            } else {
                Ordering::Less
            })
        }
        _ => {}
    }
    if a.is_zero() && b.is_zero() {
        return Some(Ordering::Equal);
    }
    // Different signs decide immediately.
    if a.sign != b.sign {
        return Some(if a.sign {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    // Same sign: compare MAGNITUDES. The leading-digit exponent
    // `digit_count(coeff) + exp` decides when the values don't overlap;
    // only values with the SAME order of magnitude need exact alignment.
    // (The previous `decval_align` unconditional scaling blew up on huge
    // exponents — e.g. D('-1e425000000') < 0 computed 10**425000000.)
    let a_zero = a.is_zero();
    let b_zero = b.is_zero();
    let mag = |v: &DecValue| digit_count(&v.coeff) as i64 + v.exp;
    let ord = if a_zero {
        Ordering::Less
    }
    // |a| = 0 < |b| (b nonzero)
    else if b_zero {
        Ordering::Greater
    } else {
        let (ma, mb) = (mag(a), mag(b));
        if ma != mb {
            if ma < mb {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            let (as_, bs, _) = decval_align(a, b);
            as_.cmp(&bs)
        }
    };
    Some(if a.sign { ord.reverse() } else { ord })
}

fn decval_to_f64(v: &DecValue) -> f64 {
    match v.special {
        DecSpecial::Infinity => {
            if v.sign {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        }
        DecSpecial::QNaN | DecSpecial::SNaN => f64::NAN,
        DecSpecial::Finite => {
            // Parse the exact decimal string rather than coeff as f64 times
            // 10^exp — that separate multiplication introduces float error
            // (e.g. 12345.0 * 0.01 != 123.45 exactly), whereas Rust's own
            // string-to-f64 parsing correctly rounds to the nearest float.
            format!("{}{}e{}", if v.sign { "-" } else { "" }, v.coeff, v.exp)
                .parse()
                .unwrap_or(0.0)
        }
    }
}

/// Extract an object's numeric VALUE as `(real, imag)` parts, covering the
/// native numeric variants PLUS `fractions.Fraction` and `decimal.Decimal`
/// instances — real CPython's numeric tower compares all of these by value
/// (`Fraction(2002,2) == 1001+0j` and `Decimal('1001.0') == 1001+0j` are
/// both True). Used by the cross-type equality path in `PyObject::equals`.
pub(crate) fn numeric_parts_from_ref(obj: &PyObjectRef) -> Option<(f64, f64)> {
    let borrowed = obj.borrow();
    match &*borrowed {
        PyObject::Complex(re, im) => Some((*re, *im)),
        PyObject::Int(n) => n.to_f64().map(|f| (f, 0.0)),
        PyObject::Float(f) => Some((*f, 0.0)),
        PyObject::Bool(b) => Some((if *b { 1.0 } else { 0.0 }, 0.0)),
        PyObject::Instance { .. } => {
            if let Some((num, den)) = frac_instance_num_den(obj) {
                Some((frac_to_f64(&num, &den), 0.0))
            } else if let Some(v) = instance_to_decval(obj) {
                Some((decval_to_f64(&v), 0.0))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn normalize_decval(v: &DecValue) -> DecValue {
    if v.special != DecSpecial::Finite || v.is_zero() {
        if v.is_zero() {
            return DecValue {
                special: DecSpecial::Finite,
                sign: v.sign,
                coeff: num_bigint::BigInt::from(0),
                exp: 0,
            };
        }
        return v.clone();
    }
    let mut coeff = v.coeff.clone();
    let mut exp = v.exp;
    let ten = num_bigint::BigInt::from(10);
    while &coeff % &ten == num_bigint::BigInt::from(0) && coeff != num_bigint::BigInt::from(0) {
        coeff /= &ten;
        exp += 1;
    }
    DecValue {
        special: DecSpecial::Finite,
        sign: v.sign,
        coeff,
        exp,
    }
}

fn get_decimal_type() -> PyObjectRef {
    let existing = DECIMAL_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_decimal_type();
    DECIMAL_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn build_decimal_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }

    type_dict.insert_str(
        "__init__",
        bf!("__init__", |args| {
            let v = if args.len() > 1 {
                decval_from_pyobject(&args[1])?
            } else {
                DecValue::zero()
            };
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert(DEC_SIGN_KEY.to_string(), py_bool(v.sign));
                dict.insert(DEC_COEFF_KEY.to_string(), py_int(v.coeff));
                dict.insert(DEC_EXP_KEY.to_string(), py_int(v.exp));
                dict.insert(
                    DEC_SPECIAL_KEY.to_string(),
                    py_str(special_to_str(&v.special)),
                );
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_str(&format!("Decimal('{}')", format_decvalue(&v))))
        }),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_str(&format_decvalue(&v)))
        }),
    );
    type_dict.insert_str(
        "__int__",
        bf!("__int__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite {
                return Err(PyError::value_error("cannot convert NaN/Infinity to int"));
            }
            let truncated = if v.exp >= 0 {
                &v.coeff * ten_pow(v.exp)
            } else {
                &v.coeff / ten_pow(-v.exp)
            };
            Ok(py_int(if v.sign { -truncated } else { truncated }))
        }),
    );
    type_dict.insert_str(
        "__float__",
        bf!("__float__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_float(decval_to_f64(&v)))
        }),
    );
    type_dict.insert_str(
        "as_integer_ratio",
        bf!("as_integer_ratio", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if matches!(v.special, DecSpecial::QNaN | DecSpecial::SNaN) {
                return Err(PyError::value_error("cannot convert NaN to integer ratio"));
            }
            if v.special != DecSpecial::Finite {
                return Err(PyError::overflow_error(
                    "cannot convert Infinity to integer ratio",
                ));
            }
            let (num, den) = if v.exp >= 0 {
                (v.coeff * ten_pow(v.exp), BigInt::one())
            } else {
                (v.coeff.clone(), ten_pow(-v.exp))
            };
            // Reduce to lowest terms (Decimal('3.5e-2') -> 7/200, not 35/1000).
            let (num, den) = frac_normalize(if v.sign { -num } else { num }, den)?;
            Ok(py_tuple(vec![py_int(num), py_int(den)]))
        }),
    );
    type_dict.insert_str(
        "sqrt",
        bf!("sqrt", |args| {
            if args.is_empty() {
                return Err(PyError::type_error("sqrt() missing self"));
            }
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.is_nan() {
                return Err(decimal_invalid_op("NaN ** (1/2)"));
            }
            if v.sign && !v.is_zero() {
                return Err(decimal_invalid_op("(-x) ** (1/2)"));
            }
            if v.special == DecSpecial::Infinity {
                return Ok(decval_to_instance(&DecValue::infinity(false)));
            }
            if v.is_zero() {
                return Ok(decval_to_instance(&v.clone()));
            }
            // Integer Newton's-method square root at the context's
            // precision (large enough for an exact f64 conversion of the
            // result, which is what statistics.py / test_math use it for).
            let (precision, _rounding) = current_decimal_context();
            let prec = (precision as i64).max(60);
            let mut c = v.coeff.clone();
            let mut e = v.exp;
            if e % 2 != 0 {
                c *= 10;
                e -= 1;
            }
            // Scale the coefficient so its integer sqrt has ~`prec`
            // significant digits, take the exact integer square root, and
            // adjust the exponent back down.
            let c_digits = (c.bits() as f64 * 0.30103) as i64 + 1;
            let m = (prec - (c_digits + 1) / 2).max(0);
            let scaled = &c * ten_pow(2 * m);
            let root = scaled.sqrt();
            let result = DecValue {
                special: DecSpecial::Finite,
                sign: false,
                coeff: root,
                exp: e / 2 - m,
            };
            Ok(decval_to_instance(&result))
        }),
    );
    type_dict.insert_str(
        "__bool__",
        bf!("__bool__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(!v.is_zero()))
        }),
    );
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite {
                return Ok(py_int(0));
            }
            // Normalize (strip trailing zeros) so numerically-equal Decimals
            // with different (coeff, exp) representations — e.g. 1 vs 1.0 —
            // hash the same way `1 == 1.0` requires.
            let n = normalize_decval(&v);
            let s = format!("{}{}{}", n.sign, n.coeff, n.exp);
            builtin_hash(&[py_str(&s)])
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            // An operand that isn't convertible to a Decimal (complex, a
            // user-defined class, ...) must return NotImplemented so the OTHER
            // side's reflected __eq__ gets a chance (`Decimal('1001.0') ==
            // 1001+0j` is True via complex.__eq__, not False).
            let b = match decval_from_pyobject(&args[1]) {
                Ok(v) => v,
                Err(_) => return Ok(py_not_implemented()),
            };
            Ok(py_bool(
                decimal_compare(&a, &b) == Some(std::cmp::Ordering::Equal),
            ))
        }),
    );
    macro_rules! dec_cmp {
        ($name:expr, $ord:pat) => {
            type_dict.insert(
                $name.to_string(),
                bf!($name, |args| {
                    let a = instance_to_decval(&args[0])
                        .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
                    let b = match decval_from_pyobject(&args[1]) {
                        Ok(v) => v,
                        // An unconvertible operand (complex, ...) must produce
                        // the standard "not supported between instances"
                        // TypeError, matching real CPython — not the internal
                        // conversion message.
                        Err(_) => {
                            return Err(PyError::type_error(format!(
                                "'{}' not supported between instances of '{}' and '{}'",
                                match $name {
                                    "__lt__" => "<",
                                    "__gt__" => ">",
                                    _ => "?",
                                },
                                args[0].get_type_name(),
                                args[1].get_type_name()
                            )))
                        }
                    };
                    match decimal_compare(&a, &b) {
                        Some($ord) => Ok(py_bool(true)),
                        Some(_) => Ok(py_bool(false)),
                        None => Err(PyError::type_error("cannot compare NaN with Decimal")),
                    }
                }),
            );
        };
    }
    dec_cmp!("__lt__", std::cmp::Ordering::Less);
    dec_cmp!("__gt__", std::cmp::Ordering::Greater);
    type_dict.insert_str(
        "__le__",
        bf!("__le__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = match decval_from_pyobject(&args[1]) {
                Ok(v) => v,
                Err(_) => {
                    return Err(PyError::type_error(format!(
                        "'<=' not supported between instances of '{}' and '{}'",
                        args[0].get_type_name(),
                        args[1].get_type_name()
                    )))
                }
            };
            match decimal_compare(&a, &b) {
                Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal) => {
                    Ok(py_bool(true))
                }
                Some(_) => Ok(py_bool(false)),
                None => Err(PyError::type_error("cannot compare NaN with Decimal")),
            }
        }),
    );
    type_dict.insert_str(
        "__ge__",
        bf!("__ge__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = match decval_from_pyobject(&args[1]) {
                Ok(v) => v,
                Err(_) => {
                    return Err(PyError::type_error(format!(
                        "'>=' not supported between instances of '{}' and '{}'",
                        args[0].get_type_name(),
                        args[1].get_type_name()
                    )))
                }
            };
            match decimal_compare(&a, &b) {
                Some(std::cmp::Ordering::Greater) | Some(std::cmp::Ordering::Equal) => {
                    Ok(py_bool(true))
                }
                Some(_) => Ok(py_bool(false)),
                None => Err(PyError::type_error("cannot compare NaN with Decimal")),
            }
        }),
    );
    macro_rules! dec_binop {
        ($name:expr, $op:expr) => {
            type_dict.insert(
                $name.to_string(),
                bf!($name, |args| {
                    let a = instance_to_decval(&args[0])
                        .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
                    // Decimal arithmetic accepts only Decimal/int/bool/float
                    // operands — anything else (Fraction, complex, str, ...)
                    // defers to the other operand's reflected method, which
                    // ultimately raises TypeError (CPython: "Decimal refuses
                    // mixed arithmetic (but not mixed comparisons)").
                    let b_ok = matches!(
                        &*args[1].borrow(),
                        PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)
                    ) || instance_to_decval(&args[1]).is_some();
                    if !b_ok {
                        return Ok(py_not_implemented());
                    }
                    let b = decval_from_pyobject(&args[1])?;
                    Ok(decval_to_instance(&$op(&a, &b)?))
                }),
            );
        };
    }
    dec_binop!("__add__", decimal_add);
    dec_binop!("__radd__", |a, b| decimal_add(b, a));
    dec_binop!("__sub__", decimal_sub);
    dec_binop!("__rsub__", |a, b| decimal_sub(b, a));
    dec_binop!("__mul__", decimal_mul);
    dec_binop!("__rmul__", |a, b| decimal_mul(b, a));
    dec_binop!("__truediv__", decimal_div);
    dec_binop!("__rtruediv__", |a, b| decimal_div(b, a));
    type_dict.insert_str(
        "__floordiv__",
        bf!("__floordiv__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            let q = decimal_div(&a, &b)?;
            if q.special != DecSpecial::Finite {
                return Ok(decval_to_instance(&q));
            }
            let truncated = if q.exp >= 0 {
                &q.coeff * ten_pow(q.exp)
            } else {
                &q.coeff / ten_pow(-q.exp)
            };
            Ok(decval_to_instance(&DecValue {
                special: DecSpecial::Finite,
                sign: q.sign,
                coeff: truncated,
                exp: 0,
            }))
        }),
    );
    type_dict.insert_str(
        "__mod__",
        bf!("__mod__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            if b.is_zero() {
                return Err(decimal_invalid_op("0 modulo"));
            }
            let q = decimal_div(&a, &b)?;
            let truncated_q = if q.exp >= 0 {
                &q.coeff * ten_pow(q.exp)
            } else {
                &q.coeff / ten_pow(-q.exp)
            };
            let trunc_dec = DecValue {
                special: DecSpecial::Finite,
                sign: q.sign,
                coeff: truncated_q,
                exp: 0,
            };
            let prod = decimal_mul(&trunc_dec, &b)?;
            Ok(decval_to_instance(&decimal_sub(&a, &prod)?))
        }),
    );
    type_dict.insert_str(
        "__pow__",
        bf!("__pow__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            if b.special != DecSpecial::Finite || b.exp < 0 {
                return Err(PyError::runtime_error(
                    "Decimal ** non-integer exponent is not supported",
                ));
            }
            let n = (&b.coeff * ten_pow(b.exp))
                .to_string()
                .parse::<i64>()
                .unwrap_or(0);
            let n = if b.sign { -n } else { n };
            if n < 0 {
                return Err(PyError::runtime_error(
                    "Decimal ** negative exponent is not supported",
                ));
            }
            let mut result = DecValue {
                special: DecSpecial::Finite,
                sign: false,
                coeff: num_bigint::BigInt::from(1),
                exp: 0,
            };
            for _ in 0..n {
                result = decimal_mul(&result, &a)?;
            }
            Ok(decval_to_instance(&result))
        }),
    );
    type_dict.insert_str(
        "__neg__",
        bf!("__neg__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&decimal_negate(&v)))
        }),
    );
    type_dict.insert_str(
        "__pos__",
        bf!("__pos__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&round_to_context(v)))
        }),
    );
    type_dict.insert_str(
        "__abs__",
        bf!("__abs__", |args| {
            let mut v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            v.sign = false;
            Ok(decval_to_instance(&v))
        }),
    );
    type_dict.insert_str(
        "is_nan",
        bf!("is_nan", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.is_nan()))
        }),
    );
    type_dict.insert_str(
        "is_infinite",
        bf!("is_infinite", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.special == DecSpecial::Infinity))
        }),
    );
    type_dict.insert_str(
        "is_finite",
        bf!("is_finite", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.special == DecSpecial::Finite))
        }),
    );
    type_dict.insert_str(
        "is_zero",
        bf!("is_zero", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.is_zero()))
        }),
    );
    type_dict.insert_str(
        "is_signed",
        bf!("is_signed", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.sign))
        }),
    );
    type_dict.insert_str(
        "copy_sign",
        bf!("copy_sign", |args| {
            let mut v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let other = decval_from_pyobject(&args[1])?;
            v.sign = other.sign;
            Ok(decval_to_instance(&v))
        }),
    );
    type_dict.insert_str(
        "copy_abs",
        bf!("copy_abs", |args| {
            let mut v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            v.sign = false;
            Ok(decval_to_instance(&v))
        }),
    );
    type_dict.insert_str(
        "copy_negate",
        bf!("copy_negate", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&decimal_negate(&v)))
        }),
    );
    type_dict.insert_str(
        "as_tuple",
        bf!("as_tuple", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let sign_val = py_int(if v.sign { 1 } else { 0 });
            let digits_str = if num_traits::Zero::is_zero(&v.coeff) {
                "0".to_string()
            } else {
                v.coeff.to_string()
            };
            let digits: Vec<PyObjectRef> = digits_str
                .chars()
                .map(|c| py_int(c.to_digit(10).unwrap_or(0) as i64))
                .collect();
            let exp_val = match v.special {
                DecSpecial::Finite => py_int(v.exp),
                DecSpecial::Infinity => py_str("F"),
                DecSpecial::QNaN => py_str("n"),
                DecSpecial::SNaN => py_str("N"),
            };
            Ok(py_tuple(vec![sign_val, py_tuple(digits), exp_val]))
        }),
    );
    type_dict.insert_str(
        "normalize",
        bf!("normalize", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&normalize_decval(&round_to_context(v))))
        }),
    );
    type_dict.insert_str(
        "quantize",
        bf!("quantize", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if args.len() < 2 {
                return Err(PyError::type_error("quantize() missing exponent argument"));
            }
            let target = decval_from_pyobject(&args[1])?;
            if v.special != DecSpecial::Finite || target.special != DecSpecial::Finite {
                return Err(decimal_invalid_op("quantize with non-finite argument"));
            }
            let (_, rounding) = current_decimal_context();
            let target_exp = target.exp;
            let result = if target_exp >= v.exp {
                let drop = (target_exp - v.exp) as usize;
                round_decvalue(
                    &v,
                    digit_count(&v.coeff).saturating_sub(drop).max(1),
                    &rounding,
                )
            } else {
                let scale = ten_pow(v.exp - target_exp);
                DecValue {
                    special: DecSpecial::Finite,
                    sign: v.sign,
                    coeff: &v.coeff * scale,
                    exp: target_exp,
                }
            };
            Ok(decval_to_instance(&DecValue {
                exp: target_exp,
                ..result
            }))
        }),
    );
    type_dict.insert_str(
        "to_integral_value",
        bf!("to_integral_value", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite || v.exp >= 0 {
                return Ok(decval_to_instance(&v));
            }
            let (_, rounding) = current_decimal_context();
            let rounded = round_decvalue(
                &v,
                digit_count(&v.coeff)
                    .saturating_sub((-v.exp) as usize)
                    .max(1),
                &rounding,
            );
            Ok(decval_to_instance(&DecValue {
                exp: 0,
                coeff: &rounded.coeff * ten_pow(rounded.exp),
                ..rounded
            }))
        }),
    );
    type_dict.insert_str(
        "compare",
        bf!("compare", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            let n: i64 = match decimal_compare(&a, &b) {
                Some(std::cmp::Ordering::Less) => -1,
                Some(std::cmp::Ordering::Greater) => 1,
                Some(std::cmp::Ordering::Equal) => 0,
                None => return Ok(decval_to_instance(&DecValue::nan())),
            };
            Ok(decval_to_instance(&DecValue {
                special: DecSpecial::Finite,
                sign: n < 0,
                coeff: num_bigint::BigInt::from(n.abs()),
                exp: 0,
            }))
        }),
    );

    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            // CPython's Decimal hash: (coeff * 10**exp) mod 2**61-1 for finite
            // values (using the modular inverse of 10 for negative exponents),
            // ±INF (314159) for infinities, 0 for nans; signed by the value.
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            const MOD: i64 = (1 << 61) - 1;
            let magnitude = match v.special {
                DecSpecial::QNaN | DecSpecial::SNaN => 0i64,
                DecSpecial::Infinity => 314159,
                DecSpecial::Finite => {
                    let modulus = num_bigint::BigInt::from(MOD);
                    let exp_hash = if v.exp >= 0 {
                        num_bigint::BigInt::from(10u32)
                            .modpow(&num_bigint::BigInt::from(v.exp), &modulus)
                    } else {
                        // 10**(-exp) = inv10**(|exp|); inv10 = 10**-1 mod P.
                        let inv10 = crate::object::bigint_mod_inverse(
                            &num_bigint::BigInt::from(10),
                            &modulus,
                        )
                        .unwrap_or_else(|| num_bigint::BigInt::from(1));
                        inv10.modpow(&num_bigint::BigInt::from(-v.exp), &modulus)
                    };
                    let h = (&v.coeff % &modulus * exp_hash) % &modulus;
                    h.to_i64().unwrap_or(0)
                }
            };
            let result = if v.sign { -magnitude } else { magnitude };
            Ok(py_int(if result == -1 { -2 } else { result }))
        }),
    );
    type_dict.insert_str(
        "from_float",
        bf!("from_float", |args| {
            // Decimal.from_float(f): the exact decimal value of the binary float.
            if args.is_empty() {
                return Err(PyError::type_error("from_float() takes exactly 1 argument"));
            }
            let f = args[0]
                .as_f64()
                .ok_or_else(|| PyError::type_error("from_float() argument must be float"))?;
            Ok(decval_to_instance(&float_to_decvalue(f)))
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "Decimal".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn build_context_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    type_dict.insert_str(
        "__init__",
        bf!("__init__", |args| {
            let ctor_args = args[1..].to_vec();
            let kw: Option<PyDict> = ctor_args.last().and_then(|a| {
                if let PyObject::Dict(d) = &*a.borrow() {
                    Some((**d).clone())
                } else {
                    None
                }
            });
            let get_kw = |name: &str| {
                kw.as_ref()
                    .and_then(|d| d.get(&py_str(name)).ok().flatten())
            };
            let precision = get_kw("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
            let rounding = get_kw("rounding")
                .map(|v| v.str())
                .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("prec", py_int(precision as i64));
                dict.insert_str("rounding", py_str(&rounding));
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            let prec = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28)
            } else {
                28
            };
            Ok(py_str(&format!("Context(prec={})", prec)))
        }),
    );
    type_dict.insert_str(
        "copy",
        bf!("copy", |args| {
            // Context.copy() creates a shallow copy of the context
            let prec = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28)
            } else {
                28
            };
            let rounding = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                dict.get_str("rounding")
                    .map(|v| v.str())
                    .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string())
            } else {
                "ROUND_HALF_EVEN".to_string()
            };
            Ok(make_context_instance(prec as usize, &rounding))
        }),
    );
    type_dict.insert_str(
        "traps",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "traps".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("traps requires self"));
                    }
                    // Return existing traps dict or create one
                    let obj = &args[0];
                    let existing = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                        dict.get_str("traps").cloned()
                    } else {
                        None
                    };
                    if let Some(v) = existing {
                        return Ok(v);
                    }
                    let mut d = crate::object::PyDict::new();
                    let result = PyObjectRef::new(PyObject::Dict(Box::new(d)));
                    if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
                        dict.insert_str("traps", result.clone());
                    }
                    Ok(result)
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    PyObjectRef::new(PyObject::Type {
        name: "Context".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn get_context_type() -> PyObjectRef {
    let existing = DECIMAL_CONTEXT_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_context_type();
    DECIMAL_CONTEXT_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn make_context_instance(precision: usize, rounding: &str) -> PyObjectRef {
    let typ = get_context_type();
    let mut dict = AttrMap::new();
    dict.insert_str("prec", py_int(precision as i64));
    dict.insert_str("rounding", py_str(rounding));
    dict.insert_str("Emax", py_int(999999999999999999i64));
    dict.insert_str("Emin", py_int(-999999999999999999i64));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub fn create_decimal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dec_func {
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
    d.insert_str("Decimal", get_decimal_type());
    d.insert_str("Context", get_context_type());
    dec_func!("getcontext", |_args| {
        let (precision, rounding) = current_decimal_context();
        Ok(make_context_instance(precision, &rounding))
    });
    dec_func!("setcontext", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("setcontext() missing context argument"));
        }
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            let precision = dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
            let rounding = dict
                .get_str("rounding")
                .map(|v| v.str())
                .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
            DECIMAL_CURRENT_CONTEXT.with(|c| {
                *c.borrow_mut() = (precision, rounding);
            });
        }
        Ok(py_none())
    });
    // localcontext(ctx=None) — a minimal context-manager-like object; full
    // save/restore-on-exit semantics aren't implemented, only prec/rounding
    // application, which covers the common `with localcontext() as ctx:
    // ctx.prec = N` pattern used for one-off precision changes.
    dec_func!("localcontext", |args| {
        let (precision, rounding) = if !args.is_empty() {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                (
                    dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize,
                    dict.get_str("rounding")
                        .map(|v| v.str())
                        .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string()),
                )
            } else {
                current_decimal_context()
            }
        } else {
            current_decimal_context()
        };
        let ctx = make_context_instance(precision, &rounding);
        let mut cm_dict = HashMap::new();
        cm_dict.insert_str(
            "__enter__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__enter__".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        let precision =
                            dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
                        let rounding = dict
                            .get_str("rounding")
                            .map(|v| v.str())
                            .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
                        DECIMAL_CURRENT_CONTEXT.with(|c| {
                            *c.borrow_mut() = (precision, rounding);
                        });
                    }
                    Ok(args[0].clone())
                },
            }),
        );
        cm_dict.insert_str(
            "__exit__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__exit__".to_string(),
                func: |_args| {
                    DECIMAL_CURRENT_CONTEXT.with(|c| {
                        *c.borrow_mut() = (28, "ROUND_HALF_EVEN".to_string());
                    });
                    Ok(py_bool(false))
                },
            }),
        );
        let cm_typ = PyObjectRef::new(PyObject::Type {
            name: "_ContextManager".to_string(),
            dict: Box::new(str_map_to_typedict(cm_dict)),
            bases: vec![],
            mro: vec![],
        });
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str("prec", py_int(precision as i64));
        inst_dict.insert_str("rounding", py_str(&rounding));
        let _ = ctx;
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: cm_typ,
            dict: inst_dict,
        }))
    });
    // Exception types
    d.insert_str(
        "DecimalException",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "DecimalException".to_string(),
            func: crate::object::builtin_make_exception_decimalexception,
        }),
    );
    d.insert_str(
        "InvalidOperation",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "InvalidOperation".to_string(),
            func: crate::object::builtin_make_exception_invalidoperation,
        }),
    );
    d.insert_str(
        "DivisionByZero",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "DivisionByZero".to_string(),
            func: crate::object::builtin_make_exception_decimaldivisionbyzero,
        }),
    );
    d.insert_str(
        "Inexact",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Inexact".to_string(),
            func: crate::object::builtin_make_exception_inexact,
        }),
    );
    d.insert_str(
        "Rounded",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Rounded".to_string(),
            func: crate::object::builtin_make_exception_rounded,
        }),
    );
    d.insert_str(
        "Clamped",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Clamped".to_string(),
            func: crate::object::builtin_make_exception_clamped,
        }),
    );
    d.insert_str(
        "Overflow",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Overflow".to_string(),
            func: crate::object::builtin_make_exception_decimaloverflow,
        }),
    );
    d.insert_str(
        "Underflow",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Underflow".to_string(),
            func: crate::object::builtin_make_exception_decimalunderflow,
        }),
    );
    d.insert_str(
        "FloatOperation",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "FloatOperation".to_string(),
            func: crate::object::builtin_make_exception_floatoperation,
        }),
    );
    // Rounding mode constants — their real string values (that's what
    // CPython's decimal.ROUND_* constants actually are), so equality checks
    // and passing them to quantize()-style calls behave as real code expects.
    for name in [
        "ROUND_CEILING",
        "ROUND_DOWN",
        "ROUND_FLOOR",
        "ROUND_HALF_DOWN",
        "ROUND_HALF_EVEN",
        "ROUND_HALF_UP",
        "ROUND_UP",
        "ROUND_05UP",
    ] {
        d.insert(name.to_string(), py_str(name));
    }
    d.insert_str("MAX_PREC", py_int(999999999999999999i64));
    d.insert_str("MAX_EMAX", py_int(999999999999999999i64));
    d.insert_str("MIN_EMIN", py_int(-999999999999999999i64));
    d
}

// ---------------------------------------------------------------------------
// fractions.Fraction — a real rational-number type, replacing a former
// complete stub whose constructor just returned a formatted `"num/den"`
// STRING (`py_str`) instead of a genuine Fraction object at all — no
// arithmetic, no `__float__`, no comparisons, nothing beyond what a plain
// string happens to support by coincidence. Found via CPython's own
// `test_math.py::testHypot`, whose `hypot(Fraction(12, 32), Fraction(5,
// 32))` reached `float(a_fraction_shaped_string)` and got `ValueError:
// could not convert string to float: '3/8'`. Represented as a real
// `PyObject::Instance` (native-Type-backed, matching how other ad-hoc
// native classes in this codebase — e.g. `HTTPConnection` — are built) with
// `numerator`/`denominator` stored as plain instance attributes (arbitrary-
// precision `int`s, always reduced to lowest terms with a positive
// denominator), so it participates in the EXISTING Instance-based
// arithmetic/comparison dispatch (`try_dunder_binop`/`try_rich_compare`)
// with no changes needed to `ops_binary.rs`/`ops_compare.rs` at all.
// ---------------------------------------------------------------------------

use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, Zero};

fn frac_bigint_gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let mut a = a.abs();
    let mut b = b.abs();
    while !b.is_zero() {
        let t = b.clone();
        b = &a % &b;
        a = t;
    }
    a
}

fn frac_normalize(mut num: BigInt, mut den: BigInt) -> PyResult<(BigInt, BigInt)> {
    if den.is_zero() {
        return Err(PyError::ZeroDivisionError(format!("Fraction({}, 0)", num)));
    }
    if den.sign() == Sign::Minus {
        num = -num;
        den = -den;
    }
    let g = frac_bigint_gcd(&num, &den);
    if g > BigInt::one() {
        num /= &g;
        den /= &g;
    }
    Ok((num, den))
}

/// Exact binary-fraction decomposition of an `f64` (no precision loss) —
/// matches real Python's `float.as_integer_ratio()` / `Fraction.from_float`.
fn frac_float_to_ratio(f: f64) -> PyResult<(BigInt, BigInt)> {
    if f.is_nan() {
        return Err(PyError::value_error("cannot convert NaN to integer ratio"));
    }
    if f.is_infinite() {
        return Err(PyError::overflow_error(
            "cannot convert Infinity to integer ratio",
        ));
    }
    if f == 0.0 {
        return Ok((BigInt::zero(), BigInt::one()));
    }
    let bits = f.to_bits();
    let neg = bits >> 63 == 1;
    let biased_exp = ((bits >> 52) & 0x7ff) as i64;
    let mantissa_bits = bits & 0x000f_ffff_ffff_ffff;
    let (mantissa, exp): (u64, i64) = if biased_exp == 0 {
        (mantissa_bits, -1074)
    } else {
        (mantissa_bits | (1u64 << 52), biased_exp - 1075)
    };
    let mut num = BigInt::from(mantissa);
    if neg {
        num = -num;
    }
    let mut den = BigInt::one();
    if exp >= 0 {
        num *= BigInt::from(2).pow(exp as u32);
    } else {
        den = BigInt::from(2).pow((-exp) as u32);
    }
    frac_normalize(num, den)
}

/// Validate a digit group: non-empty, digits only with single underscores
/// strictly BETWEEN digits (`\d+(_\d+)*`), so `_1`, `1_`, `1__2` fail.
fn frac_valid_digits(s: &str) -> bool {
    let bytes: Vec<char> = s.chars().collect();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() || !bytes[bytes.len() - 1].is_ascii_digit() {
        return false;
    }
    let mut prev_digit = true;
    for &c in &bytes {
        if c == '_' {
            if !prev_digit {
                return false;
            }
            prev_digit = false;
        } else if c.is_ascii_digit() {
            prev_digit = true;
        } else {
            return false;
        }
    }
    true
}

/// Parse `"3/4"`, `"3"`, `"1.5"`, `"-1.5e2"` (real `Fraction(str)` accepts
/// decimal-literal-like strings too, converting exactly via `from_decimal`
/// semantics) — strict about signs/underscores/whitespace, matching
/// CPython's `_RATIONAL_FORMAT`.
fn frac_parse_str(s: &str) -> PyResult<(BigInt, BigInt)> {
    let s = s.trim();
    let bad = || PyError::value_error(format!("Invalid literal for Fraction: '{}'", s));
    let (neg, rest) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    };
    let starts_with_digit = rest.starts_with(|c: char| c.is_ascii_digit());
    let starts_with_dot_digit =
        rest.starts_with('.') && rest.len() > 1 && rest.as_bytes()[1].is_ascii_digit();
    if !starts_with_digit && !starts_with_dot_digit {
        return Err(bad());
    }
    // `num / denom` — neither side may carry a sign.
    if rest.contains('/') {
        let mut parts = rest.split('/');
        let n = parts.next().unwrap_or("").trim();
        let d = parts.next().unwrap_or("").trim();
        if parts.next().is_some() || !frac_valid_digits(n) || !frac_valid_digits(d) {
            return Err(bad());
        }
        crate::object::check_int_str_digit_limit(n, None)?;
        crate::object::check_int_str_digit_limit(d, None)?;
        let num: BigInt = n.replace('_', "").parse().map_err(|_| bad())?;
        let den: BigInt = d.replace('_', "").parse().map_err(|_| bad())?;
        return frac_normalize(if neg { -num } else { num }, den);
    }
    // Decimal/exponent form: `num[.frac][e|E[sign]exp]`.
    let (mantissa, exp10) = match rest.find(['e', 'E']) {
        Some(pos) => {
            let exp_str = &rest[pos + 1..];
            let exp_neg = exp_str.starts_with('-');
            let exp_clean = exp_str.strip_prefix(['-', '+']).unwrap_or(exp_str);
            if !frac_valid_digits(exp_clean) {
                return Err(bad());
            }
            crate::object::check_int_str_digit_limit(exp_clean, None)?;
            let exp_val: i64 = exp_clean.replace('_', "").parse().map_err(|_| bad())?;
            (&rest[..pos], if exp_neg { -exp_val } else { exp_val })
        }
        None => (rest, 0),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mantissa, None),
    };
    match frac_part {
        Some(f) => {
            // `3.`, `.5` allowed; `3..5` etc. are caught because the second
            // dot isn't a digit (frac_valid_digits rejects non-digits).
            if !int_part.is_empty() && !frac_valid_digits(int_part) {
                return Err(bad());
            }
            if !f.is_empty() && !frac_valid_digits(f) {
                return Err(bad());
            }
            if int_part.is_empty() && f.is_empty() {
                return Err(bad());
            }
            crate::object::check_int_str_digit_limit(int_part, None)?;
            crate::object::check_int_str_digit_limit(f, None)?;
        }
        None => {
            if !frac_valid_digits(int_part) {
                return Err(bad());
            }
            crate::object::check_int_str_digit_limit(int_part, None)?;
        }
    }
    let int_digits = if int_part.is_empty() { "0" } else { int_part };
    let frac_clean = frac_part.unwrap_or("").replace('_', "");
    let frac_len = frac_clean.len();
    let combined = format!("{}{}", int_digits.replace('_', ""), frac_clean);
    let num_digits: BigInt = combined.parse().map_err(|_| bad())?;
    let scale = -(frac_len as i64);
    let neg = neg || mantissa.starts_with('-');
    let total_exp = scale + exp10;
    let (mut num, den) = if total_exp >= 0 {
        (
            num_digits * BigInt::from(10).pow(total_exp as u32),
            BigInt::one(),
        )
    } else {
        (num_digits, BigInt::from(10).pow((-total_exp) as u32))
    };
    if neg {
        num = -num;
    }
    frac_normalize(num, den)
}

pub(crate) fn frac_instance_num_den(v: &PyObjectRef) -> Option<(BigInt, BigInt)> {
    if let PyObject::Instance { dict, .. } = &*v.borrow() {
        let num = dict.get_str("_numerator")?;
        let den = dict.get_str("_denominator")?;
        let get = |o: &PyObjectRef| -> Option<BigInt> {
            match &*o.borrow() {
                PyObject::Int(n) => Some(n.clone()),
                // `_from_coprime_ints` stores the raw objects (an int
                // subclass like DummyIntegral) — read their int backing.
                PyObject::Instance { .. } => crate::object::int_value_or_backing(o),
                _ => None,
            }
        };
        if let (Some(n), Some(d)) = (get(&num), get(&den)) {
            return Some((n, d));
        }
    }
    None
}

fn frac_make(frac_type: &PyObjectRef, num: BigInt, den: BigInt) -> PyResult<PyObjectRef> {
    let (num, den) = frac_normalize(num, den)?;
    let mut dict = AttrMap::new();
    dict.insert_str("_numerator", py_int(num));
    dict.insert_str("_denominator", py_int(den));
    Ok(PyObjectRef::new(PyObject::Instance {
        typ: frac_type.clone(),
        dict,
    }))
}

/// Numeric operand kind for Fraction arithmetic's real-Python coercion
/// rules: `Fraction op int` stays a `Fraction`; `Fraction op float` (or
/// vice versa) coerces the WHOLE operation to plain `float` (matching real
/// `Fraction.__add__`'s own documented behavior); anything else is
/// `NotImplemented` (deferring to the other operand's reflected method).
enum FracOperand {
    Frac(BigInt, BigInt),
    Float(f64),
    Other,
}

fn frac_operand_of(v: &PyObjectRef) -> FracOperand {
    if let Some((n, d)) = frac_instance_num_den(v) {
        return FracOperand::Frac(n, d);
    }
    let b = v.borrow();
    match &*b {
        PyObject::Int(i) => FracOperand::Frac(i.clone(), BigInt::one()),
        PyObject::Bool(bv) => FracOperand::Frac(BigInt::from(*bv as i64), BigInt::one()),
        PyObject::Float(f) => FracOperand::Float(*f),
        PyObject::Instance { .. } => {
            // An `numbers.Rational`-registered class (e.g. the test suite's
            // `Rat` / `Root`) exposes numerator/denominator attributes —
            // Fraction arithmetic/comparison accepts these.
            drop(b);
            if let (Ok(num), Ok(den)) = (
                v.borrow().get_attribute("numerator"),
                v.borrow().get_attribute("denominator"),
            ) {
                let n = crate::object::int_value_or_backing(&num)
                    .or_else(|| crate::object::to_index(&num).ok());
                let d = crate::object::int_value_or_backing(&den)
                    .or_else(|| crate::object::to_index(&den).ok());
                if let (Some(n), Some(d)) = (n, d) {
                    return FracOperand::Frac(n, d);
                }
            }
            FracOperand::Other
        }
        _ => FracOperand::Other,
    }
}

/// True iff `other` is an exact `int`/`Fraction` (or subclass) — the only
/// Rationals a FORWARD Fraction arithmetic op handles directly (CPython's
/// `_operator_fallbacks` monomorphic arm); everything else defers to the
/// other operand's reflected method.
fn frac_forward_ok(other: &PyObjectRef) -> bool {
    if matches!(&*other.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
        return true;
    }
    if let PyObject::Instance { typ, .. } = &*other.borrow() {
        if typ.is(&get_fraction_type()) {
            return true;
        }
        if let PyObject::Type { mro, .. } = &*typ.borrow() {
            return mro.iter().skip(1).any(|b| b.is(&get_fraction_type()));
        }
    }
    false
}

/// Reflected-op fallback for a `numbers.Real` operand (CPython's reverse
/// `isinstance(a, numbers.Real) -> float(a) op float(self)` arm): invoke the
/// operand's `__float__` and compute `float_op(other_f, self_f)`. Decimal
/// is deliberately excluded (it has `__float__` but refuses mixed arithmetic).
fn frac_reflected_float<F: Fn(f64, f64) -> f64>(
    other: &PyObjectRef,
    self_f: f64,
    float_op: F,
) -> Option<PyObjectRef> {
    if instance_to_decval(other).is_some() {
        return None;
    }
    let f = other.borrow().get_attribute("__float__").ok()?;
    let result = crate::object::call_bound_method(f, other.clone(), vec![]).ok()?;
    let af = result.as_f64()?;
    Some(py_float(float_op(af, self_f)))
}

/// Reflected-op fallback for a `numbers.Complex` operand (CPython's reverse
/// `isinstance(a, numbers.Complex) -> complex(a) op complex(self)` arm):
/// invoke the operand's `__complex__` and apply `op` to its parts.
fn frac_reflected_complex<F: Fn((f64, f64), (f64, f64)) -> (f64, f64)>(
    other: &PyObjectRef,
    self_f: f64,
    op: F,
) -> Option<PyObjectRef> {
    if instance_to_decval(other).is_some() {
        return None;
    }
    let f = other.borrow().get_attribute("__complex__").ok()?;
    let result = crate::object::call_bound_method(f, other.clone(), vec![]).ok()?;
    let parts = crate::object::as_complex_parts(&*result.borrow())?;
    let (cr, ci) = op(parts, (self_f, 0.0));
    Some(PyObjectRef::imm(PyObject::Complex(cr, ci)))
}

/// True iff `other` is a real `complex` (or a complex-subclass instance) —
/// CPython's forward `isinstance(b, complex)` arm.
fn frac_is_complex_operand(other: &PyObjectRef) -> bool {
    if matches!(&*other.borrow(), PyObject::Complex(..)) {
        return true;
    }
    if let PyObject::Instance { typ, .. } = &*other.borrow() {
        return crate::object::native_base_of_type(typ).as_deref() == Some("complex");
    }
    false
}

/// Just the float value of a `numbers.Real`-style operand (its `__float__`).
fn frac_reflected_float_value(other: &PyObjectRef) -> Option<f64> {
    let f = other.borrow().get_attribute("__float__").ok()?;
    let result = crate::object::call_bound_method(f, other.clone(), vec![]).ok()?;
    result.as_f64()
}

fn frac_self_num_den(self_obj: &PyObjectRef) -> PyResult<(BigInt, BigInt)> {
    frac_instance_num_den(self_obj).ok_or_else(|| PyError::type_error("not a Fraction"))
}

/// Python `float(a) ** float(b)`: a negative base with a non-integral
/// exponent yields a complex result (e.g. `(-1.0) ** 0.5` -> 1j).
fn frac_float_pow(base: f64, exp: f64) -> PyObjectRef {
    if base < 0.0 && exp.fract() != 0.0 {
        let mag = (-base).powf(exp);
        let theta = std::f64::consts::PI * exp;
        PyObjectRef::imm(PyObject::Complex(mag * theta.cos(), mag * theta.sin()))
    } else {
        py_float(base.powf(exp))
    }
}

/// Rational `a ** power` for an integer power (CPython's Fraction.__pow__
/// integer branch): a non-negative power raises num/den; a negative power
/// inverts, raising ZeroDivisionError for a zero base.
fn frac_rational_pow(an: &BigInt, ad: &BigInt, power: &BigInt) -> PyResult<(BigInt, BigInt)> {
    let p = power.to_u32();
    match p {
        Some(p) => Ok((an.pow(p), ad.pow(p))),
        None if !an.is_zero() => {
            let p = (-power).to_u32().unwrap_or(u32::MAX);
            Ok((ad.pow(p), an.pow(p)))
        }
        None => {
            let p = (-power).to_u32().unwrap_or(u32::MAX);
            Err(PyError::ZeroDivisionError(format!(
                "Fraction({}, 0)",
                ad.pow(p)
            )))
        }
    }
}

pub(crate) fn frac_to_f64(num: &BigInt, den: &BigInt) -> f64 {
    if num.is_zero() {
        return 0.0;
    }
    if den.is_zero() {
        return f64::INFINITY;
    }
    let sign = if (num.sign() == num_bigint::Sign::Minus) != (den.sign() == num_bigint::Sign::Minus)
    {
        -1.0
    } else {
        1.0
    };
    let n = num.abs();
    let d = den.abs();
    // Keep ~54 bits of precision and scale both sides DOWN to fit in f64,
    // so huge numerators/denominators don't overflow to inf before dividing
    // (CPython's `int / int` true division semantics for
    // `Fraction.__float__` — `float(F(2*10**400, 3*10**400))` must round
    // to 2/3, not NaN).
    let prec = 54u64;
    let nbits = n.bits();
    let dbits = d.bits();
    let shift_n = nbits.saturating_sub(prec);
    let shift_d = dbits.saturating_sub(prec);
    let n2 = n >> shift_n;
    let d2 = d >> shift_d;
    let ratio = n2.to_f64().unwrap_or(f64::INFINITY) / d2.to_f64().unwrap_or(f64::INFINITY);
    sign * ratio * 2f64.powf(shift_n as f64 - shift_d as f64)
}

/// Exact comparison of `num/den` against an `f64` (CPython's Fraction/float
/// comparisons use the float's exact binary value, so `F(10**23) == 1e23`
/// is False). `None` when NaN is involved.
fn frac_cmp_exact(num: &BigInt, den: &BigInt, f: f64) -> Option<std::cmp::Ordering> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        // Every finite fraction is less than +inf and greater than -inf.
        return Some(if f.is_sign_positive() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        });
    }
    if num.is_zero() {
        return Some(0.0f64.partial_cmp(&f).unwrap_or(std::cmp::Ordering::Equal));
    }
    let (fn_, fd) = frac_float_to_ratio(f).ok()?;
    // Compare num/den with fn_/fd_ exactly (cross-multiplied).
    Some((num * &fd).cmp(&(&fn_ * den)))
}

/// Shared binary-op dispatcher: `op` combines two exact `(num, den)` pairs;
/// `float_op` combines two `f64`s for the mixed-with-float coercion case.
fn frac_binop(
    args: &[PyObjectRef],
    reflected: bool,
    op: impl Fn(BigInt, BigInt, BigInt, BigInt) -> PyResult<(BigInt, BigInt)>,
    float_op: impl Fn(f64, f64) -> f64,
    complex_op: fn((f64, f64), (f64, f64)) -> (f64, f64),
    py_op: fn(&PyObjectRef, &PyObjectRef) -> PyResult<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("expected 2 arguments"));
    }
    // `self` (args[0]) is always the Fraction whose method this is; for a
    // reflected call (`__radd__` etc.) `self` is semantically the RIGHT
    // operand of `other OP self`, so `op`'s arguments are swapped below
    // rather than swapping `an`/`ad` here.
    let (an, ad) = frac_self_num_den(&args[0])?;
    match frac_operand_of(&args[1]) {
        FracOperand::Frac(bn, bd) => {
            if !reflected && !frac_forward_ok(&args[1]) {
                return Ok(py_not_implemented());
            }
            let (rn, rd) = if reflected {
                op(bn, bd, an, ad)?
            } else {
                op(an, ad, bn, bd)?
            };
            frac_make(&get_fraction_type(), rn, rd)
        }
        FracOperand::Float(bf) => {
            let af = frac_to_f64(&an, &ad);
            Ok(py_float(if reflected {
                float_op(bf, af)
            } else {
                float_op(af, bf)
            }))
        }
        FracOperand::Other => {
            if reflected {
                if let Some(r) = frac_reflected_float(&args[1], frac_to_f64(&an, &ad), float_op) {
                    return Ok(r);
                }
                if let Some(r) = frac_reflected_complex(&args[1], frac_to_f64(&an, &ad), complex_op)
                {
                    return Ok(r);
                }
            } else if frac_is_complex_operand(&args[1]) {
                // `isinstance(b, complex)`: CPython's forward fallback
                // `fallback_operator(float(a), b)`.
                let af = frac_to_f64(&an, &ad);
                return py_op(&py_float(af), &args[1]);
            }
            Ok(py_not_implemented())
        }
    }
}
/// Python `divmod` (floor division): returns (q, r) with 0 <= r < |d| and
/// r matching d's sign for positive d (CPython's `divmod` semantics, which
/// the round-ties-to-even digit generation depends on for negative inputs).
fn floor_div_rem(n: BigInt, d: &BigInt) -> (BigInt, BigInt) {
    let q = &n / d;
    let r = &n % d;
    if r != BigInt::zero() && (r.sign() != d.sign()) {
        (q - 1, r + d)
    } else {
        (q, r)
    }
}

/// Round rational n/d to the nearest multiple of 10**exponent, ties-to-even
/// (port of CPython's fractions._round_to_exponent).
fn frac_round_to_exponent(n: BigInt, d: BigInt, exponent: i64) -> (bool, BigInt) {
    let (n, d) = if exponent >= 0 {
        (n, d * BigInt::from(10).pow(exponent as u32))
    } else {
        (n * BigInt::from(10).pow((-exponent) as u32), d)
    };
    let half = &d >> 1;
    let (mut q, r) = floor_div_rem(&n + &half, &d);
    if r == BigInt::zero() && (&d & BigInt::from(1)) == BigInt::zero() {
        q &= BigInt::from(-2);
    }
    let sign = n.sign() == num_bigint::Sign::Minus;
    (sign, q.abs())
}

/// Round rational n/d to `figures` significant figures (port of CPython's
/// fractions._round_to_figures).
fn frac_round_to_figures(n: BigInt, d: BigInt, figures: usize) -> (bool, BigInt, i64) {
    if n == BigInt::zero() {
        return (false, BigInt::zero(), 1 - figures as i64);
    }
    let str_n = n.abs().to_string();
    let str_d = d.to_string();
    let m = str_n.len() as i64 - str_d.len() as i64
        + if str_d.as_str() <= str_n.as_str() {
            1
        } else {
            0
        };
    let exponent = m - figures as i64;
    let (sign, mut significand) = frac_round_to_exponent(n, d, exponent);
    let mut exponent = exponent;
    if significand.to_string().len() as i64 == figures as i64 + 1 {
        significand /= 10;
        exponent += 1;
    }
    (sign, significand, exponent)
}

/// A parsed general (no-presentation-type) format spec — port of CPython's
/// `_GENERAL_FORMAT_SPECIFICATION_MATCHER`.
struct FracGeneralSpec {
    fill: char,
    align: char,
    sign: char,
    alt: bool,
    width: usize,
    thousands: Option<char>,
}

/// Parse a general format spec; `None` if the spec does not fullmatch
/// (in which case it should be tried as a float-style spec).
fn frac_parse_general_spec(spec: &str) -> Option<FracGeneralSpec> {
    let chars: Vec<char> = spec.chars().collect();
    let len = chars.len();
    let mut idx = 0;
    let mut fill = ' ';
    let mut align = '>';
    if idx + 1 < len && matches!(chars[idx + 1], '<' | '>' | '^' | '=') {
        fill = chars[idx];
        align = chars[idx + 1];
        idx += 2;
    } else if idx < len && matches!(chars[idx], '<' | '>' | '^' | '=') {
        align = chars[idx];
        idx += 1;
    }
    let mut sign = '-';
    if idx < len && matches!(chars[idx], '+' | '-' | ' ') {
        sign = chars[idx];
        idx += 1;
    }
    let mut alt = false;
    if idx < len && chars[idx] == '#' {
        alt = true;
        idx += 1;
    }
    let mut width = 0usize;
    if idx < len && chars[idx] == '0' {
        // '0' alone is a width; '0' followed by digits cannot fullmatch.
        if idx + 1 < len && chars[idx + 1].is_ascii_digit() {
            return None;
        }
        idx += 1;
    } else if idx < len && chars[idx].is_ascii_digit() {
        while idx < len && chars[idx].is_ascii_digit() {
            width = width * 10 + chars[idx].to_digit(10).unwrap() as usize;
            idx += 1;
        }
    }
    let mut thousands = None;
    if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        thousands = Some(chars[idx]);
        idx += 1;
    }
    if idx < len {
        return None;
    }
    Some(FracGeneralSpec {
        fill,
        align,
        sign,
        alt,
        width,
        thousands,
    })
}

fn frac_group_digits(s: &str, sep: Option<char>) -> String {
    match sep {
        None => s.to_string(),
        Some(sep) => {
            let mut g = String::new();
            let bytes: Vec<char> = s.chars().collect();
            for (i, c) in bytes.iter().enumerate() {
                if i > 0 && (bytes.len() - i) % 3 == 0 {
                    g.push(sep);
                }
                g.push(*c);
            }
            g
        }
    }
}

/// Format a Fraction with a general (no presentation type) spec — port of
/// CPython's `Fraction._format_general`.
fn frac_format_general(num: BigInt, den: &BigInt, p: &FracGeneralSpec) -> PyResult<String> {
    let pos_sign = if p.sign == '-' {
        String::new()
    } else {
        p.sign.to_string()
    };
    let sign_out = if num < BigInt::zero() {
        "-".to_string()
    } else {
        pos_sign
    };
    let an = num.abs();
    let body = if *den > BigInt::one() || p.alt {
        format!(
            "{}/{}",
            frac_group_digits(&an.to_string(), p.thousands),
            frac_group_digits(&den.to_string(), p.thousands)
        )
    } else {
        frac_group_digits(&an.to_string(), p.thousands)
    };
    let padding_len = p.width.saturating_sub(sign_out.len() + body.len());
    let padding = p.fill.to_string().repeat(padding_len);
    Ok(match p.align {
        '<' => format!("{}{}{}", sign_out, body, padding),
        '^' => {
            let half = padding_len / 2;
            format!(
                "{}{}{}{}",
                &padding[..half],
                sign_out,
                body,
                &padding[half..]
            )
        }
        '=' => format!("{}{}{}", sign_out, padding, body),
        _ => format!("{}{}{}", padding, sign_out, body),
    })
}

/// Format a Fraction exactly for float-style presentation types — port of
/// CPython's `Fraction._format_float_style`. `den` must be positive.
fn frac_format_exact(num: BigInt, den: BigInt, spec: &str) -> PyResult<String> {
    let chars: Vec<char> = spec.chars().collect();
    let len = chars.len();
    let mut idx = 0;
    let mut fill = ' ';
    let mut align = '>';
    let mut align_explicit = false;
    if idx + 1 < len && matches!(chars[idx + 1], '<' | '>' | '^' | '=') {
        fill = chars[idx];
        align = chars[idx + 1];
        align_explicit = true;
        idx += 2;
    } else if idx < len && matches!(chars[idx], '<' | '>' | '^' | '=') {
        align = chars[idx];
        align_explicit = true;
        idx += 1;
    }
    let mut sign = '-';
    if idx < len && matches!(chars[idx], '+' | '-' | ' ') {
        sign = chars[idx];
        idx += 1;
    }
    let mut no_neg_zero = false;
    if idx < len && chars[idx] == 'z' {
        no_neg_zero = true;
        idx += 1;
    }
    let mut alt = false;
    if idx < len && chars[idx] == '#' {
        alt = true;
        idx += 1;
    }
    let mut zeropad = false;
    if idx < len && chars[idx] == '0' && idx + 1 < len && chars[idx + 1].is_ascii_digit() {
        zeropad = true;
        idx += 1;
    }
    let mut width = 0usize;
    while idx < len && chars[idx].is_ascii_digit() {
        width = width * 10 + chars[idx].to_digit(10).unwrap() as usize;
        idx += 1;
    }
    let mut int_sep: Option<char> = None;
    if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        int_sep = Some(chars[idx]);
        idx += 1;
    }
    let mut precision = 6usize;
    let mut frac_sep: Option<char> = None;
    if idx < len && chars[idx] == '.' {
        idx += 1;
        // CPython requires the '.' to be followed by a digit or separator
        // ('.e', '.f' with no precision are invalid).
        if idx >= len || !(chars[idx].is_ascii_digit() || chars[idx] == ',' || chars[idx] == '_') {
            return Err(PyError::value_error(format!(
                "Invalid format specifier '{}' for object of type 'Fraction'",
                spec
            )));
        }
        let start = idx;
        while idx < len && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx > start {
            precision = chars[start..idx]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(6);
        }
        if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
            frac_sep = Some(chars[idx]);
            idx += 1;
        }
    }
    let ptype = if idx < len { chars[idx] } else { '\0' };
    if idx + 1 < len {
        return Err(PyError::value_error(format!(
            "Invalid format specifier '{}' for object of type 'Fraction'",
            spec
        )));
    }
    if !matches!(ptype, 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | '%') {
        return Err(PyError::value_error(format!(
            "Invalid format specifier '{}' for object of type 'Fraction'",
            spec
        )));
    }
    // Illegal to combine an explicit fill/alignment with zero padding
    // (CPython's "Refuse the temptation to guess" rule).
    if zeropad && align_explicit {
        return Err(PyError::value_error(format!(
            "Invalid format specifier '{}' for object of type 'Fraction'",
            spec
        )));
    }
    if align == '=' && fill == '0' {
        zeropad = true;
    }
    let pos_sign = if sign == '-' {
        String::new()
    } else {
        sign.to_string()
    };
    let trim_zeros = matches!(ptype, 'g' | 'G') && !alt;
    let trim_point = !alt;
    let exponent_indicator = if matches!(ptype, 'E' | 'F' | 'G') {
        "E"
    } else {
        "e"
    };

    let (negative, significand, exponent, point_pos, scientific): (bool, BigInt, i64, usize, bool) =
        if matches!(ptype, 'f' | 'F' | '%') {
            let mut exponent = -(precision as i64);
            if ptype == '%' {
                exponent -= 2;
            }
            let (neg, sig) = frac_round_to_exponent(num, den, exponent);
            (neg, sig, exponent, precision, false)
        } else {
            let figures = if matches!(ptype, 'g' | 'G') {
                precision.max(1)
            } else {
                precision + 1
            };
            let (neg, sig, exp) = frac_round_to_figures(num, den, figures);
            let scientific = matches!(ptype, 'e' | 'E') || exp > 0 || exp + figures as i64 <= -4;
            let point_pos = if scientific {
                figures - 1
            } else {
                (-exp) as usize
            };
            (neg, sig, exp, point_pos, scientific)
        };

    let suffix = if ptype == '%' {
        "%".to_string()
    } else if scientific {
        format!("{}{:+03}", exponent_indicator, exponent + point_pos as i64)
    } else {
        String::new()
    };

    let sig_str = significand.to_string();
    let negative = if no_neg_zero && significand.is_zero() {
        false
    } else {
        negative
    };
    let digits = format!("{:0>width$}", sig_str, width = point_pos + 1);
    let sign_out = if negative { "-" } else { &pos_sign };
    let leading = &digits[..digits.len() - point_pos];
    let mut frac_part = digits[digits.len() - point_pos..].to_string();
    if trim_zeros {
        frac_part = frac_part.trim_end_matches('0').to_string();
    }
    let separator = if trim_point && frac_part.is_empty() {
        ""
    } else {
        "."
    };
    let frac_part = if let Some(sep) = frac_sep {
        frac_part
            .chars()
            .collect::<Vec<char>>()
            .chunks(3)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(&sep.to_string())
    } else {
        frac_part
    };
    if separator.is_empty() && frac_part.is_empty() {
        // no-op
    }
    let mut leading = leading.to_string();
    if zeropad {
        // Zero-padding inserts '0's into the INTEGER DIGITS (before any
        // grouping) until sign + grouped digits + rest reaches the width.
        let rest_len = separator.len() + frac_part.len() + suffix.len();
        let sign_len = sign_out.len();
        let grouped_len = |d: usize| if d == 0 { 0 } else { d + (d - 1) / 3 };
        let d0 = leading.len();
        let mut d = d0;
        while sign_len + grouped_len(d) + rest_len < width {
            d += 1;
        }
        if d > d0 {
            leading = format!("{:0>width$}", leading, width = d);
        }
    }
    if let Some(sep) = int_sep {
        let mut g = String::new();
        let bytes: Vec<char> = leading.chars().collect();
        for (i, c) in bytes.iter().enumerate() {
            if i > 0 && (bytes.len() - i) % 3 == 0 {
                g.push(sep);
            }
            g.push(*c);
        }
        leading = g;
    }
    let body = format!(
        "{}{}{}{}{}",
        sign_out, leading, separator, frac_part, suffix
    );
    // Apply fill/align/width. Zero-padding (the '0' flag) pads with '0'
    // AFTER the sign, i.e. '=' alignment with fill '0'.
    if body.len() >= width {
        return Ok(body);
    }
    let pad = width - body.len();
    let eff_fill = if zeropad { '0' } else { fill };
    let eff_align = if zeropad && align != '<' && align != '^' {
        '='
    } else {
        align
    };
    if eff_align == '=' {
        let (prefix, rest) =
            if body.starts_with('-') || body.starts_with('+') || body.starts_with(' ') {
                body.split_at(1)
            } else {
                ("", body.as_str())
            };
        Ok(format!(
            "{}{}{}",
            prefix,
            eff_fill.to_string().repeat(pad),
            rest
        ))
    } else {
        match eff_align {
            '<' => Ok(format!("{}{}", body, eff_fill.to_string().repeat(pad))),
            '^' => {
                let left = pad / 2;
                let right = pad - left;
                Ok(format!(
                    "{}{}{}",
                    eff_fill.to_string().repeat(left),
                    body,
                    eff_fill.to_string().repeat(right)
                ))
            }
            _ => Ok(format!("{}{}", eff_fill.to_string().repeat(pad), body)),
        }
    }
}

/// Fallback that gets routed (by address, see vm.rs's call_function) to the
/// real `fraction_init_with_vm` — Fraction's constructor needs a live VM to
/// invoke user-provided `as_integer_ratio()` methods.
///
/// The address-based routing in call_function only fires for the *raw*
/// BuiltinFunction/BoundMethod objects; a bound copy produced through some
/// attribute-binding paths loses the original fn identity, so as a last
/// resort the fallback itself grabs the active VM via the VM_PTR
/// thread-local (always set while interpreter bytecode is running).
pub(crate) fn fraction_init_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(crate::object::with_vm_mut(|vm| fraction_init_with_vm(vm, args))??)
}

pub(crate) fn fraction_from_number_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(crate::object::with_vm_mut(|vm| fraction_from_number_with_vm(vm, args))??)
}

pub(crate) fn fraction_from_decimal_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(crate::object::with_vm_mut(|vm| fraction_from_decimal_with_vm(vm, args))??)
}

pub(crate) fn fraction_from_number_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "from_number() takes exactly 1 argument",
        ));
    }
    let cls = &args[0];
    let val = &args[1];
    let b = val.borrow();
    if matches!(&*b, PyObject::Str(_)) {
        return Err(PyError::type_error("argument should be a string or a Rational instance or have the as_integer_ratio() method"));
    }
    if matches!(&*b, PyObject::Complex(..)) {
        return Err(PyError::type_error("argument should be a string or a Rational instance or have the as_integer_ratio() method"));
    }
    drop(b);
    let (n, d) = frac_ctor_value(vm, val)?;
    frac_make(cls, n, d)
}

pub(crate) fn fraction_from_decimal_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "from_decimal() takes exactly 1 argument",
        ));
    }
    let cls = &args[0];
    let val = &args[1];
    let (n, d) = frac_ctor_value(vm, val)?;
    frac_make(cls, n, d)
}

/// Bind a raw class-dict value (as returned by `get_attribute`) to `obj`,
/// mirroring LOAD_ATTR's own auto-binding for instance method calls.
fn frac_bind_method(
    found: &PyObjectRef,
    obj: &PyObjectRef,
    typ: &PyObjectRef,
) -> Option<PyObjectRef> {
    let b = found.borrow();
    match &*b {
        PyObject::StaticMethod { func } => Some(func.clone()),
        PyObject::ClassMethod { func } => Some(PyObjectRef::imm(PyObject::BoundMethod {
            func: func.clone(),
            self_obj: typ.clone(),
        })),
        PyObject::Function(_) => Some(PyObjectRef::imm(PyObject::BoundMethod {
            func: found.clone(),
            self_obj: obj.clone(),
        })),
        PyObject::BuiltinFunction { name, func } => {
            Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.clone(),
                func: *func,
                self_obj: obj.clone(),
            }))
        }
        PyObject::BuiltinMethod { name, func, .. } => {
            Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.clone(),
                func: *func,
                self_obj: obj.clone(),
            }))
        }
        _ => None,
    }
}

/// One Rational operand for `Fraction(a, b)` / `Fraction(x)`: exact ints,
/// bools, Fraction instances, or any non-type object exposing
/// `as_integer_ratio()` (floats, Decimals, user Ratios, ...).
fn frac_ctor_value(
    vm: &mut crate::vm::VirtualMachine,
    obj: &PyObjectRef,
) -> PyResult<(BigInt, BigInt)> {
    if let PyObject::Int(n) = &*obj.borrow() {
        return Ok((n.clone(), BigInt::one()));
    }
    if let PyObject::Bool(b) = &*obj.borrow() {
        return Ok((BigInt::from(*b as i64), BigInt::one()));
    }
    if let Some((n, d)) = frac_instance_num_den(obj) {
        return Ok((n, d));
    }
    if let FracOperand::Float(f) = frac_operand_of(obj) {
        return frac_float_to_ratio(f);
    }
    if let PyObject::Str(s) = &*obj.borrow() {
        return frac_parse_str(s);
    }
    // The `numbers.Rational` protocol: read `.numerator` / `.denominator`
    // attributes (properties included) on arbitrary non-type objects —
    // checking the INSTANCE dict first (a `Rat`-style class stores them as
    // plain attributes), then the type dict/property resolution.
    if !matches!(&*obj.borrow(), PyObject::Type { .. }) {
        let (num, den) = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            let num = dict.get_str("numerator").cloned();
            let den = dict.get_str("denominator").cloned();
            if num.is_some() && den.is_some() {
                (num, den)
            } else {
                (
                    vm.resolve_descriptor_attr(obj, "numerator"),
                    vm.resolve_descriptor_attr(obj, "denominator"),
                )
            }
        } else {
            (
                vm.resolve_descriptor_attr(obj, "numerator"),
                vm.resolve_descriptor_attr(obj, "denominator"),
            )
        };
        if let (Some(num), Some(den)) = (num, den) {
            let n = crate::object::int_value_or_backing(&num)
                .or_else(|| crate::object::to_index(&num).ok())
                .or_else(|| {
                    num.as_f64().and_then(|f| {
                        if f.is_finite() {
                            Some(BigInt::from(f as i64))
                        } else {
                            None
                        }
                    })
                });
            let d = crate::object::int_value_or_backing(&den)
                .or_else(|| crate::object::to_index(&den).ok());
            if let (Some(n), Some(d)) = (n, d) {
                return Ok((n, d));
            }
        }
    }
    let is_type = matches!(&*obj.borrow(), PyObject::Type { .. });
    if !is_type {
        // An `as_integer_ratio` stored directly in the INSTANCE dict is
        // already bound (no `self` gets prepended on call) — a lambda like
        // `a.as_integer_ratio = lambda: (9, 5)`.
        let instance_attr = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            dict.get_str("as_integer_ratio").cloned()
        } else {
            None
        };
        let as_integer_ratio: Option<PyObjectRef> = if let Some(found) = instance_attr {
            Some(found)
        } else {
            let typ = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                Some(typ.clone())
            } else {
                None
            };
            if let (Some(typ), Ok(found)) = (typ, obj.borrow().get_attribute("as_integer_ratio")) {
                frac_bind_method(&found, obj, &typ)
            } else {
                None
            }
        };
        if let Some(bound) = as_integer_ratio {
            let result = vm.call_function(bound, vec![], vec![])?;
            let b = result.borrow();
            if let PyObject::Tuple(items) = &*b {
                if items.len() != 2 {
                    let msg = if items.len() < 2 {
                        format!(
                            "not enough values to unpack (expected 2, got {})",
                            items.len()
                        )
                    } else {
                        "too many values to unpack (expected 2)".to_string()
                    };
                    drop(b);
                    return Err(PyError::value_error(msg));
                }
                let num = crate::object::int_value_or_backing(&items[0])
                    .or_else(|| crate::object::to_index(&items[0]).ok())
                    .ok_or_else(|| {
                        PyError::type_error("as_integer_ratio() must return a pair of integers")
                    })?;
                let den = crate::object::int_value_or_backing(&items[1])
                    .or_else(|| crate::object::to_index(&items[1]).ok())
                    .ok_or_else(|| {
                        PyError::type_error("as_integer_ratio() must return a pair of integers")
                    })?;
                drop(b);
                return Ok((num, den));
            }
            drop(b);
            return Err(PyError::type_error(
                "cannot unpack non-iterable type from as_integer_ratio()",
            ));
        }
    }
    Err(PyError::type_error(
        "argument should be a string or a Rational instance or have the as_integer_ratio() method",
    ))
}

/// Raw numerator/denominator OBJECTS for a constructor operand — exact ints/
/// bools/Fractions/floats/strings become plain ints, but an int-subclass or
/// registered-Rational operand keeps its `.numerator`/`.denominator` objects
/// as-is (CPython stores these raw, so `F(myint(3), myint(6)).numerator` is
/// a `myint`).
fn frac_ctor_raw(
    vm: &mut crate::vm::VirtualMachine,
    obj: &PyObjectRef,
    allow_as_integer_ratio: bool,
    strict_rational: bool,
) -> PyResult<Option<(PyObjectRef, PyObjectRef)>> {
    if let PyObject::Int(n) = &*obj.borrow() {
        return Ok(Some((py_int(n.clone()), py_int(1))));
    }
    if let PyObject::Bool(b) = &*obj.borrow() {
        return Ok(Some((py_int(*b as i64), py_int(1))));
    }
    if let Some((n, d)) = frac_instance_num_den(obj) {
        return Ok(Some((py_int(n), py_int(d))));
    }
    if !strict_rational {
        if let FracOperand::Float(f) = frac_operand_of(obj) {
            let (n, d) = frac_float_to_ratio(f)?;
            return Ok(Some((py_int(n), py_int(d))));
        }
        if let PyObject::Str(s) = &*obj.borrow() {
            let (n, d) = frac_parse_str(s)?;
            return Ok(Some((py_int(n), py_int(d))));
        }
    }
    if !matches!(&*obj.borrow(), PyObject::Type { .. }) {
        let (num, den) = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            let num = dict.get_str("numerator").cloned();
            let den = dict.get_str("denominator").cloned();
            if num.is_some() && den.is_some() {
                (num, den)
            } else {
                (
                    vm.resolve_descriptor_attr(obj, "numerator"),
                    vm.resolve_descriptor_attr(obj, "denominator"),
                )
            }
        } else {
            (
                vm.resolve_descriptor_attr(obj, "numerator"),
                vm.resolve_descriptor_attr(obj, "denominator"),
            )
        };
        if let (Some(num), Some(den)) = (num, den) {
            return Ok(Some((num, den)));
        }
        // `as_integer_ratio` (instance-dict lambdas stay unbound) — only the
        // SINGLE-argument constructor form accepts these; the two-arg form
        // requires real Rational instances (CPython: `F(Ratio((3,7)), 11)`
        // raises TypeError).
        // raises TypeError).
        if allow_as_integer_ratio {
            let instance_attr = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                dict.get_str("as_integer_ratio").cloned()
            } else {
                None
            };
            let as_integer_ratio: Option<PyObjectRef> = if let Some(found) = instance_attr {
                Some(found)
            } else {
                let typ = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    Some(typ.clone())
                } else {
                    None
                };
                if let (Some(typ), Ok(found)) =
                    (typ, obj.borrow().get_attribute("as_integer_ratio"))
                {
                    frac_bind_method(&found, obj, &typ)
                } else {
                    None
                }
            };
            if let Some(bound) = as_integer_ratio {
                let result = vm.call_function(bound, vec![], vec![])?;
                let b = result.borrow();
                if let PyObject::Tuple(items) = &*b {
                    if items.len() != 2 {
                        let msg = if items.len() < 2 {
                            format!(
                                "not enough values to unpack (expected 2, got {})",
                                items.len()
                            )
                        } else {
                            "too many values to unpack (expected 2)".to_string()
                        };
                        drop(b);
                        return Err(PyError::value_error(msg));
                    }
                    let num = crate::object::int_value_or_backing(&items[0])
                        .or_else(|| crate::object::to_index(&items[0]).ok())
                        .ok_or_else(|| {
                            PyError::type_error("as_integer_ratio() must return a pair of integers")
                        })?;
                    let den = crate::object::int_value_or_backing(&items[1])
                        .or_else(|| crate::object::to_index(&items[1]).ok())
                        .ok_or_else(|| {
                            PyError::type_error("as_integer_ratio() must return a pair of integers")
                        })?;
                    drop(b);
                    return Ok(Some((py_int(num), py_int(den))));
                }
                drop(b);
                return Err(PyError::type_error(
                    "cannot unpack non-iterable type from as_integer_ratio()",
                ));
            }
        }
    }
    Ok(None)
}

/// Normalize raw numerator/denominator objects to lowest terms with a
/// positive denominator, dividing the RAW objects by the gcd (CPython's
/// `numerator //= g; denominator //= g` on the stored objects).
fn frac_normalize_raw(
    num: &PyObjectRef,
    den: &PyObjectRef,
) -> PyResult<(PyObjectRef, PyObjectRef)> {
    let ni = crate::object::int_value_or_backing(num).or_else(|| crate::object::to_index(num).ok());
    let di = crate::object::int_value_or_backing(den).or_else(|| crate::object::to_index(den).ok());
    let (ni, di) = match (ni, di) {
        (Some(n), Some(d)) => (n, d),
        _ => return Ok((num.clone(), den.clone())),
    };
    if di.is_zero() {
        return Err(PyError::ZeroDivisionError(format!("Fraction({}, 0)", ni)));
    }
    let g_pos = frac_bigint_gcd(&ni, &di);
    let g = if di.sign() == num_bigint::Sign::Minus {
        -g_pos
    } else {
        g_pos
    };
    // CPython divides ALWAYS (`numerator //= g`), which is a no-op for g == 1
    // but flips the sign for a negative g.
    let num = crate::object::py_floor_div(num, &py_int(g.clone()))?;
    let den = crate::object::py_floor_div(den, &py_int(g))?;
    Ok((num, den))
}

/// Fraction's real constructor (CPython's `Fraction.__new__`): single-arg
/// int / Rational / float / string / as_integer_ratio object, or the
/// two-arg numerator/denominator (each an int or Rational) form.
pub(crate) fn fraction_init_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__init__ requires self"));
    }
    let rest = &args[1..];
    let (num, den): (PyObjectRef, PyObjectRef) = match rest.len() {
        0 => (py_int(0), py_int(1)),
        1 => frac_ctor_raw(vm, &rest[0], true, false)?.ok_or_else(|| {
            PyError::type_error(
                "argument should be a string or a Rational instance or have the as_integer_ratio() method",
            )
        })?,
        2 => {
            let (an, ad) = frac_ctor_raw(vm, &rest[0], false, true)?.ok_or_else(|| {
                PyError::type_error("both arguments should be Rational instances")
            })?;
            let (bn, bd) = frac_ctor_raw(vm, &rest[1], false, true)?.ok_or_else(|| {
                PyError::type_error("both arguments should be Rational instances")
            })?;
            let num = crate::object::py_mul(&an, &bd)?;
            let den = crate::object::py_mul(&ad, &bn)?;
            frac_normalize_raw(&num, &den)?
        }
        _ => return Err(PyError::type_error("Fraction() takes at most 2 arguments")),
    };
    if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
        // Immutable: a re-run `r.__init__(...)` on an already-built
        // Fraction is a no-op (CPython's slots-based Fraction).
        if dict.get_str("_numerator").is_some() {
            return Ok(py_none());
        }
        dict.insert_str("_numerator", num);
        dict.insert_str("_denominator", den);
    }
    Ok(py_none())
}

pub fn create_fractions_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let mut frac_dict: HashMap<String, PyObjectRef> = HashMap::new();

    // `Fraction.from_float(f)` / `Fraction._from_coprime_ints(n, d)` —
    // classmethods: LOAD_ATTR binds the calling class as args[0].
    frac_dict.insert_str(
        "from_float",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_float".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("from_float() takes exactly 1 argument"));
                    }
                    let cls = &args[0];
                    let val = &args[1];
                    let vb = val.borrow();
                    if let PyObject::Int(n) = &*vb {
                        // An int argument is just Fraction(int).
                        return frac_make(cls, n.clone(), BigInt::one());
                    }
                    drop(vb);
                    let f = val
                        .as_f64()
                        .ok_or_else(|| PyError::type_error("argument should be a float"))?;
                    let (num, den) = frac_float_to_ratio(f)?;
                    frac_make(cls, num, den)
                },
            }),
        }),
    );
    frac_dict.insert_str(
        "from_number",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_number".to_string(),
                func: fraction_from_number_fallback,
            }),
        }),
    );
    frac_dict.insert_str(
        "from_decimal",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_decimal".to_string(),
                func: fraction_from_decimal_fallback,
            }),
        }),
    );
    frac_dict.insert_str(
        "_from_coprime_ints",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "_from_coprime_ints".to_string(),
                func: |args| {
                    if args.len() < 3 {
                        return Err(PyError::type_error(
                            "_from_coprime_ints() takes exactly 2 arguments",
                        ));
                    }
                    let cls = &args[0];
                    // Store the raw objects (CPython keeps them as-is) so
                    // `x.numerator` is the actual argument — but validate they are
                    // integers (or indexable / int-subclass instances).
                    let _ = crate::object::int_value_or_backing(&args[1])
                        .or_else(|| crate::object::to_index(&args[1]).ok())
                        .ok_or_else(|| PyError::type_error("numerator must be an integer"))?;
                    let _ = crate::object::int_value_or_backing(&args[2])
                        .or_else(|| crate::object::to_index(&args[2]).ok())
                        .ok_or_else(|| PyError::type_error("denominator must be an integer"))?;
                    let mut dict = AttrMap::new();
                    dict.insert_str("_numerator", args[1].clone());
                    dict.insert_str("_denominator", args[2].clone());
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: cls.clone(),
                        dict,
                    }))
                },
            }),
        }),
    );

    // A plain `__init__`, NOT `NATIVE_VALUE_CTOR_KEY` — the latter is only
    // for types whose direct construction returns a raw NATIVE value
    // (`int(5)` returns `PyObject::Int`, never wrapped in an `Instance`;
    // see its own doc comment) and is called with the constructor's real
    // arguments directly, no class prepended. Fraction needs the OPPOSITE:
    // a genuine `PyObject::Instance` (so it participates in ordinary
    // Instance-based arithmetic/comparison dispatch), which is exactly
    // what the standard `__init__` convention already provides — the
    // general Type-call machinery creates a fresh empty `Instance` first,
    // THEN calls `__init__(self, *real_args)` on it, matching a plain
    // `class Fraction: def __init__(self, ...): ...`. (An earlier version
    // of this mistakenly used `NATIVE_VALUE_CTOR_KEY`, which — receiving
    // the raw args directly with no class arg at all — silently
    // misinterpreted the first REAL constructor argument as if it were
    // the class, corrupting every `Fraction(...)` call.)
    frac_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: fraction_init_fallback,
        }),
    );

    macro_rules! frac_method {
        ($name:expr, $func:expr) => {
            frac_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    frac_method!("__add__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| Ok((&an * &bd + &bn * &ad, ad * bd)),
        |a, b| a + b,
        |(ar, ai), (br, bi)| (ar + br, ai + bi),
        crate::object::py_add
    ));
    frac_method!("__radd__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| Ok((&an * &bd + &bn * &ad, ad * bd)),
        |a, b| a + b,
        |(ar, ai), (br, bi)| (ar + br, ai + bi),
        crate::object::py_add
    ));
    frac_method!("__sub__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| Ok((&an * &bd - &bn * &ad, ad * bd)),
        |a, b| a - b,
        |(ar, ai), (br, bi)| (ar - br, ai - bi),
        crate::object::py_sub
    ));
    frac_method!("__rsub__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| Ok((&an * &bd - &bn * &ad, ad * bd)),
        |a, b| a - b,
        |(ar, ai), (br, bi)| (ar - br, ai - bi),
        crate::object::py_sub
    ));
    frac_method!("__mul__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| Ok((an * bn, ad * bd)),
        |a, b| a * b,
        |(ar, ai), (br, bi)| (ar * br - ai * bi, ar * bi + ai * br),
        crate::object::py_mul
    ));
    frac_method!("__rmul__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| Ok((an * bn, ad * bd)),
        |a, b| a * b,
        |(ar, ai), (br, bi)| (ar * br - ai * bi, ar * bi + ai * br),
        crate::object::py_mul
    ));
    frac_method!("__truediv__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| {
            if bn.is_zero() {
                return Err(PyError::ZeroDivisionError(
                    "Fraction division by zero".to_string(),
                ));
            }
            Ok((an * bd, ad * bn))
        },
        |a, b| a / b,
        |(ar, ai), (br, bi)| {
            // Smith's algorithm (matching CPython's complex division).
            if br.abs() >= bi.abs() {
                let ratio = bi / br;
                let denom = br + bi * ratio;
                ((ar + ai * ratio) / denom, (ai - ar * ratio) / denom)
            } else {
                let ratio = br / bi;
                let denom = br * ratio + bi;
                ((ar * ratio + ai) / denom, (ai * ratio - ar) / denom)
            }
        },
        crate::object::py_div
    ));
    frac_method!("__rtruediv__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| {
            if bn.is_zero() {
                return Err(PyError::ZeroDivisionError(
                    "Fraction division by zero".to_string(),
                ));
            }
            Ok((an * bd, ad * bn))
        },
        |a, b| a / b,
        |(ar, ai), (br, bi)| {
            // Smith's algorithm (matching CPython's complex division).
            if br.abs() >= bi.abs() {
                let ratio = bi / br;
                let denom = br + bi * ratio;
                ((ar + ai * ratio) / denom, (ai - ar * ratio) / denom)
            } else {
                let ratio = br / bi;
                let denom = br * ratio + bi;
                ((ar * ratio + ai) / denom, (ai * ratio - ar) / denom)
            }
        },
        crate::object::py_div
    ));
    frac_method!("__floordiv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => {
                if !frac_forward_ok(&args[1]) {
                    return Ok(py_not_implemented());
                }
                if bn.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                Ok(py_int(floor_div_rem(&an * &bd, &(&ad * &bn)).0))
            }
            FracOperand::Float(bf) => {
                if bf == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float floor division by zero".to_string(),
                    ));
                }
                let af = frac_to_f64(&an, &ad);
                Ok(py_float((af / bf).floor()))
            }
            FracOperand::Other => Ok(py_not_implemented()),
        }
    });
    frac_method!("__rfloordiv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (bn, bd) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(an, ad) => {
                if an.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                Ok(py_int(floor_div_rem(&an * &bd, &(&ad * &bn)).0))
            }
            FracOperand::Float(af) => {
                if af == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float floor division by zero".to_string(),
                    ));
                }
                let bf = frac_to_f64(&bn, &bd);
                Ok(py_float((af / bf).floor()))
            }
            FracOperand::Other => {
                if let Some(r) =
                    frac_reflected_float(&args[1], frac_to_f64(&bn, &bd), |a, b| (a / b).floor())
                {
                    return Ok(r);
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__mod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => {
                if !frac_forward_ok(&args[1]) {
                    return Ok(py_not_implemented());
                }
                if bn.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction modulo by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let (rn, rd) = frac_normalize(&an * &bd - &bn * &ad * q, &ad * &bd)?;
                frac_make(&get_fraction_type(), rn, rd)
            }
            FracOperand::Float(bf) => crate::object::py_float_mod(frac_to_f64(&an, &ad), bf),
            FracOperand::Other => Ok(py_not_implemented()),
        }
    });
    frac_method!("__rmod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (bn, bd) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(an, ad) => {
                if an.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction modulo by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let (rn, rd) = frac_normalize(&an * &bd - &bn * &ad * q, &ad * &bd)?;
                frac_make(&get_fraction_type(), rn, rd)
            }
            FracOperand::Float(af) => crate::object::py_float_mod(af, frac_to_f64(&bn, &bd)),
            FracOperand::Other => {
                let mod_op = |a: f64, b: f64| -> f64 {
                    let rem = a % b;
                    if rem == 0.0 {
                        if b.is_sign_negative() {
                            -0.0
                        } else {
                            0.0
                        }
                    } else if (rem < 0.0) != (b < 0.0) {
                        rem + b
                    } else {
                        rem
                    }
                };
                if let Some(r) = frac_reflected_float(&args[1], frac_to_f64(&bn, &bd), mod_op) {
                    return Ok(r);
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__divmod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => {
                if !frac_forward_ok(&args[1]) {
                    return Ok(py_not_implemented());
                }
                if bn.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let r = frac_normalize(&an * &bd - &bn * &ad * &q, &ad * &bd)?;
                Ok(py_tuple(vec![
                    py_int(q),
                    frac_make(&get_fraction_type(), r.0, r.1)?,
                ]))
            }
            FracOperand::Float(bf) => {
                if bf == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float division by zero".to_string(),
                    ));
                }
                let af = frac_to_f64(&an, &ad);
                let q = (af / bf).floor();
                let r = crate::object::py_float_mod(af, bf)?;
                Ok(py_tuple(vec![py_float(q), r]))
            }
            FracOperand::Other => Ok(py_not_implemented()),
        }
    });
    frac_method!("__rdivmod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (bn, bd) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(an, ad) => {
                if an.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let r = frac_normalize(&an * &bd - &bn * &ad * &q, &ad * &bd)?;
                Ok(py_tuple(vec![
                    py_int(q),
                    frac_make(&get_fraction_type(), r.0, r.1)?,
                ]))
            }
            FracOperand::Float(af) => {
                if af == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float division by zero".to_string(),
                    ));
                }
                let bf = frac_to_f64(&bn, &bd);
                let q = (af / bf).floor();
                let r = crate::object::py_float_mod(af, bf)?;
                Ok(py_tuple(vec![py_float(q), r]))
            }
            FracOperand::Other => {
                if let Some(other_f) = frac_reflected_float_value(&args[1]) {
                    let bf = frac_to_f64(&bn, &bd);
                    if other_f != 0.0 {
                        let q = (other_f / bf).floor();
                        let rem = crate::object::py_float_mod(other_f, bf).ok();
                        return Ok(py_tuple(vec![
                            py_float(q),
                            rem.unwrap_or_else(|| py_float(f64::NAN)),
                        ]));
                    }
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__pow__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        // 3-argument `pow(base, exp, mod)` is not supported for Fraction.
        if args.len() >= 3 && !matches!(&*args[2].borrow(), PyObject::None) {
            return Ok(py_not_implemented());
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) if bd == BigInt::one() => {
                let (rn, rd) = frac_rational_pow(&an, &ad, &bn)?;
                frac_make(&get_fraction_type(), rn, rd)
            }
            FracOperand::Frac(bn, bd) => {
                Ok(frac_float_pow(frac_to_f64(&an, &ad), frac_to_f64(&bn, &bd)))
            }
            FracOperand::Float(bf) => Ok(frac_float_pow(frac_to_f64(&an, &ad), bf)),
            FracOperand::Other => {
                // CPython's `isinstance(b, (float, complex))` arm:
                // `float(a) ** b` (delegates to a complex(-subclass)
                // exponent's own `__rpow__`).
                if frac_is_complex_operand(&args[1]) {
                    return crate::object::py_pow(&py_float(frac_to_f64(&an, &ad)), &args[1]);
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__rpow__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        if args.len() >= 3 && !matches!(&*args[2].borrow(), PyObject::None) {
            return Ok(py_not_implemented());
        }
        // self is the EXPONENT b in `a ** b` (CPython's Fraction.__rpow__).
        let (bn, bd) = frac_self_num_den(&args[0])?;
        let a = &args[1];
        // b integer and >= 0: `a ** b.numerator` keeps an int base an int.
        if bd == BigInt::one() && bn.sign() != num_bigint::Sign::Minus {
            return crate::object::py_pow(a, &py_int(bn.clone()));
        }
        match frac_operand_of(a) {
            FracOperand::Frac(an, ad) => {
                // Rational base -> Fraction(base) ** b (integer b handled
                // exactly; non-integer b -> float).
                if bd == BigInt::one() {
                    let (rn, rd) = frac_rational_pow(&an, &ad, &bn)?;
                    frac_make(&get_fraction_type(), rn, rd)
                } else {
                    Ok(frac_float_pow(frac_to_f64(&an, &ad), frac_to_f64(&bn, &bd)))
                }
            }
            FracOperand::Float(af) => {
                if bd == BigInt::one() {
                    Ok(py_float(af.powi(bn.to_i32().unwrap_or(0))))
                } else {
                    Ok(frac_float_pow(af, frac_to_f64(&bn, &bd)))
                }
            }
            FracOperand::Other => {
                // CPython's `b.denominator == 1` arm: `a ** b.numerator`
                // (for non-Rational bases like a complex subclass, keeping
                // exactness where possible).
                if bd == BigInt::one() {
                    if let Ok(r) = crate::object::py_pow(a, &py_int(bn.clone())) {
                        if !crate::object::is_not_implemented(&r) {
                            return Ok(r);
                        }
                    }
                }
                // CPython's final `a ** float(b)` arm for Real/Complex bases.
                let bf = frac_to_f64(&bn, &bd);
                let f = a.borrow().get_attribute("__pow__").ok();
                if let Some(f) = f {
                    if let Ok(r) =
                        crate::object::call_bound_method(f, a.clone(), vec![py_float(bf)])
                    {
                        if !matches!(&*r.borrow(), PyObject::None) {
                            return Ok(r);
                        }
                    }
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__neg__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&get_fraction_type(), -n, d)
    });
    frac_method!("__pos__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&get_fraction_type(), n, d)
    });
    frac_method!("__abs__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&get_fraction_type(), n.abs(), d)
    });
    frac_method!("__float__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_float(frac_to_f64(&n, &d)))
    });
    frac_method!("__complex__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(PyObjectRef::imm(PyObject::Complex(
            frac_to_f64(&n, &d),
            0.0,
        )))
    });
    frac_method!("__int__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(n / d))
    });
    frac_method!("__trunc__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(n / d))
    });
    frac_method!("__floor__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(floor_div_rem(n, &d).0))
    });
    frac_method!("__ceil__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        let (q, r) = floor_div_rem(n, &d);
        Ok(py_int(if r.is_zero() { q } else { q + 1 }))
    });
    frac_method!("__round__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        let round_int = |num: &BigInt, den: &BigInt| -> BigInt {
            let q = floor_div_rem(num.clone(), den).0;
            let r: BigInt = num - &q * den;
            if &r * 2 < *den {
                q
            } else if &r * 2 > *den {
                q + 1
            } else if (&q % BigInt::from(2)).is_zero() {
                q
            } else {
                q + 1
            }
        };
        if args.len() < 2 || matches!(&*args[1].borrow(), PyObject::None) {
            return Ok(py_int(round_int(&n, &d)));
        }
        let ndigits = args[1].as_i64().ok_or_else(|| {
            PyError::type_error("__round__() argument 'ndigits' must be integral")
        })?;
        let shift = BigInt::from(10).pow(ndigits.unsigned_abs() as u32);
        let (rn, rd) = if ndigits > 0 {
            (round_int(&(n * &shift), &d), shift)
        } else {
            (round_int(&n, &(d * &shift)) * shift, BigInt::one())
        };
        frac_make(&get_fraction_type(), rn, rd)
    });
    frac_method!("limit_denominator", |args| {
        let max_den = if args.len() < 2 || matches!(&*args[1].borrow(), PyObject::None) {
            BigInt::from(1_000_000)
        } else {
            crate::object::int_value_or_backing(&args[1])
                .or_else(|| crate::object::to_index(&args[1]).ok())
                .ok_or_else(|| PyError::type_error("max_denominator should be an integer"))?
        };
        if max_den < BigInt::one() {
            return Err(PyError::value_error("max_denominator should be at least 1"));
        }
        let (n, d) = frac_self_num_den(&args[0])?;
        if d <= max_den {
            return Ok(args[0].clone());
        }
        // Continued-fraction nearest-fraction search (port of CPython's
        // Fraction.limit_denominator).
        let (orig_n, orig_d) = (n.clone(), d.clone());
        let (mut p0, mut q0) = (BigInt::zero(), BigInt::one());
        let (mut p1, mut q1) = (BigInt::one(), BigInt::zero());
        let (mut n, mut d) = (n, d);
        loop {
            let a = &n / &d;
            let q2 = &q0 + &a * &q1;
            if q2 > max_den {
                break;
            }
            let (np0, nq0) = (p1.clone(), q1.clone());
            p1 = &a * &p1 + &p0;
            q1 = q2;
            p0 = np0;
            q0 = nq0;
            let new_n = &n - &a * &d;
            n = d;
            d = new_n;
        }
        let k = (&max_den - &q0) / &q1;
        let b1n = &p0 + &k * &p1;
        let b1d = &q0 + &k * &q1;
        // Pick whichever candidate is closer to self (ties -> smaller
        // denominator, i.e. bound2), comparing cross-multiplied distances.
        let diff2 = (&p1 * &orig_d - &q1 * &orig_n).abs() * &b1d;
        let diff1 = (&b1n * &orig_d - &b1d * &orig_n).abs() * &q1;
        let (rn, rd) = if diff2 <= diff1 { (p1, q1) } else { (b1n, b1d) };
        frac_make(&get_fraction_type(), rn, rd)
    });
    frac_method!("__bool__", |args| {
        // CPython uses `bool(self._numerator)` — a raw (int-subclass /
        // registered-Rational) numerator's own `__bool__` is consulted.
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            if let Some(num) = dict.get_str("_numerator") {
                return Ok(py_bool(num.truthy()));
            }
        }
        let (n, _d) = frac_self_num_den(&args[0])?;
        Ok(py_bool(!n.is_zero()))
    });
    frac_method!("__repr__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_str(&format!("Fraction({}, {})", n, d)))
    });
    frac_method!("__str__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        if d == BigInt::one() {
            Ok(py_str(&n.to_string()))
        } else {
            Ok(py_str(&format!("{}/{}", n, d)))
        }
    });
    frac_method!("__format__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__format__ requires 1 argument"));
        }
        if matches!(&*args[1].borrow(), PyObject::None) {
            return Err(PyError::type_error(
                "format() argument 2 must be str, not NoneType",
            ));
        }
        let spec = args[1].str();
        if spec.is_empty() {
            return Ok(py_str(&args[0].str()));
        }
        let (n, d) = frac_self_num_den(&args[0])?;
        let d = if d == BigInt::zero() {
            BigInt::one()
        } else {
            d
        };
        // Specs without a float presentation type use CPython's
        // _format_general (sign/alt/thousands-sep over the str() form);
        // everything else is float-style. Try general first, like CPython.
        let out = match frac_parse_general_spec(&spec) {
            Some(parts) => frac_format_general(n, &d, &parts)?,
            None => frac_format_exact(n, d, &spec)?,
        };
        Ok(py_str(&out))
    });
    frac_method!("__hash__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        // CPython's _hash_algorithm: hash(|N| * dinv) mod 2**61-1, with INF
        // (314159) when the denominator has no modular inverse (is a multiple
        // of the modulus), signed by the numerator.
        let modulus = (BigInt::from(1i64) << 61) - BigInt::from(1);
        let hash_ = match crate::object::bigint_mod_inverse(&d, &modulus) {
            None => 314159i64, // _PyHASH_INF
            Some(inv) => {
                let abs_n_hash = crate::object::hash_bigint(&n.abs());
                let product = BigInt::from(abs_n_hash as i64) * inv;
                crate::object::hash_bigint(&product) as i64
            }
        };
        let result = if n.sign() == num_bigint::Sign::Minus {
            -hash_
        } else {
            hash_
        };
        Ok(py_int(if result == -1 { -2 } else { result }))
    });
    frac_method!("__eq__", |args| {
        if args.len() < 2 {
            return Ok(py_bool(false));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => Ok(py_bool(an == bn && ad == bd)),
            FracOperand::Float(bf) => Ok(py_bool(match frac_cmp_exact(&an, &ad, bf) {
                Some(o) => o.is_eq(),
                None => false,
            })),
            FracOperand::Other => {
                // CPython: `isinstance(b, Complex) and b.imag == 0` ->
                // compare against the real part as a float (exactly).
                let complex_val: Option<(f64, f64)> =
                    if let PyObject::Complex(re, im) = &*args[1].borrow() {
                        Some((*re, *im))
                    } else if frac_is_complex_operand(&args[1]) {
                        match args[1].borrow().get_attribute("__complex__") {
                            Ok(f) => crate::object::call_bound_method(f, args[1].clone(), vec![])
                                .ok()
                                .and_then(|c| {
                                    let cb = c.borrow();
                                    if let PyObject::Complex(re, im) = &*cb {
                                        Some((*re, *im))
                                    } else {
                                        None
                                    }
                                }),
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                if let Some((re, im)) = complex_val {
                    if im == 0.0 {
                        return Ok(py_bool(match frac_cmp_exact(&an, &ad, re) {
                            Some(o) => o.is_eq(),
                            None => false,
                        }));
                    }
                }
                Ok(py_not_implemented())
            }
        }
    });
    macro_rules! frac_cmp {
        ($name:expr, $cmp:expr) => {
            frac_method!($name, |args| {
                if args.len() < 2 {
                    return Ok(py_not_implemented());
                }
                let (an, ad) = frac_self_num_den(&args[0])?;
                match frac_operand_of(&args[1]) {
                    FracOperand::Frac(bn, bd) => Ok(py_bool($cmp((an * &bd).cmp(&(bn * &ad))))),
                    FracOperand::Float(bf) => {
                        match frac_cmp_exact(&an, &ad, bf) {
                            Some(o) => Ok(py_bool($cmp(o))),
                            // NaN involved: every ordered comparison is False.
                            None => Ok(py_bool(false)),
                        }
                    }
                    FracOperand::Other => Ok(py_not_implemented()),
                }
            });
        };
    }
    frac_cmp!("__lt__", |o: std::cmp::Ordering| o.is_lt());
    frac_cmp!("__le__", |o: std::cmp::Ordering| o.is_le());
    frac_cmp!("__gt__", |o: std::cmp::Ordering| o.is_gt());
    frac_cmp!("__ge__", |o: std::cmp::Ordering| o.is_ge());
    frac_method!("as_integer_ratio", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_tuple(vec![py_int(n), py_int(d)]))
    });
    // Read-only `numerator`/`denominator` properties backed by the
    // `_numerator`/`_denominator` slots — the raw stored objects are
    // returned (an int-subclass or registered-Rational `numerator` from the
    // constructor is preserved, matching CPython).
    frac_dict.insert_str(
        "numerator",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "numerator".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        if let Some(v) = dict.get_str("_numerator") {
                            return Ok(v.clone());
                        }
                    }
                    Err(PyError::runtime_error("fraction has no _numerator"))
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    frac_dict.insert_str(
        "denominator",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "denominator".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        if let Some(v) = dict.get_str("_denominator") {
                            return Ok(v.clone());
                        }
                    }
                    Err(PyError::runtime_error("fraction has no _denominator"))
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    // Slots semantics: only `_numerator`/`_denominator` (and the usual
    // instance internals) may be assigned; anything else raises
    // AttributeError (CPython's `Fraction.__slots__`).
    frac_dict.insert_str(
        "__slots__",
        py_tuple(vec![py_str("_numerator"), py_str("_denominator")]),
    );
    frac_method!("is_integer", |args| {
        let (_, d) = frac_self_num_den(&args[0])?;
        Ok(py_bool(d == BigInt::one()))
    });
    frac_method!("__reduce__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_tuple(vec![
            get_fraction_type(),
            py_tuple(vec![py_int(n), py_int(d)]),
        ]))
    });
    frac_method!("__copy__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
            if typ.is(&get_fraction_type()) {
                // Immutable: I am my own clone.
                return Ok(args[0].clone());
            }
            return frac_make(typ, n, d);
        }
        Ok(args[0].clone())
    });
    frac_method!("__deepcopy__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
            if typ.is(&get_fraction_type()) {
                return Ok(args[0].clone());
            }
            return frac_make(typ, n, d);
        }
        Ok(args[0].clone())
    });

    let frac_type = PyObjectRef::new(PyObject::Type {
        name: "Fraction".to_string(),
        dict: Box::new(str_map_to_typedict(frac_dict)),
        bases: vec![],
        mro: vec![],
    });
    // Register for `type.__subclasses__` / pickle's class lookup, with the
    // `__module__` attribute so `pickle.dumps(Fraction(...))` can resolve it.
    if let PyObject::Type { dict, .. } = &mut *frac_type.borrow_mut() {
        dict.insert_str("__module__", py_str("fractions"));
    }
    crate::object::register_class(&frac_type);
    FRACTION_TYPE.with(|c| {
        *c.borrow_mut() = Some(frac_type.clone());
    });
    d.insert_str("Fraction", frac_type);
    d
}

pub fn create_calendar_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cal_func {
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

    // Add constants to module.
    // `month_name`/`month_abbr` are 1-INDEXED in real CPython (`[0]` is a
    // deliberate empty-string placeholder, `[1]` = "January" .. `[12]` =
    // "December") — matching every other month-numbering convention in
    // Python (`date.month`, `time.tm_mon`, `strftime("%m")`, all 1-12).
    // Missing the `[0]` placeholder here meant `calendar.month_name[12]`
    // (December, the extremely common `month_name[some_real_month_number]`
    // idiom) actually returned November — an off-by-one silently giving
    // the WRONG month name for every single lookup, not a crash. Real
    // trigger: CPython's own `_strptime.py`, `[calendar.month_abbr[i] for i
    // in range(13)]` (deliberately ranging through 13 to include the
    // placeholder) raising `IndexError` outright once vendored, since the
    // 12-element list had no index 12 at all.
    d.insert_str(
        "month_name",
        py_list(vec![
            py_str(""),
            py_str("January"),
            py_str("February"),
            py_str("March"),
            py_str("April"),
            py_str("May"),
            py_str("June"),
            py_str("July"),
            py_str("August"),
            py_str("September"),
            py_str("October"),
            py_str("November"),
            py_str("December"),
        ]),
    );
    d.insert_str(
        "month_abbr",
        py_list(vec![
            py_str(""),
            py_str("Jan"),
            py_str("Feb"),
            py_str("Mar"),
            py_str("Apr"),
            py_str("May"),
            py_str("Jun"),
            py_str("Jul"),
            py_str("Aug"),
            py_str("Sep"),
            py_str("Oct"),
            py_str("Nov"),
            py_str("Dec"),
        ]),
    );
    d.insert_str(
        "day_name",
        py_list(vec![
            py_str("Monday"),
            py_str("Tuesday"),
            py_str("Wednesday"),
            py_str("Thursday"),
            py_str("Friday"),
            py_str("Saturday"),
            py_str("Sunday"),
        ]),
    );
    d.insert_str(
        "day_abbr",
        py_list(vec![
            py_str("Mon"),
            py_str("Tue"),
            py_str("Wed"),
            py_str("Thu"),
            py_str("Fri"),
            py_str("Sat"),
            py_str("Sun"),
        ]),
    );
    // Weekday constants (0=Monday..6=Sunday, matching `calendar.weekday()`'s
    // own return convention) — were missing entirely.
    d.insert_str("MONDAY", py_int(0));
    d.insert_str("TUESDAY", py_int(1));
    d.insert_str("WEDNESDAY", py_int(2));
    d.insert_str("THURSDAY", py_int(3));
    d.insert_str("FRIDAY", py_int(4));
    d.insert_str("SATURDAY", py_int(5));
    d.insert_str("SUNDAY", py_int(6));

    // Calendar helper functions (inner fn items are not captured by closures)
    fn is_leap(y: i64) -> bool {
        y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
    }
    fn month_days(y: i64, m: i64) -> i64 {
        match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if is_leap(y) {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        }
    }
    // Tomohiko Sakamoto's weekday algorithm: returns 0=Sunday, 1=Monday, ..., 6=Saturday
    fn weekday(y: i64, m: i64, d: i64) -> i64 {
        let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = if m < 3 { y - 1 } else { y };
        (y + y / 4 - y / 100 + y / 400 + t[m as usize - 1] + d) % 7
    }
    // First weekday of month: 0=Monday, 6=Sunday
    fn first_weekday(y: i64, m: i64) -> i64 {
        (weekday(y, m, 1) + 6) % 7
    }

    const MONTH_NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    // ---- HTMLCalendar factory ----
    cal_func!("HTMLCalendar", |args| {
        let _ = args;

        const HTML_DAY_CLASS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

        // formatmonth method
        let mut type_dict = HashMap::new();
        type_dict.insert_str("formatmonth", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "formatmonth".to_string(),
            func: |args| {
                if args.len() < 3 {
                    return Err(PyError::type_error("formatmonth() missing required arguments (self, year, month)"));
                }
                let y = args[1].as_i64().ok_or_else(|| PyError::type_error("year must be int"))?;
                let m = args[2].as_i64().ok_or_else(|| PyError::type_error("month must be int"))?;
                if m < 1 || m > 12 {
                    return Err(PyError::type_error("month must be in 1..12"));
                }

                let dim = month_days(y, m);
                let fd = first_weekday(y, m);

                let mut html = String::new();
                html.push_str("<table border=\"0\" cellpadding=\"0\" cellspacing=\"0\" class=\"month\">\n");
                html.push_str(&format!(
                    "<tr><th colspan=\"7\" class=\"month\">{} {}</th></tr>\n",
                    MONTH_NAMES[(m - 1) as usize], y
                ));
                html.push_str("<tr><th class=\"mon\">Mon</th><th class=\"tue\">Tue</th><th class=\"wed\">Wed</th>");
                html.push_str("<th class=\"thu\">Thu</th><th class=\"fri\">Fri</th><th class=\"sat\">Sat</th><th class=\"sun\">Sun</th></tr>\n");

                html.push_str("<tr>\n");
                for _ in 0..fd {
                    html.push_str("<td class=\"noday\">&nbsp;</td>");
                }
                for day in 1..=dim {
                    let wd = ((fd + day - 1) % 7) as usize;
                    html.push_str(&format!("<td class=\"{}\">{}</td>", HTML_DAY_CLASS[wd], day));
                    if (fd + day) % 7 == 0 && day != dim {
                        html.push_str("</tr>\n<tr>\n");
                    }
                }
                let remaining = (7 - (fd + dim) % 7) % 7;
                for _ in 0..remaining {
                    html.push_str("<td class=\"noday\">&nbsp;</td>");
                }
                html.push_str("</tr>\n</table>\n");
                Ok(py_str(&html))
            },
        }));

        // formatyear method
        type_dict.insert_str("formatyear", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "formatyear".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("formatyear() missing required arguments (self, year)"));
                }
                let y = args[1].as_i64().ok_or_else(|| PyError::type_error("year must be int"))?;

                let mut html = String::new();
                html.push_str(&format!("<table border=\"0\" cellpadding=\"0\" cellspacing=\"0\" class=\"year\">\n"));
                html.push_str(&format!("<tr><th colspan=\"3\" class=\"year\">{}</th></tr>\n", y));

                for q in 0..4 {
                    html.push_str("<tr>\n");
                    for m_idx in 0..3 {
                        let m = q * 3 + m_idx + 1;
                        let dim = month_days(y, m);
                        let fd = first_weekday(y, m);

                        html.push_str("<td>\n<table border=\"0\" cellpadding=\"0\" cellspacing=\"0\" class=\"month\">\n");
                        html.push_str(&format!(
                            "<tr><th colspan=\"7\" class=\"month\">{} {}</th></tr>\n",
                            MONTH_NAMES[(m - 1) as usize], y
                        ));
                        html.push_str("<tr><th class=\"mon\">Mon</th><th class=\"tue\">Tue</th><th class=\"wed\">Wed</th>");
                        html.push_str("<th class=\"thu\">Thu</th><th class=\"fri\">Fri</th><th class=\"sat\">Sat</th><th class=\"sun\">Sun</th></tr>\n");

                        html.push_str("<tr>\n");
                        for _ in 0..fd {
                            html.push_str("<td class=\"noday\">&nbsp;</td>");
                        }
                        for day in 1..=dim {
                            let wd = ((fd + day - 1) % 7) as usize;
                            html.push_str(&format!("<td class=\"{}\">{}</td>", HTML_DAY_CLASS[wd], day));
                            if (fd + day) % 7 == 0 && day != dim {
                                html.push_str("</tr>\n<tr>\n");
                            }
                        }
                        let remaining = (7 - (fd + dim) % 7) % 7;
                        for _ in 0..remaining {
                            html.push_str("<td class=\"noday\">&nbsp;</td>");
                        }
                        html.push_str("</tr>\n</table>\n</td>\n");
                        if m_idx < 2 {
                            html.push_str("<td>&nbsp;</td>\n");
                        }
                    }
                    html.push_str("</tr>\n");
                }
                html.push_str("</table>\n");
                Ok(py_str(&html))
            },
        }));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "HTMLCalendar".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        }))
    });

    // ---- TextCalendar factory ----
    cal_func!("TextCalendar", |args| {
        let _ = args;
        let mut type_dict = HashMap::new();
        type_dict.insert_str(
            "formatmonth",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "formatmonth".to_string(),
                func: |args| {
                    if args.len() < 3 {
                        return Err(PyError::type_error(
                            "formatmonth() missing required arguments (self, year, month)",
                        ));
                    }
                    let y = match args[1].as_i64() {
                        Some(i) => i,
                        None => return Err(PyError::type_error("year must be int")),
                    };
                    let m = match args[2].as_i64() {
                        Some(i) => i,
                        None => return Err(PyError::type_error("month must be int")),
                    };
                    if m < 1 || m > 12 {
                        return Err(PyError::type_error("month must be in 1..12"));
                    }
                    let dim = month_days(y, m);
                    let fd = first_weekday(y, m);
                    let mut lines = Vec::new();
                    lines.push(format!(
                        "{:>20}",
                        format!("{} {}", MONTH_NAMES[(m - 1) as usize], y)
                    ));
                    lines.push("Mo Tu We Th Fr Sa Su".to_string());
                    let mut week: Vec<String> = Vec::new();
                    for _ in 0..fd {
                        week.push("  ".to_string());
                    }
                    for day in 1..=dim {
                        week.push(format!("{:2}", day));
                        if week.len() == 7 {
                            lines.push(week.join(" "));
                            week.clear();
                        }
                    }
                    if !week.is_empty() {
                        while week.len() < 7 {
                            week.push("  ".to_string());
                        }
                        lines.push(week.join(" "));
                    }
                    Ok(py_str(&lines.join("\n")))
                },
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "TextCalendar".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        }))
    });

    // ---- Module-level calendar functions ----
    // `calendar.timegm(tuple)` — the inverse of `time.gmtime()`: given a
    // struct_time-shaped tuple (or any 6+-element sequence with year/month/
    // day/hour/min/sec in that order), return Unix seconds treating it as
    // UTC. Missing entirely — real trigger: CPython's own `http/cookiejar.py`
    // (`from calendar import timegm`), needed to convert a parsed
    // `Expires=` cookie header back into a comparable timestamp. Accepts
    // both a real `time.struct_time` (attribute-accessible, see
    // `modules/time.rs`) and a plain tuple, matching real `timegm`'s own
    // "any sequence" acceptance.
    cal_func!("timegm", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("timegm() missing required argument"));
        }
        let get = |i: usize, field: &str| -> i64 {
            match &*args[0].borrow() {
                PyObject::Instance { dict, .. } => {
                    dict.get(field).and_then(|v| v.as_i64()).unwrap_or(0)
                }
                PyObject::Tuple(items) | PyObject::List(items) => {
                    items.get(i).and_then(|v| v.as_i64()).unwrap_or(0)
                }
                _ => 0,
            }
        };
        let year = get(0, "tm_year");
        let month = get(1, "tm_mon");
        let mday = get(2, "tm_mday");
        let hour = get(3, "tm_hour");
        let minute = get(4, "tm_min");
        let second = get(5, "tm_sec");
        // Howard Hinnant civil-days-from-epoch algorithm (same one used by
        // `modules/time.rs`'s `civil_to_days`/`epoch_to_ymd`, duplicated
        // here rather than made cross-module-public since `calendar` and
        // `time` are populated by two separate, independent dict-builder
        // functions with no shared internal-helpers module).
        let y = if month <= 2 { year - 1 } else { year };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if month > 2 { month - 3 } else { month + 9 };
        let doy = (153 * mp + 2) / 5 + mday - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe - 719468;
        Ok(py_int(days * 86400 + hour * 3600 + minute * 60 + second))
    });

    cal_func!("isleap", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error(
                "isleap() missing required argument (year)",
            ));
        }
        let year = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        Ok(py_bool(is_leap(year)))
    });

    cal_func!("weekday", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "weekday() requires 3 arguments (year, month, day)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        let d = args[2]
            .as_i64()
            .ok_or_else(|| PyError::type_error("day must be integer"))?;
        // weekday returns 0=Monday, 6=Sunday
        let wd = (weekday(y, m, d) + 6) % 7;
        Ok(py_int(wd))
    });

    cal_func!("monthrange", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "monthrange() requires 2 arguments (year, month)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        if m < 1 || m > 12 {
            return Err(PyError::type_error("month must be in 1..12"));
        }
        let fd = first_weekday(y, m);
        let ndays = month_days(y, m);
        Ok(py_tuple(vec![py_int(fd), py_int(ndays)]))
    });

    cal_func!("monthcalendar", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "monthcalendar() requires 2 arguments (year, month)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        if m < 1 || m > 12 {
            return Err(PyError::type_error("month must be in 1..12"));
        }
        let dim = month_days(y, m);
        let fd = first_weekday(y, m);
        let mut weeks: Vec<PyObjectRef> = Vec::new();
        let mut week: Vec<PyObjectRef> = Vec::new();
        for _ in 0..fd {
            week.push(py_int(0));
        }
        for day in 1..=dim {
            week.push(py_int(day));
            if week.len() == 7 {
                weeks.push(py_list(week.clone()));
                week.clear();
            }
        }
        if !week.is_empty() {
            while week.len() < 7 {
                week.push(py_int(0));
            }
            weeks.push(py_list(week));
        }
        Ok(py_list(weeks))
    });

    cal_func!("prmonth", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "prmonth() requires 2 arguments (year, month)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        if m < 1 || m > 12 {
            return Err(PyError::type_error("month must be in 1..12"));
        }
        // Simplified text print
        println!("     {} {}", MONTH_NAMES[(m - 1) as usize], y);
        println!("Mo Tu We Th Fr Sa Su");
        let dim = month_days(y, m);
        let fd = first_weekday(y, m);
        for _ in 0..fd {
            print!("   ");
        }
        for day in 1..=dim {
            print!("{:2} ", day);
            if (fd + day) % 7 == 0 {
                println!();
            }
        }
        println!();
        Ok(py_none())
    });

    // `calendar.__all__` — same fix, same reason, as `operator.__all__`
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

// ── Native _random module (C extension stub for CPython's random.py) ──────
pub fn create_random_cmodule_dict() -> HashMap<String, PyObjectRef> {
    // Delegates to the faithful MT19937 implementation in rand.rs --
    // replaces the old LCG stub that backed Lib/random.py's pure-Python
    // generator (getrandbits(2**31) took effectively forever there).
    crate::modules::rand::create_random_dict()
}

use num_traits::ToPrimitive;
use std::rc::Rc;
