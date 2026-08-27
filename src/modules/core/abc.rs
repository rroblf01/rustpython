use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;

static ABC_CACHE_TOKEN: AtomicI64 = AtomicI64::new(0);

fn _abc_get_cache_token(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(py_int(
        ABC_CACHE_TOKEN.load(std::sync::atomic::Ordering::Relaxed),
    ))
}

fn _abc_init(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error(
            "_abc_init() requires at least 1 argument",
        ));
    }
    let cls = &args[0];
    // Set _abc_impl if not already present (computed from bases)
    let needs_impl = {
        let b = cls.borrow();
        match &*b {
            PyObject::Type { dict, .. } => !dict.contains_key_str("_abc_impl"),
            _ => return Err(PyError::type_error("_abc_init() argument must be a type")),
        }
    };
    if needs_impl {
        let bases: Vec<PyObjectRef> = {
            let b = cls.borrow();
            match &*b {
                PyObject::Type { bases, .. } => bases.clone(),
                _ => vec![],
            }
        };
        let mut impl_set = PySet::new();
        for base in &bases {
            // Each ABC base contributes its _abc_impl (or itself)
            if let Ok(abc_impl) = base.borrow().get_attribute("_abc_impl") {
                if let PyObject::FrozenSet(items) = &*abc_impl.borrow() {
                    for item in items.to_vec() {
                        impl_set.add(item)?;
                    }
                }
            }
            // Also add base itself if it's an instance of ABCMeta (has _abc_impl)
            if base.borrow().get_attribute("_abc_impl").is_ok() {
                impl_set.add(base.clone())?;
            }
        }
        cls.borrow_mut()
            .set_attribute("_abc_impl", PyObjectRef::imm(PyObject::FrozenSet(impl_set)))?;
    }
    // Ensure standard ABC attributes exist
    for attr in &["_abc_registry", "_abc_cache", "_abc_negative_cache"] {
        let has = cls.borrow().get_attribute(attr).is_ok();
        if !has {
            cls.borrow_mut().set_attribute(attr, py_set())?;
        }
    }
    let has_ver = cls
        .borrow()
        .get_attribute("_abc_negative_cache_version")
        .is_ok();
    if !has_ver {
        cls.borrow_mut()
            .set_attribute("_abc_negative_cache_version", py_int(0))?;
    }
    Ok(py_none())
}

fn _abc_register(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "_abc_register() requires at least 2 arguments",
        ));
    }
    let cls = &args[0];
    let subclass = &args[1].clone();
    // Ensure registry exists — use a FrozenSet
    if cls.borrow().get_attribute("_abc_registry").is_err() {
        cls.borrow_mut().set_attribute(
            "_abc_registry",
            PyObjectRef::imm(PyObject::FrozenSet(PySet::new())),
        )?;
    }
    // Get current registry, add subclass if not already present
    let mut registered = {
        let r = cls.borrow().get_attribute("_abc_registry")?;
        let b = r.borrow();
        match &*b {
            PyObject::FrozenSet(items) => items.to_vec(),
            _ => vec![],
        }
    };
    if !registered.iter().any(|r| r.is(subclass)) {
        registered.push(subclass.clone());
    }
    // Build PySet from registered Vec
    let mut reg_set = PySet::new();
    for item in &registered {
        reg_set.add(item.clone())?;
    }
    cls.borrow_mut().set_attribute(
        "_abc_registry",
        PyObjectRef::imm(PyObject::FrozenSet(reg_set)),
    )?;
    // Invalidate cache
    ABC_CACHE_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(args[1].clone())
}

fn _abc_instancecheck(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Stub: fall back to normal Python isinstance
    if args.len() < 2 {
        return Err(PyError::type_error(
            "_abc_instancecheck() requires at least 2 arguments",
        ));
    }
    Ok(py_bool(false))
}

fn _abc_subclasscheck(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Stub: fall back to normal Python issubclass
    if args.len() < 2 {
        return Err(PyError::type_error(
            "_abc_subclasscheck() requires at least 2 arguments",
        ));
    }
    Ok(py_bool(false))
}

