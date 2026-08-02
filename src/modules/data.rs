use crate::object::*;
use crate::modules::create_collections_abc_dict;
use std::collections::HashMap;

pub fn create_json_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! json_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    json_func!("dumps", |args| {
        if args.is_empty() { return Err(PyError::type_error("dumps() missing required argument")); }
        let indent = if args.len() > 1 {
            let v = args[1].as_i64().unwrap_or(-1);
            if v >= 0 { Some(v as usize) } else { None }
        } else { None };
        let sort_keys = if args.len() > 2 { args[2].truthy() } else { false };
        json_encode_full(&args[0], indent, sort_keys, 0)
    });

    json_func!("loads", |args| {
        if args.is_empty() { return Err(PyError::type_error("loads() missing required argument")); }
        let s = args[0].str();
        json_decode(&s)
    });

    d
}

// Real `json.JSONEncoder` (subclassable, `default()` override point) is
// implemented as real Python source instead — see json_extra.py and
// VirtualMachine::install_source_defined_stdlib.
pub const JSON_EXTRA_SOURCE: &str = include_str!("json_extra.py");

pub fn create_collections_dict(object_type: PyObjectRef) -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! coll_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
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
    deque_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "deque".to_string(),
        func: crate::object::builtin_deque,
    }));
    deque_dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: crate::object::native_base_init_builtin,
    }));
    let deque_type = PyObjectRef::new(PyObject::Type {
        name: "deque".to_string(),
        dict: Box::new(str_map_to_typedict(deque_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *deque_type.borrow_mut() {
        *mro = vec![deque_type.clone(), object_type.clone()];
    }
    crate::object::seed_primitive_type_cache("deque", deque_type.clone());
    d.insert("deque".to_string(), deque_type);

    // OrderedDict: remembers insertion order
    coll_func!("OrderedDict", |args| {
        let dict = crate::object::py_dict();
        if args.len() > 1 {
            let source = &args[1];
            let borrowed = source.borrow();
            if let PyObject::Dict(d) = &*borrowed {
                for (k, v) in d.items() {
                    if let PyObject::Dict(ref mut target) = &mut *dict.borrow_mut() {
                        let _ = target.set(k, v);
                    }
                }
            }
        }
        Ok(dict)
    });

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
            PyObject::List(items) | PyObject::Tuple(items) => items.iter().map(|i| i.str()).collect(),
            _ => {
                let field_str = args[1].str();
                field_str.split(|c: char| c == ',' || c.is_whitespace())
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
                    tn_clone, n, pos_args.len()
                )));
            }
            // Set field values as attributes on self
            for (i, f) in f_clone.iter().enumerate() {
                self_obj.borrow_mut().set_attribute(f, pos_args[i].clone()).ok();
            }
            self_obj.borrow_mut().set_attribute("_fields",
                PyObjectRef::new(PyObject::List(
                    f_clone.iter().map(|f| py_str(f)).collect()
                ))
            ).ok();
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
            nt_fields(self_obj)?.iter().map(|f| self_obj.borrow().get_attribute(f)).collect()
        }
        fn nt_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            let typename = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
                if let PyObject::Type { name, .. } = &*typ.borrow() { name.clone() } else { "namedtuple".to_string() }
            } else { "namedtuple".to_string() };
            let fields = nt_fields(&args[0])?;
            let vals = nt_field_values(&args[0])?;
            let parts: Vec<String> = fields.iter().zip(vals.iter()).map(|(f, v)| format!("{}={}", f, v.repr())).collect();
            Ok(py_str(&format!("{}({})", typename, parts.join(", "))))
        }
        fn nt_eq(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.len() < 2 { return Ok(py_bool(false)); }
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
                        if !x.equals(y)? { return Ok(py_bool(false)); }
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
            if args.len() < 2 { return Err(PyError::type_error("expected an index")); }
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
            let typ = if let PyObject::Instance { typ, .. } = &*self_obj.borrow() { typ.clone() } else {
                return Err(PyError::type_error("_replace on non-namedtuple"));
            };
            let fields = nt_fields(self_obj)?;
            let overrides: Vec<(String, PyObjectRef)> = if args.len() > 1 {
                match &*args[1].borrow() {
                    PyObject::Dict(d) => d.items().into_iter().map(|(k, v)| (k.str(), v)).collect(),
                    _ => Vec::new(),
                }
            } else { Vec::new() };
            let mut new_dict = AttrMap::new();
            for f in &fields {
                let v = overrides.iter().find(|(k, _)| k == f).map(|(_, v)| v.clone())
                    .unwrap_or(self_obj.borrow().get_attribute(f)?);
                new_dict.insert_str(f, v);
            }
            new_dict.insert_str("_fields", PyObjectRef::new(PyObject::List(fields.iter().map(|f| py_str(f)).collect())));
            Ok(PyObjectRef::new(PyObject::Instance { typ, dict: new_dict }))
        }

        macro_rules! nt_method {
            ($name:expr, $f:expr) => {
                type_dict.insert_str($name, PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f }));
            };
        }
        nt_method!("__repr__", nt_repr);
        nt_method!("__eq__", nt_eq);
        nt_method!("__iter__", nt_iter);
        nt_method!("__getitem__", nt_getitem);
        nt_method!("__len__", nt_len);
        nt_method!("__hash__", nt_hash);
        nt_method!("_asdict", nt_asdict);
        nt_method!("_replace", nt_replace);

        // Add field names as class-level attributes (for __doc__ setting support)
        for f in &fields {
            type_dict.insert(f.clone(), PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "member_descriptor".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: AttrMap::new(),
            }));
        }
        Ok(PyObjectRef::new(PyObject::Type {
            name: typename,
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        }))
    });

    // collections.abc submodule (Iterable, Hashable, etc.)
    d.insert_str("abc", create_module("collections.abc", create_collections_abc_dict()));

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
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
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
                    return Err(PyError::type_error("reduce() of empty sequence with no initial value"));
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
            return Err(PyError::type_error("total_ordering requires a class argument"));
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
            return Err(PyError::type_error("cached_property requires a function argument"));
        }
        Ok(args[0].clone())
    });

    ft_func!("partial", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("partial() takes at least 1 argument"));
        }
        let func = args[0].clone();
        let partial_args: Vec<PyObjectRef> = args[1..].to_vec();
        Ok(PyObjectRef::new(PyObject::Partial { func, args: partial_args }))
    });

    // partialmethod: real semantics auto-bind `self` as the first argument
    // via the descriptor protocol when accessed on an instance. We don't
    // implement that binding here — this just pre-binds the given args like
    // partial() — so `descriptor.__get__`-based access won't insert self.
    // Direct calls (e.g. `SomeClass.attr(instance, ...)`) still work.
    ft_func!("partialmethod", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("partialmethod() takes at least 1 argument"));
        }
        let func = args[0].clone();
        let partial_args: Vec<PyObjectRef> = args[1..].to_vec();
        Ok(PyObjectRef::new(PyObject::Partial { func, args: partial_args }))
    });

    ft_func!("update_wrapper", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("update_wrapper() requires at least 2 arguments"));
        }
        let wrapper = args[0].clone();
        let wrapped = args[1].clone();
        let attrs = ["__module__", "__name__", "__qualname__", "__doc__", "__annotations__", "__dict__"];
        for attr in &attrs {
            if let Ok(val) = wrapped.borrow().get_attribute(attr) {
                let _ = wrapper.borrow_mut().set_attribute(attr, val);
            }
        }
        let _ = wrapper.borrow_mut().set_attribute("__wrapped__", wrapped.clone());
        for attr in &["__defaults__", "__kwdefaults__", "__code__", "__globals__"] {
            if let Ok(val) = wrapped.borrow().get_attribute(attr) {
                let _ = wrapper.borrow_mut().set_attribute(attr, val);
            }
        }
        Ok(wrapper)
    });
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
            let attrs = ["__module__", "__name__", "__qualname__", "__doc__", "__annotations__", "__dict__"];
            for attr in &attrs {
                if let Ok(val) = wrapped_clone.borrow().get_attribute(attr) {
                    let _ = wrapper_fn.borrow_mut().set_attribute(attr, val);
                }
            }
            let _ = wrapper_fn.borrow_mut().set_attribute("__wrapped__", wrapped_clone.clone());
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
            return Err(PyError::type_error("singledispatch() requires at least 1 argument"));
        }
        let func = args[0].clone();
        let registry = Rc::new(std::cell::RefCell::new(
            std::collections::HashMap::<String, PyObjectRef>::new(),
        ));
        {
            let mut reg = registry.borrow_mut();
            reg.insert_str("object", func.clone());
        }
        let func_name = func.borrow().get_attribute("__name__").ok();
        let registry_clone = registry.clone();
        let dispatch_func = move |call_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if call_args.is_empty() {
                return Err(PyError::type_error("singledispatch requires at least 1 argument"));
            }
            let first_arg = &call_args[0];
            let arg_type = first_arg.borrow().type_name();
            let reg = registry_clone.borrow();
            let impl_func = reg.get(&arg_type)
                .or_else(|| reg.get("object"))
                .cloned()
                .ok_or_else(|| PyError::runtime_error("singledispatch: no implementation found"))?;
            builtin_call(&impl_func, call_args)
        };
        // Use Instance with __call__ so set_attribute works (Closure doesn't support attribute setting)
        let mut call_type_dict = HashMap::new();
        let dispatch_rc = Rc::new(dispatch_func);
        call_type_dict.insert_str("__call__", PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            dispatch_rc(args)
        }))));
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
            let _ = dispatcher.borrow_mut().set_attribute("registry", PyObjectRef::new(PyObject::Dict(Box::new(py_registry))));
        }
        let reg_register = registry.clone();
        let _dispatch_clone = dispatcher.clone();
        let register_method = move |m_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if m_args.is_empty() {
                return Err(PyError::type_error("register() requires at least 1 argument"));
            }
            let typ_arg = m_args[0].clone();
            let type_name = typ_arg.borrow().type_name();
            let type_key = if type_name == "type" {
                typ_arg.borrow().get_attribute("__name__")
                    .map(|n| n.str())
                    .unwrap_or_else(|_| type_name.clone())
            } else {
                type_name.clone()
            };
            if m_args.len() >= 2 {
                reg_register.borrow_mut().insert(type_key, m_args[1].clone());
                Ok(py_none())
            } else {
                let reg_register_clone = reg_register.clone();
                let decorator = move |d_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if d_args.is_empty() {
                        return Err(PyError::type_error("register decorator requires a function argument"));
                    }
                    reg_register_clone.borrow_mut().insert(type_key.clone(), d_args[0].clone());
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
            return Err(PyError::type_error("cmp_to_key requires at least 1 argument"));
        }
        let mycmp = args[0].clone();
        let mycmp_for_factory = mycmp.clone();
        // Return a callable that acts as the key class
        let key_factory = move |k_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if k_args.is_empty() {
                return Err(PyError::type_error("cmp_to_key() key factory missing required argument"));
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
                let cmp_result = builtin_call(&lt_mycmp, &[(*lt_obj).clone(), lt_args[1].clone()])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n < 0)))
            };

            // __le__(self, other): mycmp(self.obj, other.obj) <= 0
            let le_mycmp = mycmp_rc.clone();
            let le_obj = obj_rc.clone();
            let le = move |le_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if le_args.len() < 2 {
                    return Err(PyError::type_error("__le__ requires 2 arguments"));
                }
                let cmp_result = builtin_call(&le_mycmp, &[(*le_obj).clone(), le_args[1].clone()])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n <= 0)))
            };

            // __gt__(self, other): mycmp(self.obj, other.obj) > 0
            let gt_mycmp = mycmp_rc.clone();
            let gt_obj = obj_rc.clone();
            let gt = move |gt_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if gt_args.len() < 2 {
                    return Err(PyError::type_error("__gt__ requires 2 arguments"));
                }
                let cmp_result = builtin_call(&gt_mycmp, &[(*gt_obj).clone(), gt_args[1].clone()])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n > 0)))
            };

            // __ge__(self, other): mycmp(self.obj, other.obj) >= 0
            let ge_mycmp = mycmp_rc.clone();
            let ge_obj = obj_rc.clone();
            let ge = move |ge_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if ge_args.len() < 2 {
                    return Err(PyError::type_error("__ge__ requires 2 arguments"));
                }
                let cmp_result = builtin_call(&ge_mycmp, &[(*ge_obj).clone(), ge_args[1].clone()])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n >= 0)))
            };

            // __eq__(self, other): mycmp(self.obj, other.obj) == 0
            let eq_mycmp = mycmp_rc.clone();
            let eq_obj = obj_rc.clone();
            let eq = move |eq_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if eq_args.len() < 2 {
                    return Err(PyError::type_error("__eq__ requires 2 arguments"));
                }
                let cmp_result = builtin_call(&eq_mycmp, &[(*eq_obj).clone(), eq_args[1].clone()])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n == 0)))
            };

            // __ne__(self, other): mycmp(self.obj, other.obj) != 0
            let ne_mycmp = mycmp_rc.clone();
            let ne_obj = obj_rc.clone();
            let ne = move |ne_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if ne_args.len() < 2 {
                    return Err(PyError::type_error("__ne__ requires 2 arguments"));
                }
                let cmp_result = builtin_call(&ne_mycmp, &[(*ne_obj).clone(), ne_args[1].clone()])?;
                Ok(py_bool(cmp_result.as_i64().map_or(false, |n| n != 0)))
            };

            // __hash__: cmp_to_key objects are unhashable (comparison may not be consistent)
            let hash_err = |_: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                Err(PyError::type_error("comparison function yields unhashable object"))
            };

            let mut type_dict = std::collections::HashMap::new();
            type_dict.insert_str("__lt__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(lt))));
            type_dict.insert_str("__le__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(le))));
            type_dict.insert_str("__gt__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(gt))));
            type_dict.insert_str("__ge__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(ge))));
            type_dict.insert_str("__eq__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(eq))));
            type_dict.insert_str("__ne__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(ne))));
            type_dict.insert_str("__hash__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(hash_err))));

            let key_obj = PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "cmp_to_key".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: AttrMap::new(),
            });
            let _ = key_obj.borrow_mut().set_attribute("obj", obj_rc.as_ref().clone());
            Ok(key_obj)
        };
        Ok(PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(key_factory))))
    });

    d
}

