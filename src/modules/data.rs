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

pub fn create_collections_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! coll_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // deque: double-ended queue
    coll_func!("deque", |args| {
        let iterable = if args.len() > 0 { Some(args[0].clone()) } else { None };
        let mut deque = std::collections::VecDeque::new();
        if let Some(iter) = iterable {
            // Iterate over the iterable and add items
            if let Ok(it) = crate::object::builtin_iter(&[iter]) {
                loop {
                    match crate::object::builtin_next(&[it.clone()]) {
                        Ok(v) => deque.push_back(v),
                        Err(crate::object::PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(PyObjectRef::new(PyObject::List(deque.into_iter().collect())))
    });

    // Counter: count hashable objects
    coll_func!("Counter", |args| {
        if args.is_empty() {
            return Ok(crate::object::py_dict());
        }
        let iterable = &args[0];
        let mut counts = std::collections::HashMap::<usize, (PyObjectRef, i64)>::new();
        let mut order = Vec::new();
        if let Ok(it) = crate::object::builtin_iter(&[iterable.clone()]) {
            loop {
                match crate::object::builtin_next(&[it.clone()]) {
                    Ok(item) => {
                        let hash = item.hash()?;
                        let entry = counts.entry(hash).or_insert_with(|| {
                            order.push(hash);
                            (item.clone(), 0)
                        });
                        entry.1 += 1;
                    }
                    Err(crate::object::PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        let dict = crate::object::py_dict();
        for hash in &order {
            if let Some((item, count)) = counts.get(hash) {
                let count_val = crate::object::py_int(*count);
                if let crate::object::PyObject::Dict(d) = &mut *dict.borrow_mut() {
                    d.set(item.clone(), count_val)?;
                }
            }
        }
        Ok(dict)
    });

    // defaultdict: dict subclass that calls factory for missing keys
    coll_func!("defaultdict", |args| {
        let dict = crate::object::py_dict();
        // Note: source dict copy not implemented (private buckets field)
        // defaultdict(default_factory) and defaultdict(default_factory, initial_dict)
        // work with the initial_dict as a separate argument.
        let factory = if args.len() > 0 { Some(args[0].clone()) } else { None };
        if let Some(f) = factory {
            let _ = dict.borrow_mut().set_attribute("default_factory", f);
        }
        Ok(dict)
    });

    // OrderedDict: remembers insertion order (handled by PyDict ordering)
    coll_func!("OrderedDict", |args| { Ok(crate::object::py_dict()) });

    // namedtuple: factory function — creates simple types with named fields
    coll_func!("namedtuple", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "namedtuple() needs at least 2 arguments",
            ));
        }
        let typename = args[0].str();
        let field_str = args[1].str();
        let fields: Vec<String> = field_str.split_whitespace().map(|s| s.to_string()).collect();
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
        type_dict.insert("__init__".to_string(), init_obj);
        // Add field names as class-level attributes (for __doc__ setting support)
        for f in &fields {
            type_dict.insert(f.clone(), PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "member_descriptor".to_string(),
                    dict: HashMap::new(),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: HashMap::new(),
            }));
        }
        Ok(PyObjectRef::new(PyObject::Type {
            name: typename,
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        }))
    });

    // collections.abc submodule (Iterable, Hashable, etc.)
    d.insert("abc".to_string(), create_module("collections.abc", create_collections_abc_dict()));

    d
}

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
        let mut acc = match builtin_next(&[it.clone()]) {
            Ok(v) => v,
            Err(PyError::StopIteration) => {
                if args.len() >= 3 { return Ok(args[2].clone()); }
                return Err(PyError::type_error("reduce() of empty sequence with no initial value"));
            }
            Err(e) => return Err(e),
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
        let has_le = cls.borrow().get_attribute("__le__").is_ok();
        let has_lt = cls.borrow().get_attribute("__lt__").is_ok();
        let has_ge = cls.borrow().get_attribute("__ge__").is_ok();
        let has_gt = cls.borrow().get_attribute("__gt__").is_ok();
        let has_eq = cls.borrow().get_attribute("__eq__").is_ok();
        // Basic stub: this doesn't implement all the methods, just returns the class
        // A real implementation would need to add __le__/__lt__/__ge__/__gt__/__eq__/__ne__
        Ok(cls)
    });

    // cached_property: descriptor that caches property value on first access
    ft_func!("cached_property", |args| {
        // Return the argument as-is (basic stub — doesn't cache)
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
    // lru_cache(maxsize): simple memoization cache
    ft_func!("lru_cache", |args| {
        let maxsize = if !args.is_empty() {
            match &*args[0].borrow() {
                PyObject::Int(i) => i.to_i64().unwrap_or(128) as usize,
                _ => 128,
            }
        } else { 128 };
        // Return a decorator (closure) that wraps functions
        let decorator = move |dec_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if dec_args.is_empty() {
                return Err(PyError::type_error("lru_cache requires a function argument"));
            }
            let func = dec_args[0].clone();
            let cache = std::cell::RefCell::new(
                std::collections::HashMap::<String, PyObjectRef>::new(),
            );
            let maxsize = maxsize;
            let cache_wrapper = move |inner_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                let key = inner_args
                    .iter()
                    .map(|a| format!("{:?}", a))
                    .collect::<Vec<_>>()
                    .join(",");
                let mut cache = cache.borrow_mut();
                if let Some(cached) = cache.get(&key) {
                    return Ok(cached.clone());
                }
                let result = builtin_call(&func, inner_args)?;
                if cache.len() < maxsize {
                    cache.insert(key, result.clone());
                }
                Ok(result)
            };
            Ok(PyObjectRef::new(PyObject::Closure(Rc::new(cache_wrapper))))
        };
        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(decorator))))
    });

    // cache: alias for lru_cache(maxsize=None) (unbounded cache)
    ft_func!("cache", |args| {
            let decorator = move |dec_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if dec_args.is_empty() {
                    return Err(PyError::type_error("cache requires a function argument"));
                }
                let func = dec_args[0].clone();
                let cache = std::rc::Rc::new(std::cell::RefCell::new(
                    std::collections::HashMap::<String, PyObjectRef>::new(),
                ));
                let func_for_cache = func.clone();
                let cache_wrapper = move |inner_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    let key = inner_args
                        .iter()
                        .map(|a| format!("{:?}", a))
                        .collect::<Vec<_>>()
                        .join(",");
                    let mut c = cache.borrow_mut();
                    if let Some(result) = c.get(&key) {
                        return Ok(result.clone());
                    }
                    let result = builtin_call(&func_for_cache, inner_args)?;
                    c.insert(key, result.clone());
                    Ok(result)
                };
                let wrapper = PyObjectRef::new(PyObject::Closure(Rc::new(cache_wrapper)));
                let _ = wrapper.borrow_mut().set_attribute("__wrapped__", func);
                Ok(wrapper)
            };
            Ok(PyObjectRef::new(PyObject::Closure(Rc::new(decorator))))
        });

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
            reg.insert("object".to_string(), func.clone());
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
        call_type_dict.insert("__call__".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            dispatch_rc(args)
        }))));
        let dispatcher = PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "singledispatch".to_string(),
                dict: call_type_dict,
                bases: vec![],
                mro: vec![],
            }),
            dict: HashMap::new(), // attributes like .register, .registry go here
        });
        {
            let mut py_registry = PyDict::new();
            let reg = registry.borrow();
            for (type_name, impl_func) in reg.iter() {
                py_registry.set(py_str(type_name), impl_func.clone()).ok();
            }
            let _ = dispatcher.borrow_mut().set_attribute("registry", PyObjectRef::new(PyObject::Dict(py_registry)));
        }
        let reg_register = registry.clone();
        let dispatch_clone = dispatcher.clone();
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
            type_dict.insert("__lt__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(lt))));
            type_dict.insert("__le__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(le))));
            type_dict.insert("__gt__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(gt))));
            type_dict.insert("__ge__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(ge))));
            type_dict.insert("__eq__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(eq))));
            type_dict.insert("__ne__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(ne))));
            type_dict.insert("__hash__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(hash_err))));

            let key_obj = PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "cmp_to_key".to_string(),
                    dict: type_dict,
                    bases: vec![],
                    mro: vec![],
                }),
                dict: std::collections::HashMap::new(),
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

    it_func!("chain", |args| {
        let mut items = Vec::new();
        for arg in args {
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
    });

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

    it_func!("repeat", |args| {
        if args.is_empty() { return Err(PyError::type_error("repeat() missing argument")); }
        let obj = args[0].clone();
        let times = if args.len() > 1 {
            args[1].as_i64().ok_or_else(|| PyError::type_error("times must be int"))? as usize
        } else {
            0 // signal for infinite
        };
        if times == 0 {
            // Infinite repeat — return a list of 1000 items (enough for random.py)
            let mut items = Vec::with_capacity(1000);
            for _ in 0..1000 {
                items.push(obj.clone());
            }
            Ok(py_list(items))
        } else {
            let mut items = Vec::with_capacity(times);
            for _ in 0..times {
                items.push(obj.clone());
            }
            Ok(py_list(items))
        }
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

    d
}

pub fn create_datetime_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dt_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    dt_func!("datetime", |args| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs() as i64;
        let nanos = now.subsec_nanos();
        // Format as ISO string
        let seconds = secs % 60;
        let minutes = (secs / 60) % 60;
        let hours = (secs / 3600) % 24;
        let days = secs / 86400;
        // Approximate year/month/day from days since epoch
        let mut y = 1970i64;
        let mut remaining = days;
        loop {
            let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
            if remaining < year_days { break; }
            remaining -= year_days;
            y += 1;
        }
        let is_leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let month_days = [31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut m = 1usize;
        for days_in_month in &month_days {
            if remaining < *days_in_month { break; }
            remaining -= days_in_month;
            m += 1;
        }
        let d = remaining + 1;
        let date_str = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, hours, minutes, seconds);
        Ok(py_str(&date_str))
    });

    dt_func!("date", |args| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs() as i64;
        let days = secs / 86400;
        let mut y = 1970i64;
        let mut remaining = days;
        loop {
            let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
            if remaining < year_days { break; }
            remaining -= year_days;
            y += 1;
        }
        let is_leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let month_days = [31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut m = 1usize;
        for days_in_month in &month_days {
            if remaining < *days_in_month { break; }
            remaining -= days_in_month;
            m += 1;
        }
        let d = remaining + 1;
        Ok(py_str(&format!("{:04}-{:02}-{:02}", y, m, d)))
    });

    dt_func!("now", |args| {
        let s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(py_float(s.as_secs_f64()))
    });

    // Minimal datetime.time class stub — needed by django.utils.encoding
    dt_func!("time", |args| {
        let hour = if args.len() > 0 { args[0].str().parse::<u32>().unwrap_or(0) } else { 0 };
        let minute = if args.len() > 1 { args[1].str().parse::<u32>().unwrap_or(0) } else { 0 };
        let second = if args.len() > 2 { args[2].str().parse::<u32>().unwrap_or(0) } else { 0 };
        Ok(py_str(&format!("{:02}:{:02}:{:02}", hour, minute, second)))
    });
    dt_func!("timedelta", |args| {
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "timedelta".to_string(),
                dict: HashMap::new(),
                bases: vec![],
                mro: vec![],
            }),
            dict: HashMap::new(),
        }))
    });
    dt_func!("tzinfo", |args| {
        Ok(py_none())
    });
    dt_func!("timezone", |args| {
        Ok(py_none())
    });
    dt_func!("UTC", |args| {
        Ok(py_none())
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

    stat_func!("mean", |args| {
        if args.is_empty() { return Err(PyError::type_error("mean() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.is_empty() {
                return Err(PyError::ValueError("mean() argument is empty".to_string()));
            }
            let mut sum = 0.0f64;
            let mut count = 0usize;
            for item in items {
                let v = item.borrow();
                match &*v {
                    PyObject::Int(i) => { sum += i.to_f64().unwrap_or(0.0); count += 1; }
                    PyObject::Float(f) => { sum += f; count += 1; }
                    _ => return Err(PyError::type_error("mean() argument must contain numbers")),
                }
            }
            Ok(py_float(sum / count as f64))
        } else {
            Err(PyError::type_error("mean() argument must be a list"))
        }
    });

    stat_func!("median", |args| {
        if args.is_empty() { return Err(PyError::type_error("median() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.is_empty() {
                return Err(PyError::ValueError("median() argument is empty".to_string()));
            }
            let mut nums: Vec<f64> = Vec::with_capacity(items.len());
            for item in items {
                let v = item.borrow();
                match &*v {
                    PyObject::Int(i) => nums.push(i.to_f64().unwrap_or(0.0)),
                    PyObject::Float(f) => nums.push(*f),
                    _ => return Err(PyError::type_error("median() argument must contain numbers")),
                }
            }
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = nums.len();
            if n % 2 == 0 {
                Ok(py_float((nums[n/2 - 1] + nums[n/2]) / 2.0))
            } else {
                Ok(py_float(nums[n/2]))
            }
        } else {
            Err(PyError::type_error("median() argument must be a list"))
        }
    });

    stat_func!("stdev", |args| {
        if args.is_empty() { return Err(PyError::type_error("stdev() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.len() < 2 {
                return Err(PyError::ValueError("stdev() requires at least 2 data points".to_string()));
            }
            let mut nums: Vec<f64> = Vec::with_capacity(items.len());
            for item in items {
                let v = item.borrow();
                match &*v {
                    PyObject::Int(i) => nums.push(i.to_f64().unwrap_or(0.0)),
                    PyObject::Float(f) => nums.push(*f),
                    _ => return Err(PyError::type_error("stdev() argument must contain numbers")),
                }
            }
            let n = nums.len() as f64;
            let sum: f64 = nums.iter().sum();
            let mean = sum / n;
            let variance: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
            Ok(py_float(variance.sqrt()))
        } else {
            Err(PyError::type_error("stdev() argument must be a list"))
        }
    });

    stat_func!("mode", |args| {
        if args.is_empty() { return Err(PyError::type_error("mode() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.is_empty() {
                return Err(PyError::ValueError("mode() argument is empty".to_string()));
            }
            let mut counts = std::collections::HashMap::new();
            let mut max_count = 0i64;
            let mut modes: Vec<PyObjectRef> = Vec::new();
            for item in items {
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
        } else {
            Err(PyError::type_error("mode() argument must be a list"))
        }
    });

    // Helper: extract numeric values from a list into Vec<f64>
    fn stat_extract_nums(data: &PyObjectRef) -> PyResult<Vec<f64>> {
        let borrowed = data.borrow();
        if let PyObject::List(items) = &*borrowed {
            if items.is_empty() {
                return Err(PyError::ValueError("list is empty".to_string()));
            }
            let mut nums: Vec<f64> = Vec::with_capacity(items.len());
            for item in items {
                let v = item.borrow();
                match &*v {
                    PyObject::Int(i) => nums.push(i.to_f64().unwrap_or(0.0)),
                    PyObject::Float(f) => nums.push(*f),
                    _ => return Err(PyError::type_error("argument must contain numbers")),
                }
            }
            Ok(nums)
        } else {
            Err(PyError::type_error("argument must be a list"))
        }
    }

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

    d
}

pub fn create_decimal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dec_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    dec_func!("Decimal", |args| {
        if args.is_empty() { return Err(PyError::type_error("Decimal() missing argument")); }
        let val = args[0].str();
        Ok(py_str(&format!("Decimal('{}')", val)))
    });
    d
}

pub fn create_fractions_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! frac_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    frac_func!("Fraction", |args| {
        if args.len() < 2 { return Err(PyError::type_error("Fraction() requires 2 arguments")); }
        let n = args[0].as_i64().unwrap_or(0);
        let mut den = args[1].as_i64().unwrap_or(1);
        if den == 0 { return Err(PyError::ValueError("Fraction denominator cannot be zero".to_string())); }
        let mut num = n;
        if den < 0 { num = -num; den = -den; }
        let g = {
            let mut a = num.abs();
            let mut b = den;
            while b != 0 { let t = b; b = a % b; a = t; }
            a
        };
        if g > 1 { num /= g; den /= g; }
        Ok(py_str(&format!("{}/{}", num, den)))
    });
    d
}

pub fn create_calendar_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cal_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Add constants to module
    d.insert("month_name".to_string(), py_list(vec![
        py_str("January"), py_str("February"), py_str("March"),
        py_str("April"), py_str("May"), py_str("June"),
        py_str("July"), py_str("August"), py_str("September"),
        py_str("October"), py_str("November"), py_str("December"),
    ]));
    d.insert("month_abbr".to_string(), py_list(vec![
        py_str("Jan"), py_str("Feb"), py_str("Mar"), py_str("Apr"),
        py_str("May"), py_str("Jun"), py_str("Jul"), py_str("Aug"),
        py_str("Sep"), py_str("Oct"), py_str("Nov"), py_str("Dec"),
    ]));
    d.insert("day_name".to_string(), py_list(vec![
        py_str("Monday"), py_str("Tuesday"), py_str("Wednesday"),
        py_str("Thursday"), py_str("Friday"), py_str("Saturday"),
        py_str("Sunday"),
    ]));
    d.insert("day_abbr".to_string(), py_list(vec![
        py_str("Mon"), py_str("Tue"), py_str("Wed"), py_str("Thu"),
        py_str("Fri"), py_str("Sat"), py_str("Sun"),
    ]));

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
        type_dict.insert("formatmonth".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
        type_dict.insert("formatyear".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
                dict: type_dict,
                bases: vec![],
                mro: vec![],
            }),
            dict: HashMap::new(),
        }))
    });

    // ---- TextCalendar factory ----
    cal_func!("TextCalendar", |args| {
        let _ = args;
        let mut type_dict = HashMap::new();
        type_dict.insert("formatmonth".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
                dict: type_dict,
                bases: vec![],
                mro: vec![],
            }),
            dict: HashMap::new(),
        }))
    });

    // ---- Module-level calendar functions ----
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
            inst_dict.insert("_seed".to_string(), py_int(seed as i64));
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
    type_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    type_dict.insert("random".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    type_dict.insert("seed".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    type_dict.insert("getrandbits".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    type_dict.insert("getstate".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    type_dict.insert("setstate".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
        dict: type_dict,
        bases: vec![],
        mro: vec![],
    });

    d.insert("Random".to_string(), random_type);
    d
}

use std::rc::Rc;
use std::cell::RefCell;
use num_traits::ToPrimitive;
use num_bigint::BigInt;
use std::sync::atomic::{AtomicI64, Ordering};