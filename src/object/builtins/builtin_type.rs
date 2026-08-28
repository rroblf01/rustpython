// Split out of the former monolithic object/builtins.rs — this file holds
// the `type` builtin family (`type(x)` / `type(name, bases, dict)`,
// `type.__new__`) and its primitive-type cache.
use super::*;

thread_local! {
    // `type(x)` for a builtin-native value (int/str/list/...) used to build
    // a BRAND NEW, throwaway `PyObject::Type` on every single call — so
    // `type(5) is type(6)` (and even `type(5) is type(5)`, two separate
    // calls) was ALWAYS `False`, since no two calls ever returned the same
    // object. This is an extremely common idiom (`type(self) is type(other)`
    // total-ordering-style guards, `type(x) == int` checks) — confirmed via
    // CPython's own `test_math.testIsqrt`'s `self.assertIs(type(s), int)`.
    // Caching one canonical Type object per builtin type NAME here fixes
    // same-kind identity comparisons. For a type that has been migrated to
    // a REAL `PyObject::Type` registered in `builtins` (see
    // `NATIVE_VALUE_CTOR_KEY`'s doc comment — `int` as of this writing),
    // `seed_primitive_type_cache` below pre-populates this cache with that
    // SAME canonical object at `create_builtins()` time, so `type(5) is
    // int` is genuinely `True` — not just `type(5) is type(5)`. For any
    // type NOT yet migrated, this cache still falls back to lazily
    // building a fresh placeholder `Type` per name on first use, exactly
    // as before.
    static PRIMITIVE_TYPE_CACHE: std::cell::RefCell<HashMap<String, PyObjectRef>> = std::cell::RefCell::new(HashMap::new());
}

/// Pre-seed `PRIMITIVE_TYPE_CACHE` with the canonical, already-constructed
/// `Type` object for a native value type (called once from
/// `create_builtins()` right after building e.g. `int_type`) — so
/// `builtin_type_of`/`type(x)` returns this SAME object instead of lazily
/// building an unrelated placeholder the first time `type(5)` is called.
pub(crate) fn seed_primitive_type_cache(name: &str, ty: PyObjectRef) {
    PRIMITIVE_TYPE_CACHE.with(|c| {
        c.borrow_mut().insert(name.to_string(), ty);
    });
}
pub(crate) fn get_primitive_type(name: &str) -> Option<PyObjectRef> {
    PRIMITIVE_TYPE_CACHE.with(|c| c.borrow().get(name).cloned())
}

pub fn builtin_type_of(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() == 1 {
        // type(obj) -> return the type of an object
        let borrowed = args[0].borrow();
        match &*borrowed {
            PyObject::Instance { typ, .. } => Ok(typ.clone()),
            PyObject::Type { .. } => Ok(args[0].clone()),
            // `type(exc)` for a native exception instance returns the REAL
            // exception class (the builtin `BuiltinFunction`, e.g.
            // `ZeroDivisionError`) — real CPython: `type(ZeroDivisionError(
            // 'x')) is ZeroDivisionError`. Previously it fell through to the
            // synthetic name-based Type below, so `type(exc_value) ==
            // ZeroDivisionError` was False (test_atexit's unraisable
            // assertion). Fall back to the synthetic Type if the class isn't
            // resolvable (e.g. a module-specific exception).
            PyObject::Exception { typ, .. } | PyObject::ExceptionGroup { typ, .. } => {
                let name = typ.clone();
                drop(borrowed);
                if let Some(cls) = crate::modules::get_builtin_class(&name) {
                    return Ok(cls);
                }
                if let Some(cached) = PRIMITIVE_TYPE_CACHE.with(|c| c.borrow().get(&name).cloned())
                {
                    return Ok(cached);
                }
                let new_type = PyObjectRef::new(PyObject::Type {
                    name: name.clone(),
                    dict: Box::new(TypeDict::default()),
                    bases: vec![],
                    mro: vec![],
                });
                PRIMITIVE_TYPE_CACHE.with(|c| {
                    c.borrow_mut().insert(name, new_type.clone());
                });
                Ok(new_type)
            }
            _ => {
                let name = borrowed.type_name();
                drop(borrowed);
                if let Some(cached) = PRIMITIVE_TYPE_CACHE.with(|c| c.borrow().get(&name).cloned())
                {
                    return Ok(cached);
                }
                let new_type = PyObjectRef::new(PyObject::Type {
                    name: name.clone(),
                    dict: Box::new(TypeDict::default()),
                    bases: vec![],
                    mro: vec![],
                });
                PRIMITIVE_TYPE_CACHE.with(|c| {
                    c.borrow_mut().insert(name, new_type.clone());
                });
                Ok(new_type)
            }
        }
    } else if args.len() == 3 {
        // type(name, bases, dict) -> create a new class (metaclass usage).
        // Delegates to the VM's default_build_class so a dynamically
        // created class gets exactly the same treatment as one from a
        // `class Foo(...):` statement (native-base propagation, real C3
        // MRO, __set_name__, __init_subclass__) instead of the separate,
        // less complete hand-rolled logic this used to have.
        let bases_vec = to_bases_vec(&args[1]);
        let namespace_dict = dict_arg_to_hashmap(&args[2], "type() third argument must be a dict")?;
        with_vm_mut(|vm| {
            vm.default_build_class(args[0].str(), bases_vec, namespace_dict, vec![], None)
        })?
    } else {
        Err(PyError::type_error(
            "type() takes exactly one or three arguments",
        ))
    }
}