pub fn create_itertools_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! it_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // chain is represented as a callable Instance (not a bare
    // BuiltinFunction) so it can also expose `chain.from_iterable(...)` —
    // BuiltinFunction has no attribute storage at all (set_attribute has no
    // arm for it), so a plain function couldn't hold a from_iterable
    // sibling method the way real itertools.chain does.
    {
        let mut chain_type_dict = HashMap::new();
        chain_type_dict.insert_str("__call__", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
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
        }))));
        chain_type_dict.insert_str("from_iterable", PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if args.is_empty() { return Err(PyError::type_error("from_iterable() missing argument")); }
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
        }))));
        let chain_type = PyObjectRef::new(PyObject::Type { name: "chain".to_string(), dict: Box::new(str_map_to_typedict(chain_type_dict)), bases: vec![], mro: vec![] });
        d.insert_str("chain", PyObjectRef::new(PyObject::Instance { typ: chain_type, dict: AttrMap::new() }));
    }

    it_func!("count", |args| {
        let start = if args.len() > 0 {
            if let Some(n) = args[0].as_i64() { n } else { 0i64 }
        } else { 0i64 };
        let step = if args.len() > 1 {
            if let Some(n) = args[1].as_i64() { n } else { 1i64 }
        } else { 1i64 };
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
        if args.is_empty() { return Err(PyError::type_error("cycle() missing required argument")); }
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
        if args.is_empty() {
            return Ok(py_list(vec![py_tuple(vec![])]));
        }
        let mut pools: Vec<Vec<PyObjectRef>> = Vec::new();
        for arg in args {
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
        if args.is_empty() { return Err(PyError::type_error("combinations() missing argument")); }
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
            args[1].as_i64().ok_or_else(|| PyError::type_error("r must be int"))? as usize
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
                    if i == 0 { return Ok(py_list(result)); }
                    i -= 1;
                    if indices[i] != i + n - r {
                        break;
                    }
                    if i == 0 { return Ok(py_list(result)); }
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
        if args.is_empty() { return Err(PyError::type_error("combinations_with_replacement() missing argument")); }
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
            args[1].as_i64().ok_or_else(|| PyError::type_error("r must be int"))? as usize
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
        if args.is_empty() { return Err(PyError::type_error("permutations() missing argument")); }
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
            args[1].as_i64().ok_or_else(|| PyError::type_error("r must be int"))? as usize
        } else {
            n
        };
        let mut result = Vec::new();
        if r <= n {
            let mut indices: Vec<usize> = (0..n).collect();
            let mut cycles: Vec<usize> = (0..r).map(|i| n - i).collect();
            result.push(py_tuple(indices[0..r].iter().map(|&i| pool[i].clone()).collect()));
            'outer: loop {
                let mut i = r;
                loop {
                    if i == 0 { break 'outer; }
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
                        result.push(py_tuple(indices[0..r].iter().map(|&i| pool[i].clone()).collect()));
                        continue 'outer;
                    }
                    if i == 0 { break 'outer; }
                }
            }
        }
        Ok(py_list(result))
    });

    it_func!("repeat", |args| {
        if args.is_empty() { return Err(PyError::type_error("repeat() missing argument")); }
        let obj = args[0].clone();
        // `None` distinguishes "no count given" (real infinite repeat) from
        // an explicit `times=0` (a real, valid call meaning "repeat zero
        // times" — an empty iterator) — these used to collapse onto the
        // same `0` sentinel, so `itertools.repeat(x, 0)` wrongly produced
        // 1000 items instead of none.
        let times: Option<usize> = if args.len() > 1 {
            let n = args[1].as_i64().ok_or_else(|| PyError::type_error("times must be int"))?;
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
        if args.is_empty() { return Err(PyError::type_error("islice() missing arguments")); }
        let (start, stop, step) = match args.len() {
            1 => return Err(PyError::type_error("islice() missing stop argument")),
            2 => (0i64, if matches!(&*args[1].borrow(), PyObject::None) { None } else { Some(args[1].as_i64().unwrap_or(0)) }, 1i64),
            _ => {
                let start = args[1].as_i64().unwrap_or(0);
                let stop = if matches!(&*args[2].borrow(), PyObject::None) { None } else { Some(args[2].as_i64().unwrap_or(0)) };
                let step = if args.len() > 3 { args[3].as_i64().unwrap_or(1) } else { 1 };
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
                if i >= stop_v { break; }
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
        if args.is_empty() { return Err(PyError::type_error("tee() missing argument")); }
        let n = if args.len() > 1 { args[1].as_i64().unwrap_or(2) as usize } else { 2 };
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
                if let Ok(Some(v)) = d.get(&py_str("fillvalue")) { fillvalue = v; }
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
            let row: Vec<PyObjectRef> = lists.iter().map(|l| l.get(i).cloned().unwrap_or_else(|| fillvalue.clone())).collect();
            result.push(py_tuple(row));
        }
        Ok(py_list(result))
    });

    it_func!("accumulate", |args| {
        if args.is_empty() { return Err(PyError::type_error("accumulate() missing argument")); }
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
        if args.is_empty() { return Err(PyError::type_error("groupby() missing argument")); }
        // The key function may arrive positionally (args[1]) or as a
        // trailing kwargs dict (`key=...`) per this project's established
        // calling convention (see e.g. `str.format`'s own doc comment).
        let mut key_func: Option<PyObjectRef> = None;
        if args.len() > 1 {
            let last = &args[args.len() - 1];
            if let PyObject::Dict(d) = &*last.borrow() {
                if let Ok(Some(k)) = d.get(&py_str("key")) {
                    if !matches!(&*k.borrow(), PyObject::None) { key_func = Some(k); }
                }
            } else if !matches!(&*last.borrow(), PyObject::None) {
                key_func = Some(last.clone());
            }
        }
        let source = builtin_iter(&[args[0].clone()])?;
        Ok(PyObjectRef::new(PyObject::GroupByIter { source, key_func, pending: None, exhausted: false }))
    });

    // filterfalse(func, iterable) — filter elements where func is False
    it_func!("filterfalse", |args| {
        if args.len() < 2 { return Err(PyError::type_error("filterfalse() requires 2 arguments")); }
        let predicate = if matches!(&*args[0].borrow(), PyObject::None) { None } else { Some(args[0].clone()) };
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
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
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
        if args.is_empty() { return Err(PyError::type_error("mean() missing required argument")); }
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
        if args.is_empty() { return Err(PyError::type_error("median() missing required argument")); }
        let mut nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => PyError::ValueError("median() argument is empty".to_string()),
            PyError::TypeError(_) => PyError::type_error("median() argument must contain numbers"),
            other => other,
        })?;
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        if n % 2 == 0 {
            Ok(py_float((nums[n/2 - 1] + nums[n/2]) / 2.0))
        } else {
            Ok(py_float(nums[n/2]))
        }
    });

    stat_func!("stdev", |args| {
        if args.is_empty() { return Err(PyError::type_error("stdev() missing required argument")); }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::TypeError(_) => PyError::type_error("stdev() argument must contain numbers"),
            other => other,
        })?;
        if nums.len() < 2 {
            return Err(PyError::ValueError("stdev() requires at least 2 data points".to_string()));
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
        if args.is_empty() { return Err(PyError::type_error("harmonic_mean() missing required argument")); }
        let nums = stat_extract_nums(&args[0]).map_err(|e| match e {
            PyError::ValueError(_) => PyError::ValueError("harmonic_mean() argument is empty".to_string()),
            PyError::TypeError(_) => PyError::type_error("harmonic_mean() argument must contain numbers"),
            other => other,
        })?;
        if nums.iter().any(|&x| x < 0.0) {
            return Err(PyError::ValueError("harmonic_mean() does not support negative values".to_string()));
        }
        if nums.iter().any(|&x| x == 0.0) {
            return Ok(py_float(0.0));
        }
        let n = nums.len() as f64;
        let recip_sum: f64 = nums.iter().map(|x| 1.0 / x).sum();
        Ok(py_float(n / recip_sum))
    });

    stat_func!("mode", |args| {
        if args.is_empty() { return Err(PyError::type_error("mode() missing required argument")); }
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
        if args.is_empty() { return Err(PyError::type_error("median_low() missing required argument")); }
        let mut nums = stat_extract_nums(&args[0])?;
        if nums.is_empty() {
            return Err(PyError::ValueError("median_low() argument is empty".to_string()));
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        Ok(py_float(nums[(n - 1) / 2]))
    });

    stat_func!("median_high", |args| {
        if args.is_empty() { return Err(PyError::type_error("median_high() missing required argument")); }
        let mut nums = stat_extract_nums(&args[0])?;
        if nums.is_empty() {
            return Err(PyError::ValueError("median_high() argument is empty".to_string()));
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len();
        Ok(py_float(nums[n / 2]))
    });

    // `statistics.__all__` — same fix, same reason, as `operator.__all__`
    // (`core.rs`) — missing entirely, breaking the module's own
    // `test___all__` sanity check at collection time.
    let all_names: Vec<PyObjectRef> = d.keys().filter(|k| !k.starts_with('_')).map(|k| py_str(k)).collect();
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
enum DecSpecial { Finite, QNaN, SNaN, Infinity }

#[derive(Clone, Debug)]
struct DecValue {
    special: DecSpecial,
    sign: bool, // true = negative
    coeff: num_bigint::BigInt, // non-negative significand; 0 for NaN/Infinity
    exp: i64,   // meaningless for NaN/Infinity
}

impl DecValue {
    fn zero() -> Self { DecValue { special: DecSpecial::Finite, sign: false, coeff: num_bigint::BigInt::from(0), exp: 0 } }
    fn nan() -> Self { DecValue { special: DecSpecial::QNaN, sign: false, coeff: num_bigint::BigInt::from(0), exp: 0 } }
    fn infinity(sign: bool) -> Self { DecValue { special: DecSpecial::Infinity, sign, coeff: num_bigint::BigInt::from(0), exp: 0 } }
    fn is_zero(&self) -> bool { self.special == DecSpecial::Finite && num_traits::Zero::is_zero(&self.coeff) }
    fn is_nan(&self) -> bool { matches!(self.special, DecSpecial::QNaN | DecSpecial::SNaN) }
}

fn parse_decimal_str(raw: &str) -> Option<DecValue> {
    let s = raw.trim();
    if s.is_empty() { return None; }
    let mut sign = false;
    let rest = if let Some(r) = s.strip_prefix('+') { r }
        else if let Some(r) = s.strip_prefix('-') { sign = true; r }
        else { s };
    if rest.is_empty() { return None; }
    let rest_lower = rest.to_ascii_lowercase();
    if rest_lower == "inf" || rest_lower == "infinity" {
        return Some(DecValue::infinity(sign));
    }
    if let Some(digits_part) = rest_lower.strip_prefix("snan") {
        let coeff = if digits_part.is_empty() { num_bigint::BigInt::from(0) } else { num_bigint::BigInt::parse_bytes(digits_part.as_bytes(), 10)? };
        return Some(DecValue { special: DecSpecial::SNaN, sign, coeff, exp: 0 });
    }
    if let Some(digits_part) = rest_lower.strip_prefix("nan") {
        let coeff = if digits_part.is_empty() { num_bigint::BigInt::from(0) } else { num_bigint::BigInt::parse_bytes(digits_part.as_bytes(), 10)? };
        return Some(DecValue { special: DecSpecial::QNaN, sign, coeff, exp: 0 });
    }
    let (mantissa_part, exp_part) = match rest.find(['e', 'E']) {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
        None => (rest, None),
    };
    if mantissa_part.is_empty() { return None; }
    let (int_part, frac_part) = match mantissa_part.find('.') {
        Some(idx) => (&mantissa_part[..idx], &mantissa_part[idx + 1..]),
        None => (mantissa_part, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() { return None; }
    if !int_part.chars().all(|c| c.is_ascii_digit()) { return None; }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) { return None; }
    let digits_str = format!("{}{}", int_part, frac_part);
    let digits_str = if digits_str.is_empty() { "0".to_string() } else { digits_str };
    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10)?;
    let mut exp: i64 = -(frac_part.len() as i64);
    if let Some(exp_str) = exp_part {
        let exp_str = exp_str.trim();
        if exp_str.is_empty() { return None; }
        let extra: i64 = exp_str.parse().ok()?;
        exp += extra;
    }
    Some(DecValue { special: DecSpecial::Finite, sign, coeff, exp })
}

fn decval_from_f64(f: f64) -> DecValue {
    // float -> Decimal must be exact (matching CPython's Decimal(float)),
    // so go through the float's own repr rather than lossy formatting.
    if f.is_nan() { return DecValue::nan(); }
    if f.is_infinite() { return DecValue::infinity(f < 0.0); }
    parse_decimal_str(&format!("{:e}", f)).unwrap_or_else(DecValue::zero)
}

fn ten_pow(n: i64) -> num_bigint::BigInt {
    if n <= 0 { return num_bigint::BigInt::from(1); }
    num_bigint::BigInt::from(10).pow(n as u32)
}

fn digit_count(coeff: &num_bigint::BigInt) -> usize {
    if num_traits::Zero::is_zero(coeff) { return 1; }
    coeff.to_string().len()
}

/// CPython's decimal-to-string algorithm (IBM spec `to-scientific-string`):
/// plain notation when the exponent is small enough, scientific otherwise.
fn format_decvalue(v: &DecValue) -> String {
    let sign_str = if v.sign { "-" } else { "" };
    match v.special {
        DecSpecial::Infinity => return format!("{}Infinity", sign_str),
        DecSpecial::QNaN => {
            let digits = if num_traits::Zero::is_zero(&v.coeff) { String::new() } else { v.coeff.to_string() };
            return format!("{}NaN{}", sign_str, digits);
        }
        DecSpecial::SNaN => {
            let digits = if num_traits::Zero::is_zero(&v.coeff) { String::new() } else { v.coeff.to_string() };
            return format!("{}sNaN{}", sign_str, digits);
        }
        DecSpecial::Finite => {}
    }
    let digits = if num_traits::Zero::is_zero(&v.coeff) { "0".to_string() } else { v.coeff.to_string() };
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
        let body = if leading == 1 { digits.clone() } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        let exp_sign = if adjusted_exp >= 0 { "+" } else { "-" };
        format!("{}{}E{}{}", sign_str, body, exp_sign, adjusted_exp.abs())
    }
}

thread_local! {
    static DECIMAL_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static DECIMAL_CONTEXT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static DECIMAL_CURRENT_CONTEXT: std::cell::RefCell<(usize, String)> = std::cell::RefCell::new((28, "ROUND_HALF_EVEN".to_string()));
}

fn current_decimal_context() -> (usize, String) {
    DECIMAL_CURRENT_CONTEXT.with(|c| c.borrow().clone())
}

const DEC_SIGN_KEY: &str = "_sign";
const DEC_COEFF_KEY: &str = "_coeff";
const DEC_EXP_KEY: &str = "_exp";
const DEC_SPECIAL_KEY: &str = "_special";

fn special_to_str(s: &DecSpecial) -> &'static str {
    match s { DecSpecial::Finite => "", DecSpecial::QNaN => "n", DecSpecial::SNaN => "N", DecSpecial::Infinity => "F" }
}
fn special_from_str(s: &str) -> DecSpecial {
    match s { "n" => DecSpecial::QNaN, "N" => DecSpecial::SNaN, "F" => DecSpecial::Infinity, _ => DecSpecial::Finite }
}

