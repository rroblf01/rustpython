use crate::modules::create_collections_abc_dict;
use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;

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
        // Handle keyword-only args: rename, defaults, module
        let mut rename = false;
        let mut defaults_vals: Vec<PyObjectRef> = Vec::new();
        let mut module_val: Option<PyObjectRef> = None;
        if let Some(last) = args.last() {
            if let PyObject::Dict(d) = &*last.borrow() {
                let has_rename = d.get(&py_str("rename")).ok().flatten().is_some() || d.get(&py_str("rename")).is_ok() && d.get(&py_str("rename")).ok().is_some();
                // Use contains check via get
                let has_rename2 = d.get(&py_str("rename")).ok().flatten().is_some();
                let has_defaults = d.get(&py_str("defaults")).ok().flatten().is_some();
                let has_module = d.get(&py_str("module")).ok().flatten().is_some();
                // also check if dict contains those keys even if value is None/false
                let contains_rename = d.get(&py_str("rename")).is_ok();
                // Actually check via get with Ok
                let check_contains = |k: &str| d.get(&py_str(k)).is_ok();
                let cr = check_contains("rename");
                let cd = check_contains("defaults");
                let cm = check_contains("module");
                if cr || cd || cm || has_rename2 || has_defaults || has_module {
                    if args.len() > 3 {
                        return Err(PyError::type_error("namedtuple() takes 2 positional arguments but more were given"));
                    }
                    if let Ok(Some(v)) = d.get(&py_str("rename")) {
                        rename = v.truthy();
                    }
                    if let Ok(Some(v)) = d.get(&py_str("defaults")) {
                        if !matches!(&*v.borrow(), PyObject::None) {
                            match &*v.borrow() {
                                PyObject::List(items) | PyObject::Tuple(items) => {
                                    defaults_vals = items.clone();
                                }
                                _ => {
                                    if let Ok(collected) = crate::object::collect_iterable(&v) {
                                        defaults_vals = collected;
                                    }
                                }
                            }
                        }
                    }
                    if let Ok(Some(v)) = d.get(&py_str("module")) {
                        module_val = Some(v.clone());
                    }
                } else if args.len() > 2 {
                    return Err(PyError::type_error("namedtuple() takes 2 positional arguments but 3 were given"));
                }
            } else if args.len() > 2 {
                return Err(PyError::type_error("namedtuple() takes 2 positional arguments but 3 were given"));
            }
        } else if args.len() > 2 {
            return Err(PyError::type_error("namedtuple() takes 2 positional arguments but 3 were given"));
        }
        let mut fields = fields;
        if rename {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (i, f) in fields.iter_mut().enumerate() {
                let is_valid = !f.is_empty() && f.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) && f.chars().all(|c| c.is_alphanumeric() || c == '_') && !["def","class","return","if","for","while","import","from","as","pass","break","continue","and","or","not","in","is","lambda","with","yield","try","except","finally","raise","assert","del","global","nonlocal","True","False","None"].contains(&f.as_str()) && !f.starts_with('_');
                if !is_valid || seen.contains(f) {
                    *f = format!("_{}", i);
                }
                seen.insert(f.clone());
            }
        }
        if defaults_vals.len() > fields.len() {
            return Err(PyError::type_error("Too many defaults"));
        }
        let n = fields.len();
        let defaults_start = n.saturating_sub(defaults_vals.len());
        let f_clone = fields.clone();
        let tn_clone = typename.clone();
        let defaults_clone = defaults_vals.clone();
        // __init__: called by Type handler after creating empty Instance
        let init_fn = move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if args.len() < 1 {
                return Err(PyError::type_error("__init__ missing self"));
            }
            let self_obj = &args[0];
            let pos_args = &args[1..];
            let mut kwargs_map: std::collections::HashMap<String, PyObjectRef> = std::collections::HashMap::new();
            let mut actual_pos: Vec<PyObjectRef> = pos_args.to_vec();
            if let Some(last) = pos_args.last() {
                if let PyObject::Dict(d) = &*last.borrow() {
                    let mut is_kwargs = false;
                    for f in &f_clone {
                        if d.get(&py_str(f)).ok().flatten().is_some() {
                            is_kwargs = true;
                            break;
                        }
                    }
                    if is_kwargs {
                        for (k, v) in d.items() {
                            kwargs_map.insert(k.str(), v);
                        }
                        actual_pos.pop();
                    }
                }
            }
            if actual_pos.len() < defaults_start || actual_pos.len() > n {
                return Err(PyError::type_error(format!(
                    "{} expects {} arguments, got {}",
                    tn_clone,
                    n,
                    actual_pos.len()
                )));
            }
            let mut full_vals: Vec<PyObjectRef> = Vec::with_capacity(n);
            for i in 0..n {
                let field = &f_clone[i];
                if i < actual_pos.len() {
                    full_vals.push(actual_pos[i].clone());
                } else if let Some(v) = kwargs_map.get(field) {
                    full_vals.push(v.clone());
                } else if i >= defaults_start {
                    let idx = i - defaults_start;
                    full_vals.push(defaults_clone[idx].clone());
                } else {
                    return Err(PyError::type_error(format!("missing value for field {}", field)));
                }
            }
            for k in kwargs_map.keys() {
                if !f_clone.contains(k) {
                    return Err(PyError::type_error(format!("Got unexpected field names: {}", k)));
                }
            }
            for (i, f) in f_clone.iter().enumerate() {
                self_obj
                    .borrow_mut()
                    .set_attribute(f, full_vals[i].clone())
                    .ok();
            }
            self_obj
                .borrow_mut()
                .set_attribute(
                    "_fields",
                    PyObjectRef::new(PyObject::Tuple(f_clone.iter().map(|f| py_str(f)).collect())),
                )
                .ok();
            Ok(py_none())
        };
        let init_obj = PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(init_fn)));
        let mut type_dict = HashMap::new();
        type_dict.insert_str("__init__", init_obj);
        type_dict.insert_str("__slots__", py_tuple(vec![]));
        type_dict.insert_str("__match_args__", py_tuple(fields.iter().map(|f| py_str(f)).collect()));
        type_dict.insert_str("_fields", py_tuple(fields.iter().map(|f| py_str(f)).collect()));
        {
            let mut fd = crate::object::PyDict::new();
            for (i, f) in fields.iter().enumerate() {
                if i >= defaults_start {
                    let _ = fd.set(py_str(f), defaults_vals[i - defaults_start].clone());
                }
            }
            type_dict.insert_str("_field_defaults", PyObjectRef::new(PyObject::Dict(Box::new(fd))));
        }
        type_dict.insert_str("__module__", module_val.clone().unwrap_or_else(|| py_str("collections")));
        type_dict.insert_str("__doc__", py_str(&format!("{}({})", typename, fields.join(", "))));
        {
            let fields_for_setattr = fields.clone();
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if args.len() < 3 { return Err(PyError::type_error("__setattr__ requires 3 args")); }
                let name = args[1].str();
                if fields_for_setattr.contains(&name) {
                    return Err(PyError::attribute_error(format!("can't set attribute '{}'", name)));
                }
                args[0].borrow_mut().set_attribute(&name, args[2].clone())?;
                Ok(py_none())
            }))));
        }
        {
            let fields_for_del = fields.clone();
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if args.len() < 2 { return Err(PyError::type_error("__delattr__ requires 2 args")); }
                let name = args[1].str();
                if fields_for_del.contains(&name) {
                    return Err(PyError::attribute_error(format!("can't delete attribute '{}'", name)));
                }
                return Err(PyError::attribute_error(format!("can't delete attribute '{}'", name)));
            }))));
        }

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
                PyObject::Tuple(items) => Ok(items.iter().map(|v| v.str()).collect()),
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
                PyObjectRef::new(PyObject::Tuple(fields.iter().map(|f| py_str(f)).collect())),
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
                        PyObjectRef::new(PyObject::Tuple(
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
        let tuple_type = crate::object::get_primitive_type("tuple").unwrap_or_else(|| py_none());
        let typ = PyObjectRef::new(PyObject::Type {
            name: typename.clone(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: if matches!(&*tuple_type.borrow(), PyObject::Type { .. }) { vec![tuple_type.clone()] } else { vec![] },
            mro: vec![],
        });
        if matches!(&*tuple_type.borrow(), PyObject::Type { .. }) {
            if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
                *mro = vec![typ.clone(), tuple_type.clone()];
            }
        } else if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
            *mro = vec![typ.clone()];
        }
        Ok(typ)
    });

    // collections.abc submodule (Iterable, Hashable, etc.)
    d.insert_str(
        "abc",
        create_module("collections.abc", create_collections_abc_dict()),
    );

    d
}

/// UserList/UserDict/UserString source (like CPython's own collections.py).
/// Compiled and run once, post-construction, against the real VM — see
/// `VirtualMachine::install_collections_user_types` in vm.rs. Composition
/// over self.data works correctly for real subclassing (unlike inheriting
/// from the native list/dict/str types directly, which isn't supported).
pub const COLLECTIONS_USER_TYPES_SOURCE: &str = include_str!("../collections_user_types.py");