fn _abc_get_dump(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error(
            "_get_dump() requires at least 1 argument",
        ));
    }
    let cls = &args[0];
    let registry = cls
        .borrow()
        .get_attribute("_abc_registry")
        .unwrap_or_else(|_| py_dict());
    let cache = cls
        .borrow()
        .get_attribute("_abc_cache")
        .unwrap_or_else(|_| py_dict());
    let neg_cache = cls
        .borrow()
        .get_attribute("_abc_negative_cache")
        .unwrap_or_else(|_| py_dict());
    let version = cls
        .borrow()
        .get_attribute("_abc_negative_cache_version")
        .unwrap_or(py_int(0));
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        registry, cache, neg_cache, version,
    ])))
}

fn _abc_reset_registry(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error(
            "_reset_registry() requires at least 1 argument",
        ));
    }
    let cls = &args[0];
    cls.borrow_mut().set_attribute("_abc_registry", py_set())?;
    Ok(py_none())
}

fn _abc_reset_caches(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error(
            "_reset_caches() requires at least 1 argument",
        ));
    }
    let cls = &args[0];
    cls.borrow_mut().set_attribute("_abc_cache", py_set())?;
    cls.borrow_mut()
        .set_attribute("_abc_negative_cache", py_set())?;
    Ok(py_none())
}

/// ABCMeta(name, bases, namespace) -> new class with ABC support.
///
/// This is the metaclass that `abc.ABCMeta` inherits from `type` in CPython.
/// Since our RustPython doesn't have a full C-level `type` metaclass with
/// 3-argument form, we expose a builtin function that creates a class and
/// calls `_abc_init` to set up ABC data structures.
fn _abc_abcmeta(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error(
            "ABCMeta() requires at least 3 arguments",
        ));
    }
    let name_str = args[0].str();
    let bases_vec = if let PyObject::Tuple(t) = &*args[1].borrow() {
        t.clone()
    } else {
        return Err(PyError::type_error("ABCMeta() bases must be a tuple"));
    };
    let namespace_dict = {
        let b = args[2].borrow();
        if let PyObject::Dict(d) = &*b {
            let mut h = HashMap::new();
            for (k, v) in d.items() {
                h.insert(k.str(), v);
            }
            h
        } else {
            return Err(PyError::type_error("ABCMeta() namespace must be a dict"));
        }
    };
    let class = PyObjectRef::new(PyObject::Type {
        name: name_str,
        dict: Box::new(str_map_to_typedict(namespace_dict)),
        bases: bases_vec.clone(),
        mro: vec![],
    });
    // Compute and set MRO
    let mut mro = vec![class.clone()];
    for base in &bases_vec {
        mro.push(base.clone());
    }
    if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
        *mro_field = mro;
    }
    // Run _abc_init to set up ABC data structures
    let _ = _abc_init(&[class.clone()]);
    Ok(class)
}

/// Create the `_abc` module dictionary.
pub fn create_abc_builtins_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "ABCMeta",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "ABCMeta".to_string(),
            func: _abc_abcmeta,
        }),
    );
    d.insert_str(
        "get_cache_token",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "get_cache_token".to_string(),
            func: _abc_get_cache_token,
        }),
    );
    d.insert_str(
        "_abc_init",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_abc_init".to_string(),
            func: _abc_init,
        }),
    );
    d.insert_str(
        "_abc_register",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_abc_register".to_string(),
            func: _abc_register,
        }),
    );
    d.insert_str(
        "_abc_instancecheck",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_abc_instancecheck".to_string(),
            func: _abc_instancecheck,
        }),
    );
    d.insert_str(
        "_abc_subclasscheck",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_abc_subclasscheck".to_string(),
            func: _abc_subclasscheck,
        }),
    );
    d.insert_str(
        "_get_dump",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_get_dump".to_string(),
            func: _abc_get_dump,
        }),
    );
    d.insert_str(
        "_reset_registry",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_reset_registry".to_string(),
            func: _abc_reset_registry,
        }),
    );
    d.insert_str(
        "_reset_caches",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_reset_caches".to_string(),
            func: _abc_reset_caches,
        }),
    );
    d
}