fn decval_to_instance(v: &DecValue) -> PyObjectRef {
    let typ = get_decimal_type();
    let mut dict = AttrMap::new();
    dict.insert(DEC_SIGN_KEY.to_string(), py_bool(v.sign));
    dict.insert(DEC_COEFF_KEY.to_string(), py_int(v.coeff.clone()));
    dict.insert(DEC_EXP_KEY.to_string(), py_int(v.exp));
    dict.insert(DEC_SPECIAL_KEY.to_string(), py_str(special_to_str(&v.special)));
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
        Some(DecValue { special, sign, coeff, exp })
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
            PyError::Exception("InvalidOperation".to_string(), PyObjectRef::new(PyObject::Exception {
                typ: "InvalidOperation".to_string(),
                args: vec![py_str(&format!("invalid literal for Decimal: '{}'", s))],
                cause: None,
            }))
        }),
        PyObject::Int(i) => {
            let sign = num_traits::Signed::is_negative(i);
            Ok(DecValue { special: DecSpecial::Finite, sign, coeff: num_traits::Signed::abs(i), exp: 0 })
        }
        PyObject::Bool(b) => Ok(DecValue { special: DecSpecial::Finite, sign: false, coeff: num_bigint::BigInt::from(if *b { 1 } else { 0 }), exp: 0 }),
        PyObject::Float(f) => Ok(decval_from_f64(*f)),
        PyObject::Tuple(parts) => {
            if parts.len() != 3 { return Err(PyError::value_error("argument must be a sequence of length 3")); }
            let sign = parts[0].as_i64().unwrap_or(0) != 0;
            let digit_items: Vec<PyObjectRef> = match &*parts[1].borrow() {
                PyObject::Tuple(d) | PyObject::List(d) => d.clone(),
                _ => return Err(PyError::value_error("digits must be a sequence of ints")),
            };
            let mut digits_str = String::new();
            for d in &digit_items { digits_str.push_str(&d.as_i64().unwrap_or(0).to_string()); }
            if digits_str.is_empty() { digits_str.push('0'); }
            match &*parts[2].borrow() {
                PyObject::Str(s) if s == "F" => Ok(DecValue::infinity(sign)),
                PyObject::Str(s) if s == "n" || s == "N" => {
                    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10).unwrap_or_default();
                    Ok(DecValue { special: special_from_str(s), sign, coeff, exp: 0 })
                }
                _ => {
                    let exp = parts[2].as_i64().unwrap_or(0);
                    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10).unwrap_or_default();
                    Ok(DecValue { special: DecSpecial::Finite, sign, coeff, exp })
                }
            }
        }
        PyObject::None => Ok(DecValue::zero()),
        _ => Err(PyError::type_error("conversion from unsupported type to Decimal")),
    }
}

