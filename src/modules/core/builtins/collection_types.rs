use crate::object::*;
use std::collections::HashMap;

pub fn register_collection_types(builtins: &mut HashMap<String, PyObjectRef>, object_type: &PyObjectRef) {
    // `list` — same shape as `int`/`str` above. No method is reached ONLY
    // at the class level (unlike `int.from_bytes`) — every instance method
    // (`.append()`, `.sort()`, etc.) keeps resolving via native-backing
    // delegation on `class MyList(list)` instances — so `list_dict` needs
    // just the ctor marker.
    let mut list_dict: HashMap<String, PyObjectRef> = HashMap::new();
    list_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "list".to_string(),
            func: builtin_list,
        }),
    );
    list_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    list_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_list_repr as BuiltinFunc,
        }),
    );
    let list_type = PyObjectRef::new(PyObject::Type {
        name: "list".to_string(),
        dict: Box::new(str_map_to_typedict(list_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *list_type.borrow_mut() {
        *mro = vec![list_type.clone(), object_type.clone()];
    }
    builtins.insert_str("list", list_type.clone());
    crate::object::seed_primitive_type_cache("list", list_type);


    // `dict` — same shape as the others above. Unlike `list`/`str`
    // (nothing class-level-only), `dict` DOES have genuine class-level
    // methods previously reached via `bf_name == "dict" && name == "..."`
    // in `get_attribute_impl` (`attrs.rs`) — those become unreachable once
    // `dict` is a real `Type`, so `fromkeys`/`__setitem__`/`__getitem__`
    // move into `dict_dict` here instead.
    let mut dict_dict: HashMap<String, PyObjectRef> = HashMap::new();
    dict_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "dict".to_string(),
            func: builtin_dict,
        }),
    );
    dict_dict.insert_str(
        "fromkeys",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fromkeys".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("fromkeys() takes at least 1 argument"));
                }
                let keys = crate::object::collect_iterable(&args[0])?;
                let value = args.get(1).cloned().unwrap_or_else(py_none);
                let mut d = PyDict::new();
                for k in keys {
                    d.set(k, value.clone())?;
                }
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
            },
        }),
    );
    dict_dict.insert_str(
        "__setitem__",
        PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "__setitem__".to_string(),
            func: crate::object::builtin_dict_setitem as BuiltinFunc,
            self_obj: py_none(),
        }),
    );
    dict_dict.insert_str(
        "__getitem__",
        PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "__getitem__".to_string(),
            func: crate::object::builtin_dict_getitem as BuiltinFunc,
            self_obj: py_none(),
        }),
    );
    dict_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    dict_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_dict_repr as BuiltinFunc,
        }),
    );
    let dict_type = PyObjectRef::new(PyObject::Type {
        name: "dict".to_string(),
        dict: Box::new(str_map_to_typedict(dict_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *dict_type.borrow_mut() {
        *mro = vec![dict_type.clone(), object_type.clone()];
    }
    builtins.insert_str("dict", dict_type.clone());
    crate::object::seed_primitive_type_cache("dict", dict_type);

    // `tuple` — same shape as `list`/`str` (no class-level-only method).
    // Unlike those two, `tuple` was NOT previously in
    // `is_recognized_native_base_name`/`make_native_backing`/
    // `synthesize_native_init` at all — `class MyTuple(tuple): ...` was
    // silently broken before this migration added it there too (same class
    // of gap found and fixed for `float`; see the memory entry for this
    // migration effort).
    let mut tuple_dict: HashMap<String, PyObjectRef> = HashMap::new();
    tuple_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "tuple".to_string(),
            func: builtin_tuple,
        }),
    );
    tuple_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    tuple_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_tuple_repr as BuiltinFunc,
        }),
    );
    let tuple_type = PyObjectRef::new(PyObject::Type {
        name: "tuple".to_string(),
        dict: Box::new(str_map_to_typedict(tuple_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *tuple_type.borrow_mut() {
        *mro = vec![tuple_type.clone(), object_type.clone()];
    }
    builtins.insert_str("tuple", tuple_type.clone());
    crate::object::seed_primitive_type_cache("tuple", tuple_type);

    // `bytes` — same shape as `list`/`str`/`tuple` (no dunder-shaped
    // class-level method — `fromhex` is a plain classmethod-style
    // function, not a dunder, so it carries none of the ancestor-mro
    // hazards documented for `dict`'s migration).
    let mut bytes_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bytes_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bytes".to_string(),
            func: builtin_bytes,
        }),
    );
    bytes_dict.insert_str(
        "fromhex",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fromhex".to_string(),
            func: builtin_bytes_fromhex,
        }),
    );
    bytes_dict.insert_str(
        "maketrans",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "maketrans".to_string(),
            func: crate::object::bytes_maketrans_builtin,
        }),
    );
    bytes_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    bytes_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bytes_repr as BuiltinFunc,
        }),
    );
    let bytes_type = PyObjectRef::new(PyObject::Type {
        name: "bytes".to_string(),
        dict: Box::new(str_map_to_typedict(bytes_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bytes_type.borrow_mut() {
        *mro = vec![bytes_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bytes", bytes_type.clone());
    crate::object::seed_primitive_type_cache("bytes", bytes_type);

    // `set` — same shape, no class-level-only method at all.
    let mut set_dict: HashMap<String, PyObjectRef> = HashMap::new();
    set_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "set".to_string(),
            func: builtin_set,
        }),
    );
    set_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    set_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_set_repr as BuiltinFunc,
        }),
    );
    let set_type = PyObjectRef::new(PyObject::Type {
        name: "set".to_string(),
        dict: Box::new(str_map_to_typedict(set_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *set_type.borrow_mut() {
        *mro = vec![set_type.clone(), object_type.clone()];
    }
    builtins.insert_str("set", set_type.clone());
    crate::object::seed_primitive_type_cache("set", set_type);

    // `complex` — same shape; `from_number` is a plain classmethod-style
    // function (not a dunder).
    let mut complex_dict: HashMap<String, PyObjectRef> = HashMap::new();
    complex_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "complex".to_string(),
            func: builtin_complex,
        }),
    );
    complex_dict.insert_str(
        "from_number",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "from_number".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "complex.from_number() takes exactly 1 argument",
                    ));
                }
                let n = args[0].as_f64().unwrap_or(0.0);
                Ok(PyObjectRef::imm(PyObject::Complex(n, 0.0)))
            },
        }),
    );
    complex_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    complex_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_complex_repr as BuiltinFunc,
        }),
    );
    let complex_type = PyObjectRef::new(PyObject::Type {
        name: "complex".to_string(),
        dict: Box::new(str_map_to_typedict(complex_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *complex_type.borrow_mut() {
        *mro = vec![complex_type.clone(), object_type.clone()];
    }
    builtins.insert_str("complex", complex_type.clone());
    crate::object::seed_primitive_type_cache("complex", complex_type);

    // `bytearray` — same shape, no class-level-only method.
    let mut bytearray_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bytearray_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bytearray".to_string(),
            func: builtin_bytearray,
        }),
    );
    bytearray_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    bytearray_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bytearray_repr as BuiltinFunc,
        }),
    );
    let bytearray_type = PyObjectRef::new(PyObject::Type {
        name: "bytearray".to_string(),
        dict: Box::new(str_map_to_typedict(bytearray_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bytearray_type.borrow_mut() {
        *mro = vec![bytearray_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bytearray", bytearray_type.clone());
    crate::object::seed_primitive_type_cache("bytearray", bytearray_type);

    // `frozenset` — same shape, no class-level-only method.
    let mut frozenset_dict: HashMap<String, PyObjectRef> = HashMap::new();
    frozenset_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "frozenset".to_string(),
            func: builtin_frozenset,
        }),
    );
    frozenset_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    frozenset_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_frozenset_repr as BuiltinFunc,
        }),
    );
    let frozenset_type = PyObjectRef::new(PyObject::Type {
        name: "frozenset".to_string(),
        dict: Box::new(str_map_to_typedict(frozenset_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *frozenset_type.borrow_mut() {
        *mro = vec![frozenset_type.clone(), object_type.clone()];
    }
    builtins.insert_str("frozenset", frozenset_type.clone());
    crate::object::seed_primitive_type_cache("frozenset", frozenset_type);
}
