use crate::object::*;
use std::collections::HashMap;
use num_traits::ToPrimitive;

pub fn register_primitive_types(builtins: &mut HashMap<String, PyObjectRef>, object_type: &PyObjectRef) {
    // `int` — the first of the "native value" types (int/str/list/dict/...)
    // migrated from a plain `PyObject::BuiltinFunction` constructor to a
    // real, subclassable `PyObject::Type`, closing the long-standing
    // "native types aren't real Type objects" gap for this one type (see
    // `NATIVE_VALUE_CTOR_KEY`'s doc comment in object.rs for the full
    // mechanism). `int_dict`'s `NATIVE_VALUE_CTOR_KEY` entry points at the
    // ORIGINAL native constructor closure (`builtin_int`) — `call_function`
    // dispatches through it and returns the raw, unwrapped `PyObject::Int`
    // result, so `int(5)` still produces a plain int, never an
    // instance-of-int wrapper. `from_bytes` is the one method that
    // genuinely needs to live in this dict now (every other int method
    // like `bit_length` keeps resolving via native-backing delegation on a
    // `class MyInt(int)` instance, unaffected by this change).
    let mut int_dict: HashMap<String, PyObjectRef> = HashMap::new();
    int_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "int".to_string(),
            func: builtin_int,
        }),
    );
    int_dict.insert_str(
        "from_bytes",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "from_bytes".to_string(),
            func: crate::object::builtin_int_from_bytes,
        }),
    );
    int_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    int_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_int_repr as BuiltinFunc,
        }),
    );
    // int.__format__(self, format_spec): format integer with format spec mini-language
    int_dict.insert_str(
        "__format__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__format__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__format__ requires at least 1 argument (self)",
                    ));
                }
                let spec = if args.len() > 1 {
                    args[1].str()
                } else {
                    String::new()
                };
                if spec.is_empty() {
                    Ok(py_str(&args[0].repr()))
                } else {
                    crate::vm::format_with_spec(&args[0], &spec).map(|s| py_str(&s))
                }
            },
        }),
    );
    // int.__itemsize__ — Python arbitrary-precision ints have no fixed size
    int_dict.insert_str(
        "__itemsize__",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__itemsize__".to_string(),
                func: |_args| Ok(py_int(0)),
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    let int_type = PyObjectRef::new(PyObject::Type {
        name: "int".to_string(),
        dict: Box::new(str_map_to_typedict(int_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *int_type.borrow_mut() {
        *mro = vec![int_type.clone(), object_type.clone()];
    }
    builtins.insert_str("int", int_type.clone());
    crate::object::seed_primitive_type_cache("int", int_type.clone());


    // `str` — the second type migrated to the `NATIVE_VALUE_CTOR_KEY`
    // mechanism, same shape as `int` above. Unlike `int` (which needed
    // `from_bytes` as a genuine class-level method), `str` has no method
    // that's reached ONLY at the class level — every instance method
    // (`.upper()`, `.join()`, etc.) keeps resolving via native-backing
    // delegation on `class MyStr(str)` instances, unaffected by this change
    // — so `str_dict` needs just the ctor marker.
    let mut str_dict: HashMap<String, PyObjectRef> = HashMap::new();
    str_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "str".to_string(),
            func: builtin_str,
        }),
    );
    str_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    str_dict.insert_str(
        "maketrans",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "maketrans".to_string(),
            func: crate::object::str_maketrans_builtin,
        }),
    );
    // `str.casefold` — the instance-level method resolves via the Str
    // get_attribute arm, but unbound CLASS-level access (`str.casefold`,
    // real CPython's own `test_bisect.py` uses it as a key function) failed
    // with AttributeError because the type dict only had the ctor marker.
    str_dict.insert_str(
        "casefold",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "casefold".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error("casefold() missing required argument"));
                }
                Ok(py_str(&args[0].str().to_lowercase()))
            },
        }),
    );
    // Unbound CLASS-level access to the common str methods (`str.strip`,
    // `map(str.strip, ...)` — the standard idiom test_format_testfile uses).
    // Each delegates to the same instance method (resolved via get_attribute
    // on the first arg). The macro's per-name literal makes each closure
    // non-capturing (coerces to BuiltinFunc).
    macro_rules! str_unbound {
        ($name:literal) => {
            str_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: |args: &[PyObjectRef]| {
                        if args.is_empty() {
                            return Err(PyError::type_error(concat!(
                                $name,
                                "() missing required argument: 'self'"
                            )));
                        }
                        // The instance method's BuiltinMethod carries a PLACEHOLDER
                        // self_obj (None) — the receiver is bound by the CALL
                        // machinery on `s.strip()`. Call the underlying func
                        // directly with the receiver as the first arg.
                        // Resolve the real method from the NATIVE BACKING when the
                        // receiver is a native-base subclass instance (`class
                        // Foo(str)`) — resolving through the instance's MRO
                        // returns THIS very same str_unbound closure, recursing
                        // forever (stack overflow on `Foo('hi').upper()`).
                        let method = match crate::object::native_backing_of(&args[0]) {
                            Some(native) => native.borrow().get_attribute($name)?,
                            None => args[0].borrow().get_attribute($name)?,
                        };
                        let is_builtin_method = {
                            let m = method.borrow();
                            matches!(&*m, PyObject::BuiltinMethod { .. })
                        };
                        if is_builtin_method {
                            let m = method.borrow();
                            if let PyObject::BuiltinMethod { func, .. } = &*m {
                                let mut all = vec![args[0].clone()];
                                all.extend_from_slice(&args[1..]);
                                return func(&all);
                            }
                        }
                        // Non-BuiltinMethod (e.g. an override from a subclass's
                        // dict) — call it with the receiver prepended.
                        let mut all = vec![args[0].clone()];
                        all.extend_from_slice(&args[1..]);
                        crate::object::call_function_disposable(&method, all, vec![])
                    },
                }),
            );
        };
    }
    str_unbound!("strip");
    str_unbound!("rstrip");
    str_unbound!("lstrip");
    str_unbound!("split");
    str_unbound!("rsplit");
    str_unbound!("upper");
    str_unbound!("lower");
    str_unbound!("replace");
    str_unbound!("format");
    str_unbound!("join");
    str_unbound!("startswith");
    str_unbound!("endswith");
    str_unbound!("find");
    str_unbound!("rfind");
    str_unbound!("index");
    str_unbound!("rindex");
    str_unbound!("count");
    str_unbound!("splitlines");
    str_unbound!("capitalize");
    str_unbound!("title");
    str_unbound!("swapcase");
    str_unbound!("zfill");
    str_unbound!("encode");
    str_unbound!("decode");
    str_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_str_repr as BuiltinFunc,
        }),
    );
    let str_type = PyObjectRef::new(PyObject::Type {
        name: "str".to_string(),
        dict: Box::new(str_map_to_typedict(str_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *str_type.borrow_mut() {
        *mro = vec![str_type.clone(), object_type.clone()];
    }
    builtins.insert_str("str", str_type.clone());
    crate::object::seed_primitive_type_cache("str", str_type);

    // `float` — same shape as `int`/`str`/`list` above, but unlike those
    // three, `float` DOES have genuine class-level-only methods (previously
    // reached via a `bf_name == "float" && name == "..."` special case in
    // `get_attribute_impl`, `attrs.rs` — that dispatch only ever fired when
    // the object being accessed WAS the bare `BuiltinFunction` itself, i.e.
    // `float.fromhex`/`float.hex`/`float.__getformat__`/`float.from_number`
    // called unbound; a plain float VALUE's own `.hex()`/`.is_integer()`
    // etc. go through a wholly separate `PyObject::Float(_)` instance arm,
    // unaffected by this migration) — those become unreachable once `float`
    // is a real `Type` (the generic `BuiltinFunction`-name dispatch never
    // fires for it again), so they must move into `float_dict` here instead.
    let mut float_dict: HashMap<String, PyObjectRef> = HashMap::new();
    float_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "float".to_string(),
            func: builtin_float,
        }),
    );
    float_dict.insert_str(
        "__getformat__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getformat__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__getformat__() takes exactly one argument",
                    ));
                }
                if !matches!(&*args[0].borrow(), PyObject::Str(_)) {
                    return Err(PyError::type_error("__getformat__() argument must be str"));
                }
                match args[0].str().as_str() {
                    "double" | "float" => Ok(py_str("IEEE, little-endian")),
                    other => Err(PyError::value_error(format!(
                        "__getformat__() argument must be 'double' or 'float', not '{}'",
                        other
                    ))),
                }
            },
        }),
    );
    float_dict.insert_str(
        "fromhex",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "fromhex".to_string(),
                func: crate::object::float_fromhex,
            }),
        }),
    );
    float_dict.insert_str(
        "hex",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "hex".to_string(),
            func: crate::object::float_class_hex,
        }),
    );
    float_dict.insert_str(
        "from_number",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_number".to_string(),
                func: |args| {
                    // Bound as a classmethod: args[0] is the calling type, args[1]
                    // the number.
                    if args.len() < 2 {
                        return Err(PyError::type_error(
                            "float.from_number() takes exactly 1 argument",
                        ));
                    }
                    let cls = &args[0];
                    let number = &args[1];
                    let extract = |number: &PyObjectRef| -> PyResult<Option<f64>> {
                        let b = number.borrow();
                        match &*b {
                            PyObject::Float(f) => Ok(Some(*f)),
                            PyObject::Int(i) => Ok(Some(crate::object::bigint_to_float(i)?)),
                            PyObject::Bool(b2) => Ok(Some(if *b2 { 1.0 } else { 0.0 })),
                            PyObject::Complex(..)
                            | PyObject::Str(_)
                            | PyObject::Bytes(_)
                            | PyObject::ByteArray(_) => Ok(None),
                            PyObject::Instance { .. } => {
                                drop(b);
                                // A transparent float-subclass instance's native
                                // backing is its value.
                                if let Some(native) = crate::object::native_backing_of(number) {
                                    if let Some(f) = native.as_f64() {
                                        return Ok(Some(f));
                                    }
                                }
                                let typ = match &*number.borrow() {
                                    PyObject::Instance { typ, .. } => typ.clone(),
                                    _ => unreachable!(),
                                };
                                // __float__, then __index__ (CPython's PyFloat_AsDouble).
                                if let Some(f) =
                                    crate::object::lookup_dunder_via_mro(&typ, "__float__")
                                {
                                    let result = crate::object::call_bound_method(
                                        f,
                                        number.clone(),
                                        vec![],
                                    )?;
                                    return Ok(Some(result.as_f64().ok_or_else(|| {
                                        PyError::type_error("__float__ returned non-float")
                                    })?));
                                }
                                if let Some(f) =
                                    crate::object::lookup_dunder_via_mro(&typ, "__index__")
                                {
                                    let result = crate::object::call_bound_method(
                                        f,
                                        number.clone(),
                                        vec![],
                                    )?;
                                    let v = result.borrow();
                                    if let PyObject::Int(i) = &*v {
                                        return Ok(Some(i.to_f64().unwrap_or(0.0)));
                                    }
                                    return Err(PyError::type_error("__index__ returned non-int"));
                                }
                                Ok(None)
                            }
                            _ => Ok(None),
                        }
                    };
                    let value = extract(number)?.ok_or_else(|| {
                        PyError::type_error(format!(
                            "float.from_number() argument must be a number, not '{}'",
                            number.borrow().type_name()
                        ))
                    })?;
                    let is_plain_float =
                        matches!(&*cls.borrow(), PyObject::Type { name, .. } if name == "float");
                    if is_plain_float {
                        // Exact-float input returns the SAME object (CPython
                        // identity contract: float.from_number(NAN) is NAN).
                        if matches!(&*number.borrow(), PyObject::Float(_)) {
                            return Ok(number.clone());
                        }
                        return Ok(py_float(value));
                    }
                    // Subclass call: build a float-subclass instance carrying the
                    // value as its native backing (mirrors what `FloatSubclass(3.14)`
                    // produces; done directly here because re-entering the VM via
                    // `with_vm_mut` from inside this already-in-call chain is UB).
                    let mut dict = crate::object::AttrMap::new();
                    dict.insert(
                        crate::object::NATIVE_BACKING_KEY.to_string(),
                        py_float(value),
                    );
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: cls.clone(),
                        dict,
                    }))
                },
            }),
        }),
    );
    float_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    // __hash__: float values hash via CPython's mod-2**61-1 double hash.
    // Present in the type dict so a transparent float SUBCLASS routes here
    // (MRO finds float.__hash__ before any later base's Python-level
    // __hash__), and so a subclass nan hashes by object identity — matching
    // `object.__hash__`'s pointer hash, as CPython 3.13+ requires.
    float_dict.insert_str(
        "__hash__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__hash__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "float.__hash__() takes exactly 1 argument",
                    ));
                }
                // The float VALUE is either the arg itself (SmallFloat/boxed
                // Float) or the native backing of a transparent subclass
                // instance.
                let v = {
                    let b = args[0].borrow();
                    match &*b {
                        PyObject::Float(f) => *f,
                        PyObject::Instance { .. } => {
                            let backing = crate::object::native_backing_of(&args[0]);
                            match backing {
                                Some(bk) => match &*bk.borrow() {
                                    PyObject::Float(f) => *f,
                                    _ => {
                                        return Err(PyError::type_error(
                                            "float.__hash__() argument has no float backing",
                                        ))
                                    }
                                },
                                None => {
                                    return Err(PyError::type_error(
                                        "float.__hash__() argument has no float backing",
                                    ))
                                }
                            }
                        }
                        _ => {
                            return Err(PyError::type_error(
                                "float.__hash__() argument must be float",
                            ))
                        }
                    }
                };
                if v.is_nan() {
                    // CPython hashes NaN by object identity.
                    let ptr: *const PyObject = &*args[0].borrow();
                    return Ok(py_int(ptr as i64));
                }
                Ok(py_int(crate::object::hash_double(v) as i64))
            },
        }),
    );
    float_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_float_repr as BuiltinFunc,
        }),
    );
    let float_type = PyObjectRef::new(PyObject::Type {
        name: "float".to_string(),
        dict: Box::new(str_map_to_typedict(float_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *float_type.borrow_mut() {
        *mro = vec![float_type.clone(), object_type.clone()];
    }
    builtins.insert_str("float", float_type.clone());
    crate::object::seed_primitive_type_cache("float", float_type);

    // `bool` — a real `Type` (fixing `type(True) is bool`) but
    // DELIBERATELY excluded from `is_recognized_native_base_name` and given
    // NO entry in `make_native_backing`/`synthesize_native_init`: real
    // Python disallows subclassing `bool` at all (`TypeError: type 'bool'
    // is not an acceptable base type`) — `default_build_class` (`vm.rs`)
    // checks for this explicitly, by identity against this exact binding,
    // before it would otherwise reach the generic `NATIVE_VALUE_CTOR_KEY`
    // detection arm and wrongly treat `bool` as a valid native base. Bases
    // are `[int_type, object_type]`, matching real CPython's actual
    // hierarchy (`bool.__bases__ == (int,)`) — `__new__` (constructing a
    // bool from a truthiness-tested value) moves in from what used to be a
    // `bf_name == "bool" && name == "__new__"` special case in
    // `get_attribute_impl` (`attrs.rs`).
    let mut bool_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bool_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bool".to_string(),
            func: builtin_bool,
        }),
    );
    bool_dict.insert_str(
        "__new__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__new__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Ok(py_bool(false));
                }
                if args.len() >= 2 {
                    return Ok(py_bool(args[1].truthy()));
                }
                Ok(py_bool(false))
            },
        }),
    );
    // bool.from_bytes returns a REAL bool (False/True), unlike the int 0/1
    // int.from_bytes produces (test_bool's test_from_bytes asserts
    // `is False`/`is True`). Must be in bool's own dict — via int's MRO it
    // would resolve to the same object as int.from_bytes.
    bool_dict.insert_str(
        "from_bytes",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "from_bytes".to_string(),
            func: |args: &[PyObjectRef]| {
                let v = crate::object::builtin_int_from_bytes(args)?;
                Ok(py_bool(v.truthy()))
            },
        }),
    );
    bool_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bool_repr as BuiltinFunc,
        }),
    );
    let bool_type = PyObjectRef::new(PyObject::Type {
        name: "bool".to_string(),
        dict: Box::new(str_map_to_typedict(bool_dict)),
        bases: vec![int_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bool_type.borrow_mut() {
        *mro = vec![bool_type.clone(), int_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bool", bool_type.clone());
    crate::object::seed_primitive_type_cache("bool", bool_type);

    // `type` — a real, subclassable Type object (not just the `type(x)`
    // introspection/`type(name,bases,ns)` construction BuiltinFunction that
    // used to be the sole binding for this name), so `class MyMeta(type):
    // ...` works as a genuine metaclass with real MRO-based method
    // resolution (needed for e.g. enum's EnumType). `type.__new__` is what
    // a custom metaclass's `super().__new__(metacls, name, bases, ns,
    // **kwds)` bottoms out on. `type(x)` / `type(name, bases, ns)` calling
    // `type` itself still needs its own dual-arity behavior — that's kept
    // in `builtin_type_of` (object.rs) and special-cased at the top of
    // `call_function` (vm.rs) by identity against this exact object,
    // before the generic "call a Type to instantiate it" path would
    // otherwise try to build a plain Instance instead.
    let mut type_dict = HashMap::new();
    type_dict.insert_str(
        "__new__",
        PyObjectRef::new(PyObject::StaticMethod {
            func: PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__new__".to_string(),
                func: crate::object::type_new_builtin,
            }),
        }),
    );
    let type_type = PyObjectRef::new(PyObject::Type {
        name: "type".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *type_type.borrow_mut() {
        *mro = vec![type_type.clone(), object_type.clone()];
    }
    builtins.insert_str("type", type_type);
    builtins.insert_str(
        "_type_func",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "type".to_string(),
            func: builtin_type_of,
        }),
    );
}