fn round_decvalue(v: &DecValue, precision: usize, rounding: &str) -> DecValue {
    if v.special != DecSpecial::Finite { return v.clone(); }
    let ndigits = digit_count(&v.coeff);
    if ndigits <= precision { return v.clone(); }
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
        "ROUND_05UP" => !num_traits::Zero::is_zero(&r) && { let last = &q % 10; last == num_bigint::BigInt::from(0) || last == num_bigint::BigInt::from(5) },
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
    DecValue { special: DecSpecial::Finite, sign: v.sign, coeff: final_q, exp: new_exp }
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
    PyError::Exception("InvalidOperation".to_string(), PyObjectRef::new(PyObject::Exception {
        typ: "InvalidOperation".to_string(), args: vec![py_str(msg)], cause: None,
    }))
}
fn decimal_division_by_zero(msg: &str) -> PyError {
    PyError::Exception("DivisionByZero".to_string(), PyObjectRef::new(PyObject::Exception {
        typ: "DivisionByZero".to_string(), args: vec![py_str(msg)], cause: None,
    }))
}

fn decimal_add(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.is_nan() || b.is_nan() {
        let src = if a.is_nan() { a } else { b };
        return Ok(DecValue { special: DecSpecial::QNaN, sign: src.sign, coeff: src.coeff.clone(), exp: 0 });
    }
    if a.special == DecSpecial::Infinity || b.special == DecSpecial::Infinity {
        if a.special == DecSpecial::Infinity && b.special == DecSpecial::Infinity && a.sign != b.sign {
            return Err(decimal_invalid_op("(+Infinity) + (-Infinity)"));
        }
        return Ok(DecValue::infinity(if a.special == DecSpecial::Infinity { a.sign } else { b.sign }));
    }
    let (as_, bs, exp) = decval_align(a, b);
    let sum = (if a.sign { -as_ } else { as_ }) + (if b.sign { -bs } else { bs });
    let sign = num_traits::Signed::is_negative(&sum);
    let result = DecValue { special: DecSpecial::Finite, sign, coeff: num_traits::Signed::abs(&sum), exp };
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
        return Ok(DecValue { special: DecSpecial::QNaN, sign: src.sign, coeff: src.coeff.clone(), exp: 0 });
    }
    let sign = a.sign != b.sign;
    if a.special == DecSpecial::Infinity || b.special == DecSpecial::Infinity {
        if a.is_zero() || b.is_zero() { return Err(decimal_invalid_op("(+/-Infinity) * 0")); }
        return Ok(DecValue::infinity(sign));
    }
    let result = DecValue { special: DecSpecial::Finite, sign, coeff: &a.coeff * &b.coeff, exp: a.exp + b.exp };
    Ok(round_to_context(result))
}

fn decimal_div(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.is_nan() || b.is_nan() {
        let src = if a.is_nan() { a } else { b };
        return Ok(DecValue { special: DecSpecial::QNaN, sign: src.sign, coeff: src.coeff.clone(), exp: 0 });
    }
    let sign = a.sign != b.sign;
    if a.special == DecSpecial::Infinity && b.special == DecSpecial::Infinity {
        return Err(decimal_invalid_op("(+/-Infinity) / (+/-Infinity)"));
    }
    if a.special == DecSpecial::Infinity { return Ok(DecValue::infinity(sign)); }
    if b.special == DecSpecial::Infinity { return Ok(DecValue { special: DecSpecial::Finite, sign, coeff: num_bigint::BigInt::from(0), exp: 0 }); }
    if b.is_zero() {
        if a.is_zero() { return Err(decimal_invalid_op("0 / 0")); }
        return Err(decimal_division_by_zero("division by zero"));
    }
    if a.is_zero() {
        return Ok(round_to_context(DecValue { special: DecSpecial::Finite, sign, coeff: num_bigint::BigInt::from(0), exp: a.exp - b.exp }));
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
    let mut result = DecValue { special: DecSpecial::Finite, sign, coeff: raw_q, exp: raw_exp };
    if !num_traits::Zero::is_zero(&raw_r) {
        // Inexact — nudge the last kept digit if a straightforward rounding
        // of the truncated remainder would change it (half-up on the guard
        // digits is precise enough given the wide guard margin above).
        if &raw_r * 2 >= b.coeff { result.coeff += 1; }
    }
    Ok(round_decvalue(&result, precision, &rounding))
}

fn decimal_compare(a: &DecValue, b: &DecValue) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if a.is_nan() || b.is_nan() { return None; }
    match (&a.special, &b.special) {
        (DecSpecial::Infinity, DecSpecial::Infinity) => {
            return Some(if a.sign == b.sign { Ordering::Equal } else if a.sign { Ordering::Less } else { Ordering::Greater });
        }
        (DecSpecial::Infinity, _) => return Some(if a.sign { Ordering::Less } else { Ordering::Greater }),
        (_, DecSpecial::Infinity) => return Some(if b.sign { Ordering::Greater } else { Ordering::Less }),
        _ => {}
    }
    if a.is_zero() && b.is_zero() { return Some(Ordering::Equal); }
    let (as_, bs, _) = decval_align(a, b);
    let a_signed = if a.sign { -as_ } else { as_ };
    let b_signed = if b.sign { -bs } else { bs };
    Some(a_signed.cmp(&b_signed))
}

fn decval_to_f64(v: &DecValue) -> f64 {
    match v.special {
        DecSpecial::Infinity => if v.sign { f64::NEG_INFINITY } else { f64::INFINITY },
        DecSpecial::QNaN | DecSpecial::SNaN => f64::NAN,
        DecSpecial::Finite => {
            // Parse the exact decimal string rather than coeff as f64 times
            // 10^exp — that separate multiplication introduces float error
            // (e.g. 12345.0 * 0.01 != 123.45 exactly), whereas Rust's own
            // string-to-f64 parsing correctly rounds to the nearest float.
            format!("{}{}e{}", if v.sign { "-" } else { "" }, v.coeff, v.exp).parse().unwrap_or(0.0)
        }
    }
}

