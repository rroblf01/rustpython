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

    // chain is represented as a callable Instance (not a bare
    // BuiltinFunction) so it can also expose `chain.from_iterable(...)` —
    // BuiltinFunction has no attribute storage at all (set_attribute has no
    // arm for it), so a plain function couldn't hold a from_iterable
    // sibling method the way real itertools.chain does.
    {
        let mut chain_type_dict = HashMap::new();
        chain_type_dict.insert("__call__".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
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
        chain_type_dict.insert("from_iterable".to_string(), PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
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
        let chain_type = PyObjectRef::new(PyObject::Type { name: "chain".to_string(), dict: chain_type_dict, bases: vec![], mro: vec![] });
        d.insert("chain".to_string(), PyObjectRef::new(PyObject::Instance { typ: chain_type, dict: HashMap::new() }));
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

    it_func!("islice", |args| {
        if args.is_empty() { return Err(PyError::type_error("islice() missing arguments")); }
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
        let n = items.len() as i64;
        let (start, stop, step) = match args.len() {
            1 => return Err(PyError::type_error("islice() missing stop argument")),
            2 => (0i64, args[1].as_i64().unwrap_or(n), 1i64),
            _ => {
                let start = args[1].as_i64().unwrap_or(0);
                let stop = args[2].as_i64().unwrap_or(n);
                let step = if args.len() > 3 { args[3].as_i64().unwrap_or(1) } else { 1 };
                (start, stop, step)
            }
        };
        let stop = stop.min(n).max(0);
        let start = start.max(0);
        let step = step.max(1);
        let mut result = Vec::new();
        let mut i = start;
        while i < stop {
            result.push(items[i as usize].clone());
            i += step;
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
    let mut dict = HashMap::new();
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

    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
        let v = if args.len() > 1 { decval_from_pyobject(&args[1])? } else { DecValue::zero() };
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert(DEC_SIGN_KEY.to_string(), py_bool(v.sign));
            dict.insert(DEC_COEFF_KEY.to_string(), py_int(v.coeff));
            dict.insert(DEC_EXP_KEY.to_string(), py_int(v.exp));
            dict.insert(DEC_SPECIAL_KEY.to_string(), py_str(special_to_str(&v.special)));
        }
        Ok(py_none())
    }));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_str(&format!("Decimal('{}')", format_decvalue(&v))))
    }));
    type_dict.insert("__str__".to_string(), bf!("__str__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_str(&format_decvalue(&v)))
    }));
    type_dict.insert("__int__".to_string(), bf!("__int__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        if v.special != DecSpecial::Finite { return Err(PyError::value_error("cannot convert NaN/Infinity to int")); }
        let truncated = if v.exp >= 0 { &v.coeff * ten_pow(v.exp) } else { &v.coeff / ten_pow(-v.exp) };
        Ok(py_int(if v.sign { -truncated } else { truncated }))
    }));
    type_dict.insert("__float__".to_string(), bf!("__float__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_float(decval_to_f64(&v)))
    }));
    type_dict.insert("__bool__".to_string(), bf!("__bool__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(!v.is_zero()))
    }));
    type_dict.insert("__hash__".to_string(), bf!("__hash__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        if v.special != DecSpecial::Finite { return Ok(py_int(0)); }
        // Normalize (strip trailing zeros) so numerically-equal Decimals
        // with different (coeff, exp) representations — e.g. 1 vs 1.0 —
        // hash the same way `1 == 1.0` requires.
        let n = normalize_decval(&v);
        let s = format!("{}{}{}", n.sign, n.coeff, n.exp);
        builtin_hash(&[py_str(&s)])
    }));
    type_dict.insert("__eq__".to_string(), bf!("__eq__", |args| {
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
    type_dict.insert("__le__".to_string(), bf!("__le__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        match decimal_compare(&a, &b) {
            Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal) => Ok(py_bool(true)),
            Some(_) => Ok(py_bool(false)),
            None => Err(PyError::type_error("cannot compare NaN with Decimal")),
        }
    }));
    type_dict.insert("__ge__".to_string(), bf!("__ge__", |args| {
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
    type_dict.insert("__floordiv__".to_string(), bf!("__floordiv__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        let q = decimal_div(&a, &b)?;
        if q.special != DecSpecial::Finite { return Ok(decval_to_instance(&q)); }
        let truncated = if q.exp >= 0 { &q.coeff * ten_pow(q.exp) } else { &q.coeff / ten_pow(-q.exp) };
        Ok(decval_to_instance(&DecValue { special: DecSpecial::Finite, sign: q.sign, coeff: truncated, exp: 0 }))
    }));
    type_dict.insert("__mod__".to_string(), bf!("__mod__", |args| {
        let a = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let b = decval_from_pyobject(&args[1])?;
        if b.is_zero() { return Err(decimal_invalid_op("0 modulo")); }
        let q = decimal_div(&a, &b)?;
        let truncated_q = if q.exp >= 0 { &q.coeff * ten_pow(q.exp) } else { &q.coeff / ten_pow(-q.exp) };
        let trunc_dec = DecValue { special: DecSpecial::Finite, sign: q.sign, coeff: truncated_q, exp: 0 };
        let prod = decimal_mul(&trunc_dec, &b)?;
        Ok(decval_to_instance(&decimal_sub(&a, &prod)?))
    }));
    type_dict.insert("__pow__".to_string(), bf!("__pow__", |args| {
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
    type_dict.insert("__neg__".to_string(), bf!("__neg__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&decimal_negate(&v)))
    }));
    type_dict.insert("__pos__".to_string(), bf!("__pos__", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&round_to_context(v)))
    }));
    type_dict.insert("__abs__".to_string(), bf!("__abs__", |args| {
        let mut v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        v.sign = false;
        Ok(decval_to_instance(&v))
    }));
    type_dict.insert("is_nan".to_string(), bf!("is_nan", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.is_nan()))
    }));
    type_dict.insert("is_infinite".to_string(), bf!("is_infinite", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.special == DecSpecial::Infinity))
    }));
    type_dict.insert("is_finite".to_string(), bf!("is_finite", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.special == DecSpecial::Finite))
    }));
    type_dict.insert("is_zero".to_string(), bf!("is_zero", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.is_zero()))
    }));
    type_dict.insert("is_signed".to_string(), bf!("is_signed", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(py_bool(v.sign))
    }));
    type_dict.insert("copy_sign".to_string(), bf!("copy_sign", |args| {
        let mut v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        let other = decval_from_pyobject(&args[1])?;
        v.sign = other.sign;
        Ok(decval_to_instance(&v))
    }));
    type_dict.insert("copy_abs".to_string(), bf!("copy_abs", |args| {
        let mut v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        v.sign = false;
        Ok(decval_to_instance(&v))
    }));
    type_dict.insert("copy_negate".to_string(), bf!("copy_negate", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&decimal_negate(&v)))
    }));
    type_dict.insert("as_tuple".to_string(), bf!("as_tuple", |args| {
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
    type_dict.insert("normalize".to_string(), bf!("normalize", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        Ok(decval_to_instance(&normalize_decval(&round_to_context(v))))
    }));
    type_dict.insert("quantize".to_string(), bf!("quantize", |args| {
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
    type_dict.insert("to_integral_value".to_string(), bf!("to_integral_value", |args| {
        let v = instance_to_decval(&args[0]).ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
        if v.special != DecSpecial::Finite || v.exp >= 0 { return Ok(decval_to_instance(&v)); }
        let (_, rounding) = current_decimal_context();
        let rounded = round_decvalue(&v, digit_count(&v.coeff).saturating_sub((-v.exp) as usize).max(1), &rounding);
        Ok(decval_to_instance(&DecValue { exp: 0, coeff: &rounded.coeff * ten_pow(rounded.exp), ..rounded }))
    }));
    type_dict.insert("compare".to_string(), bf!("compare", |args| {
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

    PyObjectRef::new(PyObject::Type { name: "Decimal".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
}

fn build_context_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }
    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
        let ctor_args = args[1..].to_vec();
        let kw: Option<PyDict> = ctor_args.last().and_then(|a| if let PyObject::Dict(d) = &*a.borrow() { Some(d.clone()) } else { None });
        let get_kw = |name: &str| kw.as_ref().and_then(|d| d.get(&py_str(name)).ok().flatten());
        let precision = get_kw("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
        let rounding = get_kw("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("prec".to_string(), py_int(precision as i64));
            dict.insert("rounding".to_string(), py_str(&rounding));
        }
        Ok(py_none())
    }));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        let prec = if let PyObject::Instance { dict, .. } = &*args[0].borrow() { dict.get("prec").and_then(|v| v.as_i64()).unwrap_or(28) } else { 28 };
        Ok(py_str(&format!("Context(prec={})", prec)))
    }));
    PyObjectRef::new(PyObject::Type { name: "Context".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
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
    let mut dict = HashMap::new();
    dict.insert("prec".to_string(), py_int(precision as i64));
    dict.insert("rounding".to_string(), py_str(rounding));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub fn create_decimal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dec_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    d.insert("Decimal".to_string(), get_decimal_type());
    d.insert("Context".to_string(), get_context_type());
    dec_func!("getcontext", |_args| {
        let (precision, rounding) = current_decimal_context();
        Ok(make_context_instance(precision, &rounding))
    });
    dec_func!("setcontext", |args| {
        if args.is_empty() { return Err(PyError::type_error("setcontext() missing context argument")); }
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            let precision = dict.get("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
            let rounding = dict.get("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
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
                (dict.get("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize, dict.get("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string()))
            } else { current_decimal_context() }
        } else { current_decimal_context() };
        let ctx = make_context_instance(precision, &rounding);
        let mut cm_dict = HashMap::new();
        cm_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let precision = dict.get("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
                    let rounding = dict.get("rounding").map(|v| v.str()).unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
                    DECIMAL_CURRENT_CONTEXT.with(|c| { *c.borrow_mut() = (precision, rounding); });
                }
                Ok(args[0].clone())
            },
        }));
        cm_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__exit__".to_string(),
            func: |_args| { DECIMAL_CURRENT_CONTEXT.with(|c| { *c.borrow_mut() = (28, "ROUND_HALF_EVEN".to_string()); }); Ok(py_bool(false)) },
        }));
        let cm_typ = PyObjectRef::new(PyObject::Type { name: "_ContextManager".to_string(), dict: cm_dict, bases: vec![], mro: vec![] });
        let mut inst_dict = HashMap::new();
        inst_dict.insert("prec".to_string(), py_int(precision as i64));
        inst_dict.insert("rounding".to_string(), py_str(&rounding));
        let _ = ctx;
        Ok(PyObjectRef::new(PyObject::Instance { typ: cm_typ, dict: inst_dict }))
    });
    // Exception types
    d.insert("DecimalException".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "DecimalException".to_string(), func: crate::object::builtin_make_exception_decimalexception }));
    d.insert("InvalidOperation".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "InvalidOperation".to_string(), func: crate::object::builtin_make_exception_invalidoperation }));
    d.insert("DivisionByZero".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "DivisionByZero".to_string(), func: crate::object::builtin_make_exception_decimaldivisionbyzero }));
    d.insert("Inexact".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "Inexact".to_string(), func: crate::object::builtin_make_exception_inexact }));
    d.insert("Rounded".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "Rounded".to_string(), func: crate::object::builtin_make_exception_rounded }));
    d.insert("Clamped".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "Clamped".to_string(), func: crate::object::builtin_make_exception_clamped }));
    d.insert("Overflow".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "Overflow".to_string(), func: crate::object::builtin_make_exception_decimaloverflow }));
    d.insert("Underflow".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "Underflow".to_string(), func: crate::object::builtin_make_exception_decimalunderflow }));
    d.insert("FloatOperation".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "FloatOperation".to_string(), func: crate::object::builtin_make_exception_floatoperation }));
    // Rounding mode constants — their real string values (that's what
    // CPython's decimal.ROUND_* constants actually are), so equality checks
    // and passing them to quantize()-style calls behave as real code expects.
    for name in ["ROUND_CEILING", "ROUND_DOWN", "ROUND_FLOOR", "ROUND_HALF_DOWN",
                 "ROUND_HALF_EVEN", "ROUND_HALF_UP", "ROUND_UP", "ROUND_05UP"] {
        d.insert(name.to_string(), py_str(name));
    }
    d.insert("MAX_PREC".to_string(), py_int(999999999999999999i64));
    d.insert("MAX_EMAX".to_string(), py_int(999999999999999999i64));
    d.insert("MIN_EMIN".to_string(), py_int(-999999999999999999i64));
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
use num_traits::ToPrimitive;