fn to_bases_vec(bases: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObject::Tuple(t) = &*bases.borrow() {
        t.clone()
    } else if matches!(&*bases.borrow(), PyObject::None) {
        vec![]
    } else {
        vec![bases.clone()]
    }
}

/// A class-namespace argument (`type.__new__`'s 4th positional arg, or
/// `type(name, bases, ns)`'s 3rd) is usually a plain dict, but when a
/// metaclass has a `__prepare__` returning a real dict-subclass instance
/// (e.g. enum's `_EnumDict`, used to track member-definition order via an
/// overridden `__setitem__` — see `EnumType.__prepare__`), it arrives here
/// as a `PyObject::Instance` whose actual dict contents live in its native
/// backing, not a bare `PyObject::Dict`. Check both.
pub(crate) fn dict_arg_to_hashmap(
    namespace: &PyObjectRef,
    err_msg: &str,
) -> PyResult<HashMap<String, PyObjectRef>> {
    if let Some(native) = native_backing_of(namespace) {
        return dict_arg_to_hashmap(&native, err_msg);
    }
    match &*namespace.borrow() {
        PyObject::Dict(d) => Ok(d.items().into_iter().map(|(k, v)| (k.str(), v)).collect()),
        _ => Err(PyError::type_error(err_msg)),
    }
}

/// `type.__new__(metacls, name, bases, namespace, **kwds)` — the real,
/// CPython-shaped 4-argument metaclass `__new__` convention (distinct from
/// `builtin_type_of`'s `type(x)`/`type(name, bases, ns)` conventions above,
/// which have no `metacls` parameter — kept as two separate functions so
/// the two calling shapes are never ambiguous). Reached when a user
/// metaclass's own `__new__` calls `super().__new__(metacls, name, bases,
/// namespace, **kwds)` and the super-mro walk bottoms out at plain `type`
/// (see `type`'s registration in `create_builtins`).
pub fn type_new_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
        eprintln!(
            "type_new_builtin: args.len()={} args={:?}",
            args.len(),
            args.iter().map(|a| a.repr()).collect::<Vec<_>>()
        );
    }
    if args.len() < 4 {
        return Err(PyError::type_error(
            "type.__new__() takes at least 4 arguments (metacls, name, bases, namespace)",
        ));
    }
    let metacls = args[0].clone();
    let name_str = args[1].str();
    let bases_vec = to_bases_vec(&args[2]);
    let namespace_dict = dict_arg_to_hashmap(&args[3], "type.__new__(): namespace must be a dict")?;
    let kwargs: Vec<(String, PyObjectRef)> = args
        .get(4)
        .map(|d| {
            dict_arg_to_hashmap(d, "")
                .unwrap_or_default()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    let metatype = with_vm_mut(|vm| {
        let is_bare_type = vm
            .builtins
            .get(&interner::intern("type"))
            .map(|t| t.is(&metacls))
            .unwrap_or(false);
        if is_bare_type {
            None
        } else {
            Some(metacls.clone())
        }
    })?;
    with_vm_mut(|vm| vm.default_build_class(name_str, bases_vec, namespace_dict, kwargs, metatype))?
}