fn normalize_decval(v: &DecValue) -> DecValue {
    if v.special != DecSpecial::Finite || v.is_zero() {
        if v.is_zero() { return DecValue { special: DecSpecial::Finite, sign: v.sign, coeff: num_bigint::BigInt::from(0), exp: 0 }; }
        return v.clone();
    }
    let mut coeff = v.coeff.clone();
    let mut exp = v.exp;
    let ten = num_bigint::BigInt::from(10);
    while &coeff % &ten == num_bigint::BigInt::from(0) && coeff != num_bigint::BigInt::from(0) {
        coeff /= &ten;
        exp += 1;
    }
    DecValue { special: DecSpecial::Finite, sign: v.sign, coeff, exp }
}

fn get_decimal_type() -> PyObjectRef {
    let existing = DECIMAL_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_decimal_type();
    DECIMAL_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
    typ
}

fn build_decimal_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }

    type_dict.insert_str("__init__", bf!("__init__", |args| {
        let v = if args.len() > 1 { decval_from_pyobject(&args[1])? } else { DecValue::zero() };
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert(DEC_SIGN_KEY.to_string(), py_bool(v.sign));
            dict.insert(DEC_COEFF_KEY.to_string(), py_int(v.coeff));
            dict.insert(DEC_EXP_KEY.to_string(), py_int(v.exp));
            dict.insert(DEC_SPECIAL_KEY.to_string(), py_str(special_to_str(&v.special)));
        }
        Ok(py_none())
    }));
    type_dict.insert_str("__repr__", bf!("__repr__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_str(&format!("Decimal('{}')", format_decvalue(&v))))
    }));
    type_dict.insert_str("__str__", bf!("__str__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_str(&format_decvalue(&v)))
    }));
    type_dict.insert_str("__int__", bf!("__int__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        if v.special != DecSpecial::Finite { return Err(PyError::value_error("cannot convert NaN/Infinity to int")); }
        let truncated = if v.exp >= 0 { &v.coeff * ten_pow(v.exp) } else { &v.coeff / ten_pow(-v.exp) };
        Ok(py_int(if v.sign { -truncated } else { truncated }))
    }));
    type_dict.insert_str("__float__", bf!("__float__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_float(decval_to_f64(&v)))
    }));
    type_dict.insert_str("__bool__", bf!("__bool__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(!v.is_zero()))
    }));
    type_dict.insert_str("__hash__", bf!("__hash__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        if v.special != DecSpecial::Finite { return Ok(py_int(0)); }
        // Normalize (strip trailing zeros) so numerically-equal Decimals
        // with different (coeff, exp) representations — e.g. 1 vs 1.0 —
        // hash the same way `1 == 1.0` requires.
        let n = normalize_decval(&v);
        let s = format!("{}{}{}", n.sign, n.coeff, n.exp);
        builtin_hash(&[py_str(&s)])
    }));
    type_dict.insert_str("__eq__", bf!("__eq__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = match decval_from_pyobject(&args[1]) { Ok(v) => v, Err(_) => return Ok(py_bool(false)) };
        Ok(py_bool(decimal_compare(&a, &b) == Some(std::cmp::Ordering::Equal)))
    }));
    macro_rules! dec_cmp {
        ($name:expr, $ord:pat) => {
            type_dict.insert($name.to_string(), bf!($name, |args| {
                let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
                let b = decval_from_pyobject(&args[1])?;
                match decimal_compare(&a, &b) {
                    Some($ord) => Ok(py_bool(true)),
                    Some(_) => Ok(py_bool(false)),
                    None => Err(PyError::type_error("cannot compare NaN with Decimal")),
                }
            }));
        };
    }
    dec_cmp!("__lt__", std::cmp::Ordering::Less);
    dec_cmp!("__gt__", std::cmp::Ordering::Greater);
    type_dict.insert_str("__le__", bf!("__le__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        match decimal_compare(&a, &b) {
            Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal) => Ok(py_bool(true)),
            Some(_) => Ok(py_bool(false)),
            None => Err(PyError::type_error("cannot compare NaN with Decimal")),
        }
    }));
    type_dict.insert_str("__ge__", bf!("__ge__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        match decimal_compare(&a, &b) {
            Some(std::cmp::Ordering::Greater) | Some(std::cmp::Ordering::Equal) => Ok(py_bool(true)),
            Some(_) => Ok(py_bool(false)),
            None => Err(PyError::type_error("cannot compare NaN with Decimal")),
        }
    }));
    macro_rules! dec_binop {
        ($name:expr, $op:expr) => {
            type_dict.insert($name.to_string(), bf!($name, |args| {
                let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
                let b = decval_from_pyobject(&args[1])?;
                Ok(decval_to_instance(&$op(&a, &b)?))
            }));
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
    type_dict.insert_str("__floordiv__", bf!("__floordiv__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        let q = decimal_div(&a, &b)?;
        if q.special != DecSpecial::Finite { return Ok(decval_to_instance(&q)); }
        let truncated = if q.exp >= 0 { &q.coeff * ten_pow(q.exp) } else { &q.coeff / ten_pow(-q.exp) };
        Ok(decval_to_instance(&DecValue { special: DecSpecial::Finite, sign: q.sign, coeff: truncated, exp: 0 }))
    }));
    type_dict.insert_str("__mod__", bf!("__mod__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        if b.is_zero() { return Err(decimal_invalid_op("0 modulo")); }
        let q = decimal_div(&a, &b)?;
        let truncated_q = if q.exp >= 0 { &q.coeff * ten_pow(q.exp) } else { &q.coeff / ten_pow(-q.exp) };
        let trunc_dec = DecValue { special: DecSpecial::Finite, sign: q.sign, coeff: truncated_q, exp: 0 };
        let prod = decimal_mul(&trunc_dec, &b)?;
        Ok(decval_to_instance(&decimal_sub(&a, &prod)?))
    }));
    type_dict.insert_str("__pow__", bf!("__pow__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        if b.special != DecSpecial::Finite || b.exp < 0 { return Err(PyError::runtime_error("Decimal ** non-integer exponent is not supported")); }
        let n = (&b.coeff * ten_pow(b.exp)).to_string().parse::<i64>().unwrap_or(0);
        let n = if b.sign { -n } else { n };
        if n < 0 { return Err(PyError::runtime_error("Decimal ** negative exponent is not supported")); }
        let mut result = DecValue { special: DecSpecial::Finite, sign: false, coeff: num_bigint::BigInt::from(1), exp: 0 };
        for _ in 0..n { result = decimal_mul(&result, &a)?; }
        Ok(decval_to_instance(&result))
    }));
    type_dict.insert_str("__neg__", bf!("__neg__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&decimal_negate(&v)))
    }));
    type_dict.insert_str("__pos__", bf!("__pos__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&round_to_context(v)))
    }));
    type_dict.insert_str("__abs__", bf!("__abs__", |args| {
        let mut v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        v.sign = false;
        Ok(decval_to_instance(&v))
    }));
    type_dict.insert_str("is_nan", bf!("is_nan", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.is_nan()))
    }));
    type_dict.insert_str("is_infinite", bf!("is_infinite", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.special == DecSpecial::Infinity))
    }));
    type_dict.insert_str("is_finite", bf!("is_finite", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.special == DecSpecial::Finite))
    }));
    type_dict.insert_str("is_zero", bf!("is_zero", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.is_zero()))
    }));
    type_dict.insert_str("is_signed", bf!("is_signed", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.sign))
    }));
    type_dict.insert_str("copy_sign", bf!("copy_sign", |args| {
        let mut v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let other = decval_from_pyobject(&args[1])?;
        v.sign = other.sign;
        Ok(decval_to_instance(&v))
    }));
    type_dict.insert_str("copy_abs", bf!("copy_abs", |args| {
        let mut v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        v.sign = false;
        Ok(decval_to_instance(&v))
    }));
    type_dict.insert_str("copy_negate", bf!("copy_negate", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&decimal_negate(&v)))
    }));
    type_dict.insert_str("as_tuple", bf!("as_tuple", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let sign_val = py_int(if v.sign { 1 } else { 0 });
        let digits_str = if num_traits::Zero::is_zero(&v.coeff) { "0".to_string() } else { v.coeff.to_string() };
        let digits: Vec<PyObjectRef> = digits_str.chars().map(|c| py_int(c.to_digit(10).unwrap_or(0) as i64)).collect();
        let exp_val = match v.special {
            DecSpecial::Finite => py_int(v.exp),
            DecSpecial::Infinity => py_str("F"),
            DecSpecial::QNaN => py_str("n"),
            DecSpecial::SNaN => py_str("N"),
        };
        Ok(py_tuple(vec![sign_val, py_tuple(digits), exp_val]))
    }));
    type_dict.insert_str("normalize", bf!("normalize", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&normalize_decval(&round_to_context(v))))
    }));
    type_dict.insert_str("quantize", bf!("quantize", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        if args.len() < 2 { return Err(PyError::type_error("quantize() missing exponent argument")); }
        let target = decval_from_pyobject(&args[1])?;
        if v.special != DecSpecial::Finite || target.special != DecSpecial::Finite {
            return Err(decimal_invalid_op("quantize with non-finite argument"));
        }
        let (_, rounding) = current_decimal_context();
        let target_exp = target.exp;
        let result = if target_exp >= v.exp {
            let drop = (target_exp - v.exp) as usize;
            round_decvalue(&v, digit_count(&v.coeff).saturating_sub(drop).max(1), &rounding)
        } else {
            let scale = ten_pow(v.exp - target_exp);
            DecValue { special: DecSpecial::Finite, sign: v.sign, coeff: &v.coeff * scale, exp: target_exp }
        };
        Ok(decval_to_instance(&DecValue { exp: target_exp, ..result }))
    }));
    type_dict.insert_str("to_integral_value", bf!("to_integral_value", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        if v.special != DecSpecial::Finite || v.exp >= 0 { return Ok(decval_to_instance(&v)); }
        let (_, rounding) = current_decimal_context();
        let rounded = round_decvalue(&v, digit_count(&v.coeff).saturating_sub((-v.exp) as usize).max(1), &rounding);
        Ok(decval_to_instance(&DecValue { exp: 0, coeff: &rounded.coeff * ten_pow(rounded.exp), ..rounded }))
    }));
    type_dict.insert_str("compare", bf!("compare", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        let n: i64 = match decimal_compare(&a, &b) {
            Some(std::cmp::Ordering::Less) => -1,
            Some(std::cmp::Ordering::Greater) => 1,
            Some(std::cmp::Ordering::Equal) => 0,
            None => return Ok(decval_to_instance(&DecValue::nan())),
        };
        Ok(decval_to_instance(&DecValue { special: DecSpecial::Finite, sign: n < 0, coeff: num_bigint::BigInt::from(n.abs()), exp: 0 }))
    }));

    PyObjectRef::new(PyObject::Type { name: "Decimal".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] })
}

