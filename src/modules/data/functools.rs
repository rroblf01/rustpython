use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;

/// `lru_cache`/`cache` source — see VirtualMachine::install_source_defined_stdlib.
pub const FUNCTOOLS_EXTRA_SOURCE: &str = include_str!("../functools_extra.py");

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