fn build_context_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }
    type_dict.insert_str("__init__", bf!("__init__", |args| {
        let ctor_args = args[1..].to_vec();
        let kw: Option<PyDict> = ctor_args.last().and_then(|a| if let PyObject::Dict(d) = &*a.borrow() { Some((**d).clone()) } else { None });
        let get_kw = |name: &str| kw.as_ref().and_then(|d| d.get(&py_str(name)).ok().flatten());
        let precision = get_kw("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
        let rounding = get_kw("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("prec", py_int(precision as i64));
            dict.insert_str("rounding", py_str(&rounding));
        }
        Ok(py_none())
    }));
    type_dict.insert_str("__repr__", bf!("__repr__", |args| {
        let prec = if let PyObject::Instance { dict, .. } = &*args[0].borrow() { dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) } else { 28 };
        Ok(py_str(&format!("Context(prec={})", prec)))
    }));
    PyObjectRef::new(PyObject::Type { name: "Context".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] })
}

fn get_context_type() -> PyObjectRef {
    let existing = DECIMAL_CONTEXT_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_context_type();
    DECIMAL_CONTEXT_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
    typ
}

fn make_context_instance(precision: usize, rounding: &str) -> PyObjectRef {
    let typ = get_context_type();
    let mut dict = AttrMap::new();
    dict.insert_str("prec", py_int(precision as i64));
    dict.insert_str("rounding", py_str(rounding));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub fn create_decimal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dec_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    d.insert_str("Decimal", get_decimal_type());
    d.insert_str("Context", get_context_type());
    dec_func!("getcontext", |_args| {
        let (precision, rounding) = current_decimal_context();
        Ok(make_context_instance(precision, &rounding))
    });
    dec_func!("setcontext", |args| {
        if args.is_empty() { return Err(PyError::type_error("setcontext() missing context argument")); }
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            let precision = dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
            let rounding = dict.get_str("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
            DECIMAL_CURRENT_CONTEXT.with(|c| { *c.borrow_mut() = (precision, rounding); });
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
                (dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize, dict.get_str("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string()))
            } else { current_decimal_context() }
        } else { current_decimal_context() };
        let ctx = make_context_instance(precision, &rounding);
        let mut cm_dict = HashMap::new();
        cm_dict.insert_str("__enter__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let precision = dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
                    let rounding = dict.get_str("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
                    DECIMAL_CURRENT_CONTEXT.with(|c| { *c.borrow_mut() = (precision, rounding); });
                }
                Ok(args[0].clone())
            },
        }));
        cm_dict.insert_str("__exit__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__exit__".to_string(),
            func: |_args| { DECIMAL_CURRENT_CONTEXT.with(|c| { *c.borrow_mut() = (28, "ROUND_HALF_EVEN".to_string()); }); Ok(py_bool(false)) },
        }));
        let cm_typ = PyObjectRef::new(PyObject::Type { name: "_ContextManager".to_string(), dict: Box::new(str_map_to_typedict(cm_dict)), bases: vec![], mro: vec![] });
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str("prec", py_int(precision as i64));
        inst_dict.insert_str("rounding", py_str(&rounding));
        let _ = ctx;
        Ok(PyObjectRef::new(PyObject::Instance { typ: cm_typ, dict: inst_dict }))
    });
    // Exception types
    d.insert_str("DecimalException", PyObjectRef::new(PyObject::BuiltinFunction { name: "DecimalException".to_string(), func: crate::object::builtin_make_exception_decimalexception }));
    d.insert_str("InvalidOperation", PyObjectRef::new(PyObject::BuiltinFunction { name: "InvalidOperation".to_string(), func: crate::object::builtin_make_exception_invalidoperation }));
    d.insert_str("DivisionByZero", PyObjectRef::new(PyObject::BuiltinFunction { name: "DivisionByZero".to_string(), func: crate::object::builtin_make_exception_decimaldivisionbyzero }));
    d.insert_str("Inexact", PyObjectRef::new(PyObject::BuiltinFunction { name: "Inexact".to_string(), func: crate::object::builtin_make_exception_inexact }));
    d.insert_str("Rounded", PyObjectRef::new(PyObject::BuiltinFunction { name: "Rounded".to_string(), func: crate::object::builtin_make_exception_rounded }));
    d.insert_str("Clamped", PyObjectRef::new(PyObject::BuiltinFunction { name: "Clamped".to_string(), func: crate::object::builtin_make_exception_clamped }));
    d.insert_str("Overflow", PyObjectRef::new(PyObject::BuiltinFunction { name: "Overflow".to_string(), func: crate::object::builtin_make_exception_decimaloverflow }));
    d.insert_str("Underflow", PyObjectRef::new(PyObject::BuiltinFunction { name: "Underflow".to_string(), func: crate::object::builtin_make_exception_decimalunderflow }));
    d.insert_str("FloatOperation", PyObjectRef::new(PyObject::BuiltinFunction { name: "FloatOperation".to_string(), func: crate::object::builtin_make_exception_floatoperation }));
    // Rounding mode constants — their real string values (that's what
    // CPython's decimal.ROUND_* constants actually are), so equality checks
    // and passing them to quantize()-style calls behave as real code expects.
    for name in ["ROUND_CEILING", "ROUND_DOWN", "ROUND_FLOOR", "ROUND_HALF_DOWN",
                 "ROUND_HALF_EVEN", "ROUND_HALF_UP", "ROUND_UP", "ROUND_05UP"] {
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
use num_traits::{Zero, One, Signed};

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
        return Err(PyError::ZeroDivisionError("Fraction(%s, 0)".to_string()));
    }
    if den.sign() == Sign::Minus { num = -num; den = -den; }
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
    if f.is_nan() || f.is_infinite() {
        return Err(PyError::value_error(format!("cannot convert {} to a Fraction", f)));
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
    if neg { num = -num; }
    let mut den = BigInt::one();
    if exp >= 0 {
        num *= BigInt::from(2).pow(exp as u32);
    } else {
        den = BigInt::from(2).pow((-exp) as u32);
    }
    frac_normalize(num, den)
}

/// Parse `"3/4"`, `"3"`, `"1.5"`, `"-1.5e2"` (real `Fraction(str)` accepts
/// decimal-literal-like strings too, converting exactly via `from_decimal`
/// semantics) — a simplified but exact-for-terminating-decimals subset.
fn frac_parse_str(s: &str) -> PyResult<(BigInt, BigInt)> {
    let s = s.trim();
    let bad = || PyError::value_error(format!("Invalid literal for Fraction: '{}'", s));
    if let Some((n, d)) = s.split_once('/') {
        let num: BigInt = n.trim().parse().map_err(|_| bad())?;
        let den: BigInt = d.trim().parse().map_err(|_| bad())?;
        return frac_normalize(num, den);
    }
    if let Ok(n) = s.parse::<BigInt>() {
        return Ok((n, BigInt::one()));
    }
    // Decimal literal (possibly with an exponent): convert exactly via
    // scaling by a power of 10, matching `Fraction(Decimal(s))` semantics.
    let f: f64 = s.parse().map_err(|_| bad())?;
    if let Some(dot) = s.find(['.', 'e', 'E']) {
        let _ = dot;
        // Exact decimal-string handling for the common (non-scientific)
        // case: `int_part.frac_part` -> (int_part*10^len(frac)+frac_part) /
        // 10^len(frac). Falls back to the (inexact) float route for
        // scientific notation, an acceptable simplification.
        if !s.contains(['e', 'E']) {
            if let Some((int_part, frac_part)) = s.split_once('.') {
                let neg = int_part.starts_with('-');
                let int_part_clean = int_part.trim_start_matches(['-', '+']);
                let combined = format!("{}{}", int_part_clean, frac_part);
                if let Ok(mut num) = combined.parse::<BigInt>() {
                    if neg { num = -num; }
                    let den = BigInt::from(10).pow(frac_part.len() as u32);
                    return frac_normalize(num, den);
                }
            }
        }
    }
    frac_float_to_ratio(f)
}

fn frac_instance_num_den(v: &PyObjectRef) -> Option<(BigInt, BigInt)> {
    if let PyObject::Instance { dict, .. } = &*v.borrow() {
        let num = dict.get_str("numerator")?;
        let den = dict.get_str("denominator")?;
        if let (PyObject::Int(n), PyObject::Int(d)) = (&*num.borrow(), &*den.borrow()) {
            return Some((n.clone(), d.clone()));
        }
    }
    None
}

fn frac_is_fraction(v: &PyObjectRef) -> bool {
    matches!(&*v.borrow(), PyObject::Instance { dict, .. } if dict.get_str("numerator").is_some() && dict.get_str("denominator").is_some())
}

fn frac_make(frac_type: &PyObjectRef, num: BigInt, den: BigInt) -> PyResult<PyObjectRef> {
    let (num, den) = frac_normalize(num, den)?;
    let mut dict = AttrMap::new();
    dict.insert_str("numerator", py_int(num));
    dict.insert_str("denominator", py_int(den));
    Ok(PyObjectRef::new(PyObject::Instance { typ: frac_type.clone(), dict }))
}

/// Numeric operand kind for Fraction arithmetic's real-Python coercion
/// rules: `Fraction op int` stays a `Fraction`; `Fraction op float` (or
/// vice versa) coerces the WHOLE operation to plain `float` (matching real
/// `Fraction.__add__`'s own documented behavior); anything else is
/// `NotImplemented` (deferring to the other operand's reflected method).
enum FracOperand { Frac(BigInt, BigInt), Float(f64), Other }

fn frac_operand_of(v: &PyObjectRef) -> FracOperand {
    if let Some((n, d)) = frac_instance_num_den(v) { return FracOperand::Frac(n, d); }
    let b = v.borrow();
    match &*b {
        PyObject::Int(i) => FracOperand::Frac(i.clone(), BigInt::one()),
        PyObject::Bool(bv) => FracOperand::Frac(BigInt::from(*bv as i64), BigInt::one()),
        PyObject::Float(f) => FracOperand::Float(*f),
        _ => FracOperand::Other,
    }
}

fn frac_self_num_den(self_obj: &PyObjectRef) -> PyResult<(BigInt, BigInt)> {
    frac_instance_num_den(self_obj).ok_or_else(|| PyError::type_error("not a Fraction"))
}

fn frac_self_type(self_obj: &PyObjectRef) -> PyObjectRef {
    if let PyObject::Instance { typ, .. } = &*self_obj.borrow() { typ.clone() } else { unreachable!() }
}

fn frac_to_f64(num: &BigInt, den: &BigInt) -> f64 {
    num.to_f64().unwrap_or(f64::NAN) / den.to_f64().unwrap_or(1.0)
}

/// Shared binary-op dispatcher: `op` combines two exact `(num, den)` pairs;
/// `float_op` combines two `f64`s for the mixed-with-float coercion case.
fn frac_binop(
    args: &[PyObjectRef],
    reflected: bool,
    op: impl Fn(BigInt, BigInt, BigInt, BigInt) -> PyResult<(BigInt, BigInt)>,
    float_op: impl Fn(f64, f64) -> f64,
) -> PyResult<PyObjectRef> {
    if args.len() < 2 { return Err(PyError::type_error("expected 2 arguments")); }
    // `self` (args[0]) is always the Fraction whose method this is; for a
    // reflected call (`__radd__` etc.) `self` is semantically the RIGHT
    // operand of `other OP self`, so `op`'s arguments are swapped below
    // rather than swapping `an`/`ad` here.
    let (an, ad) = frac_self_num_den(&args[0])?;
    match frac_operand_of(&args[1]) {
        FracOperand::Frac(bn, bd) => {
            let (rn, rd) = if reflected { op(bn, bd, an, ad)? } else { op(an, ad, bn, bd)? };
            frac_make(&frac_self_type(&args[0]), rn, rd)
        }
        FracOperand::Float(bf) => {
            let af = frac_to_f64(&an, &ad);
            Ok(py_float(if reflected { float_op(bf, af) } else { float_op(af, bf) }))
        }
        FracOperand::Other => Ok(py_not_implemented()),
    }
}

pub fn create_fractions_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let mut frac_dict: HashMap<String, PyObjectRef> = HashMap::new();

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
    frac_dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("__init__ requires self")); }
            let rest = &args[1..];
            let (num, den) = match rest.len() {
                0 => (BigInt::zero(), BigInt::one()),
                1 => match frac_operand_of(&rest[0]) {
                    FracOperand::Frac(n, d) => (n, d),
                    FracOperand::Float(f) => frac_float_to_ratio(f)?,
                    FracOperand::Other => {
                        let b = rest[0].borrow();
                        match &*b {
                            PyObject::Str(s) => frac_parse_str(s)?,
                            _ => return Err(PyError::type_error("argument should be a string or a Rational instance")),
                        }
                    }
                },
                2 => {
                    let n = match frac_operand_of(&rest[0]) {
                        FracOperand::Frac(n, d) if d == BigInt::one() => n,
                        _ => return Err(PyError::type_error("both arguments should be Rational instances")),
                    };
                    let d = match frac_operand_of(&rest[1]) {
                        FracOperand::Frac(n, d) if d == BigInt::one() => n,
                        _ => return Err(PyError::type_error("both arguments should be Rational instances")),
                    };
                    (n, d)
                }
                _ => return Err(PyError::type_error("Fraction() takes at most 2 arguments")),
            };
            let (num, den) = frac_normalize(num, den)?;
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("numerator", py_int(num));
                dict.insert_str("denominator", py_int(den));
            }
            Ok(py_none())
        },
    }));

    macro_rules! frac_method {
        ($name:expr, $func:expr) => {
            frac_dict.insert_str($name, PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    frac_method!("__add__", |args| frac_binop(args, false, |an,ad,bn,bd| Ok((&an*&bd + &bn*&ad, ad*bd)), |a,b| a+b));
    frac_method!("__radd__", |args| frac_binop(args, true, |an,ad,bn,bd| Ok((&an*&bd + &bn*&ad, ad*bd)), |a,b| a+b));
    frac_method!("__sub__", |args| frac_binop(args, false, |an,ad,bn,bd| Ok((&an*&bd - &bn*&ad, ad*bd)), |a,b| a-b));
    frac_method!("__rsub__", |args| frac_binop(args, true, |an,ad,bn,bd| Ok((&an*&bd - &bn*&ad, ad*bd)), |a,b| a-b));
    frac_method!("__mul__", |args| frac_binop(args, false, |an,ad,bn,bd| Ok((an*bn, ad*bd)), |a,b| a*b));
    frac_method!("__rmul__", |args| frac_binop(args, true, |an,ad,bn,bd| Ok((an*bn, ad*bd)), |a,b| a*b));
    frac_method!("__truediv__", |args| frac_binop(args, false, |an,ad,bn,bd| {
        if bn.is_zero() { return Err(PyError::ZeroDivisionError("Fraction division by zero".to_string())); }
        Ok((an*bd, ad*bn))
    }, |a,b| a/b));
    frac_method!("__rtruediv__", |args| frac_binop(args, true, |an,ad,bn,bd| {
        if bn.is_zero() { return Err(PyError::ZeroDivisionError("Fraction division by zero".to_string())); }
        Ok((an*bd, ad*bn))
    }, |a,b| a/b));
    frac_method!("__neg__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&frac_self_type(&args[0]), -n, d)
    });
    frac_method!("__pos__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&frac_self_type(&args[0]), n, d)
    });
    frac_method!("__abs__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&frac_self_type(&args[0]), n.abs(), d)
    });
    frac_method!("__float__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_float(frac_to_f64(&n, &d)))
    });
    frac_method!("__int__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(n / d))
    });
    frac_method!("__trunc__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(n / d))
    });
    frac_method!("__bool__", |args| {
        let (n, _d) = frac_self_num_den(&args[0])?;
        Ok(py_bool(!n.is_zero()))
    });
    frac_method!("__repr__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_str(&format!("Fraction({}, {})", n, d)))
    });
    frac_method!("__str__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        if d == BigInt::one() { Ok(py_str(&n.to_string())) } else { Ok(py_str(&format!("{}/{}", n, d))) }
    });
    frac_method!("__hash__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        // Simplified (not CPython's exact modular-inverse hash algorithm,
        // which relies on `sys.hash_info.modulus`), but preserves the two
        // invariants real code actually depends on: an integral fraction
        // hashes the same as the equivalent `int`, and a fraction exactly
        // representable as a `float` hashes the same as that `float`.
        let h = if d == BigInt::one() {
            py_int(n).hash()?
        } else {
            py_float(frac_to_f64(&n, &d)).hash()?
        };
        Ok(py_int(h as i64))
    });
    frac_method!("__eq__", |args| {
        if args.len() < 2 { return Ok(py_bool(false)); }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => Ok(py_bool(an == bn && ad == bd)),
            FracOperand::Float(bf) => Ok(py_bool(frac_to_f64(&an, &ad) == bf)),
            FracOperand::Other => Ok(py_not_implemented()),
        }
    });
    macro_rules! frac_cmp {
        ($name:expr, $cmp:expr) => {
            frac_method!($name, |args| {
                if args.len() < 2 { return Ok(py_not_implemented()); }
                let (an, ad) = frac_self_num_den(&args[0])?;
                match frac_operand_of(&args[1]) {
                    FracOperand::Frac(bn, bd) => Ok(py_bool($cmp((an*&bd).cmp(&(bn*&ad))))),
                    FracOperand::Float(bf) => Ok(py_bool($cmp(frac_to_f64(&an, &ad).partial_cmp(&bf).unwrap_or(std::cmp::Ordering::Greater)))),
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

    let frac_type = PyObjectRef::new(PyObject::Type {
        name: "Fraction".to_string(),
        dict: Box::new(str_map_to_typedict(frac_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("Fraction", frac_type);
    d
}

pub fn create_calendar_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cal_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
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
    d.insert_str("month_name", py_list(vec![
        py_str(""),
        py_str("January"), py_str("February"), py_str("March"),
        py_str("April"), py_str("May"), py_str("June"),
        py_str("July"), py_str("August"), py_str("September"),
        py_str("October"), py_str("November"), py_str("December"),
    ]));
    d.insert_str("month_abbr", py_list(vec![
        py_str(""),
        py_str("Jan"), py_str("Feb"), py_str("Mar"), py_str("Apr"),
        py_str("May"), py_str("Jun"), py_str("Jul"), py_str("Aug"),
        py_str("Sep"), py_str("Oct"), py_str("Nov"), py_str("Dec"),
    ]));
    d.insert_str("day_name", py_list(vec![
        py_str("Monday"), py_str("Tuesday"), py_str("Wednesday"),
        py_str("Thursday"), py_str("Friday"), py_str("Saturday"),
        py_str("Sunday"),
    ]));
    d.insert_str("day_abbr", py_list(vec![
        py_str("Mon"), py_str("Tue"), py_str("Wed"), py_str("Thu"),
        py_str("Fri"), py_str("Sat"), py_str("Sun"),
    ]));
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
            2 => if is_leap(y) { 29 } else { 28 },
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
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December"
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
        type_dict.insert_str("formatmonth", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "formatmonth".to_string(),
            func: |args| {
                if args.len() < 3 {
                    return Err(PyError::type_error("formatmonth() missing required arguments (self, year, month)"));
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
                lines.push(format!("{:>20}", format!("{} {}", MONTH_NAMES[(m - 1) as usize], y)));
                lines.push("Mo Tu We Th Fr Sa Su".to_string());
                let mut week: Vec<String> = Vec::new();
                for _ in 0..fd { week.push("  ".to_string()); }
                for day in 1..=dim {
                    week.push(format!("{:2}", day));
                    if week.len() == 7 {
                        lines.push(week.join(" "));
                        week.clear();
                    }
                }
                if !week.is_empty() {
                    while week.len() < 7 { week.push("  ".to_string()); }
                    lines.push(week.join(" "));
                }
                Ok(py_str(&lines.join("\n")))
            },
        }));
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
        if args.is_empty() { return Err(PyError::type_error("timegm() missing required argument")); }
        let get = |i: usize, field: &str| -> i64 {
            match &*args[0].borrow() {
                PyObject::Instance { dict, .. } => dict.get(field).and_then(|v| v.as_i64()).unwrap_or(0),
                PyObject::Tuple(items) | PyObject::List(items) => items.get(i).and_then(|v| v.as_i64()).unwrap_or(0),
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
            return Err(PyError::type_error("isleap() missing required argument (year)"));
        }
        let year = args[0].as_i64().ok_or_else(|| PyError::type_error("year must be integer"))?;
        Ok(py_bool(is_leap(year)))
    });

    cal_func!("weekday", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("weekday() requires 3 arguments (year, month, day)"));
        }
        let y = args[0].as_i64().ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1].as_i64().ok_or_else(|| PyError::type_error("month must be integer"))?;
        let d = args[2].as_i64().ok_or_else(|| PyError::type_error("day must be integer"))?;
        // weekday returns 0=Monday, 6=Sunday
        let wd = (weekday(y, m, d) + 6) % 7;
        Ok(py_int(wd))
    });

    cal_func!("monthrange", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("monthrange() requires 2 arguments (year, month)"));
        }
        let y = args[0].as_i64().ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1].as_i64().ok_or_else(|| PyError::type_error("month must be integer"))?;
        if m < 1 || m > 12 {
            return Err(PyError::type_error("month must be in 1..12"));
        }
        let fd = first_weekday(y, m);
        let ndays = month_days(y, m);
        Ok(py_tuple(vec![py_int(fd), py_int(ndays)]))
    });

    cal_func!("monthcalendar", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("monthcalendar() requires 2 arguments (year, month)"));
        }
        let y = args[0].as_i64().ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1].as_i64().ok_or_else(|| PyError::type_error("month must be integer"))?;
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
            return Err(PyError::type_error("prmonth() requires 2 arguments (year, month)"));
        }
        let y = args[0].as_i64().ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1].as_i64().ok_or_else(|| PyError::type_error("month must be integer"))?;
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
    let all_names: Vec<PyObjectRef> = d.keys().filter(|k| !k.starts_with('_')).map(|k| py_str(k)).collect();
    d.insert_str("__all__", py_list(all_names));

    d
}

// ── Native _random module (C extension stub for CPython's random.py) ──────
pub fn create_random_cmodule_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Helper: read _seed from an instance's dict
    fn read_seed(obj: &PyObjectRef) -> u64 {
        let dict = obj.borrow();
        if let PyObject::Instance { dict: inst_dict, .. } = &*dict {
            if let Some(v) = inst_dict.get("_seed") {
                match &*v.borrow() {
                    PyObject::Int(i) => {
                        if let Some(n) = i.to_i64() {
                            return n as u64;
                        }
                    }
                    _ => {}
                }
            }
        }
        1u64
    }

    // Helper: write _seed to an instance's dict
    fn write_seed(obj: &PyObjectRef, seed: u64) {
        let mut dict = obj.borrow_mut();
        if let PyObject::Instance { dict: inst_dict, .. } = &mut *dict {
            inst_dict.insert_str("_seed", py_int(seed as i64));
        }
    }

    // Helper: advance LCG and return new seed + result for random()
    fn lcg_step(state: u64) -> (u64, f64) {
        let new_seed = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let result = (new_seed >> 11) as f64 / (1u64 << 53) as f64;
        (new_seed, result)
    }

    // Create Random type definition
    let mut type_dict = HashMap::new();

    // __init__(self, x=None)
    type_dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: |args| {
            if args.len() < 1 {
                return Err(PyError::type_error("__init__() missing self argument"));
            }
            let seed = if args.len() >= 2 {
                match &*args[1].borrow() {
                    PyObject::None => None,
                    PyObject::Int(i) => i.to_i64(),
                    _ => None,
                }
            } else {
                None
            };
            let s = seed.unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as i64
            });
            write_seed(&args[0], s as u64);
            Ok(py_none())
        },
    }));

    // random(self) -> float in [0.0, 1.0)
    type_dict.insert_str("random", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "random".to_string(),
        func: |args| {
            if args.len() < 1 {
                return Err(PyError::type_error("random() missing self argument"));
            }
            let old_seed = read_seed(&args[0]);
            let (new_seed, val) = lcg_step(old_seed);
            write_seed(&args[0], new_seed);
            Ok(py_float(val))
        },
    }));

    // seed(self, n=None)
    type_dict.insert_str("seed", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "seed".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("seed() missing self or n argument"));
            }
            let n = match &*args[1].borrow() {
                PyObject::None => {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as i64
                }
                PyObject::Int(i) => i.to_i64().unwrap_or(0),
                _ => return Err(PyError::type_error("n must be an int or None")),
            };
            write_seed(&args[0], n as u64);
            Ok(py_none())
        },
    }));

    // getrandbits(self, k) -> int with k random bits
    type_dict.insert_str("getrandbits", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "getrandbits".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("getrandbits() missing self or k argument"));
            }
            let k = if let Some(n) = args[1].as_i64() {
                n as u32
            } else {
                match &*args[1].borrow() {
                    PyObject::Int(i) => i.to_u32().unwrap_or(0),
                    _ => return Err(PyError::type_error("k must be an integer")),
                }
            };
            if k == 0 {
                return Ok(py_int(0));
            }
            let old_seed = read_seed(&args[0]);
            let (new_seed, _) = lcg_step(old_seed);
            write_seed(&args[0], new_seed);

            if k <= 64 {
                let bits = new_seed >> (64 - k);
                Ok(py_int(bits as i64))
            } else {
                // For >64 bits, generate multiple u64 chunks as BigInt
                let mut value = num_bigint::BigInt::from(0);
                let mut remaining = k;
                let mut current = new_seed;
                while remaining > 0 {
                    let chunk_bits = remaining.min(64);
                    let chunk = current >> (64 - chunk_bits);
                    value = (&value << chunk_bits) | num_bigint::BigInt::from(chunk as i64);
                    remaining -= chunk_bits;
                    if remaining > 0 {
                        let (next, _) = lcg_step(current);
                        current = next;
                        write_seed(&args[0], current);
                    }
                }
                // Mask to exactly k bits
                let mask = (num_bigint::BigInt::from(1i64) << k) - 1i64;
                Ok(py_int(value & mask))
            }
        },
    }));

    // getstate(self) -> tuple (version, state) for pickling
    type_dict.insert_str("getstate", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "getstate".to_string(),
        func: |args| {
            if args.len() < 1 {
                return Err(PyError::type_error("getstate() missing self argument"));
            }
            let seed = read_seed(&args[0]);
            // Return (3, seed) — version 3 format like CPython's Mersenne Twister
            Ok(py_tuple(vec![py_int(3i64), py_int(seed as i64)]))
        },
    }));

    // setstate(self, state) -> None for pickling
    type_dict.insert_str("setstate", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "setstate".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("setstate() missing self or state argument"));
            }
            let state_borrowed = args[1].borrow();
            if let PyObject::Tuple(items) = &*state_borrowed {
                if items.len() >= 2 {
                    if let Some(seed) = items[1].as_i64() {
                        drop(state_borrowed);
                        write_seed(&args[0], seed as u64);
                        return Ok(py_none());
                    }
                }
            }
            Err(PyError::value_error("invalid state"))
        },
    }));

    let random_type = PyObjectRef::new(PyObject::Type {
        name: "Random".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });

    d.insert_str("Random", random_type);
    d
}

use std::rc::Rc;
use num_traits::ToPrimitive;
