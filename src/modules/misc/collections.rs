use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;
#[allow(unused_imports)]
use std::cell::RefCell;

pub fn create_collections_abc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! abc_func {
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

    abc_func!("__import__", |_| Ok(py_bool(true)));

    // Abstract base classes as simple markers
    let abc_meta = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ABCMeta".to_string(),
        func: |_args| {
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_dict(), // simplified type
                dict: AttrMap::new(),
            }))
        },
    });

    d.insert_str("ABCMeta", abc_meta);

    // collections.abc ABCs — real Type objects (not plain strings) so they
    // support subscripting (`Sequence[int]`), which is pervasive in type
    // hints across the ecosystem (PEP 585). __class_getitem__ returns a
    // minimal placeholder "generic alias" Instance rather than a real one —
    // it doesn't track __origin__/__args__ properly, but it does support
    // `__or__` and further `[...]` subscripting so that annotations like
    // `Callable[_P, int] | Callable[_P, str]` (real code seen in asgiref)
    // don't crash — nothing at runtime actually inspects these values.
    fn generic_alias_placeholder(repr: String) -> PyObjectRef {
        let mut type_dict = HashMap::new();
        type_dict.insert_str(
            "__class_getitem__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__class_getitem__".to_string(),
                func: |_args| Ok(generic_alias_placeholder("...".to_string())),
            }),
        );
        type_dict.insert_str(
            "__or__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__or__".to_string(),
                func: |_args| Ok(generic_alias_placeholder("...".to_string())),
            }),
        );
        PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: repr,
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        })
    }

    macro_rules! abc_class {
        ($name:expr) => {
            PyObjectRef::new(PyObject::Type {
                name: $name.to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::from([
                    (
                        "__class_getitem__".to_string(),
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "__class_getitem__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error(
                                        "__class_getitem__ requires 2 args",
                                    ));
                                }
                                Ok(generic_alias_placeholder(format!(
                                    "{}[{}]",
                                    args[0].str(),
                                    args[1].str()
                                )))
                            },
                        }),
                    ),
                    // `isinstance(x, Hashable)` etc. via a method-presence
                    // check, like CPython's __subclasshook__.
                    (
                        "__instancecheck__".to_string(),
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "__instancecheck__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error(
                                        "__instancecheck__ requires 2 args",
                                    ));
                                }
                                let cls_name = match &*args[0].borrow() {
                                    PyObject::Type { name, .. } => name.clone(),
                                    _ => String::new(),
                                };
                                let required: &[&str] = match cls_name.as_str() {
                                    "Hashable" => &["__hash__"],
                                    "Iterable" => &["__iter__"],
                                    "Iterator" => &["__iter__", "__next__"],
                                    "Generator" => &["__iter__", "__next__", "send", "throw", "close"],
                                    "Reversible" => &["__iter__", "__reversed__"],
                                    "Sized" => &["__len__"],
                                    "Container" => &["__contains__"],
                                    "Collection" => &["__iter__", "__len__", "__contains__"],
                                    "Callable" => &[],
                                    "Awaitable" => &["__await__"],
                                    "Coroutine" => &["__await__", "send", "throw", "close"],
                                    "AsyncIterable" => &["__aiter__"],
                                    "AsyncIterator" => &["__aiter__", "__anext__"],
                                    "AsyncGenerator" => &["__aiter__", "__anext__", "asend", "athrow", "aclose"],
                                    "Sequence" => &["__getitem__", "__len__"],
                                    "MutableSequence" => &["__getitem__", "__setitem__", "__delitem__", "__len__", "insert"],
                                    "Set" => &["__contains__", "__iter__", "__len__"],
                                    "MutableSet" => &["__contains__", "__iter__", "__len__", "add", "discard"],
                                    "Mapping" => &["__getitem__", "__iter__", "__len__"],
                                    "MutableMapping" => &["__getitem__", "__setitem__", "__delitem__", "__iter__", "__len__"],
                                    "MappingView" => &["__len__"],
                                    "KeysView" => &["__len__", "__iter__", "__contains__"],
                                    "ItemsView" => &["__len__", "__iter__", "__contains__"],
                                    "ValuesView" => &["__len__", "__iter__"],
                                    "Buffer" => &["__buffer__"],
                                    "ByteString" => &["__getitem__", "__len__"],
                                    "SupportsInt" => &["__int__"],
                                    "SupportsFloat" => &["__float__"],
                                    "SupportsComplex" => &["__complex__"],
                                    "SupportsRound" => &["__round__"],
                                    "SupportsIndex" => &["__index__"],
                                    "SupportsAbs" => &["__abs__"],
                                    "SupportsBytes" => &["__bytes__"],
                                    _ => &[],
                                };
                                // Native fallback for types that should be considered instances even without explicit methods
                                let type_name = args[1].borrow().type_name();
                                let native_sized = matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray" | "range" | "memoryview" | "dict_keys" | "dict_values" | "dict_items" | "KeysView" | "ItemsView" | "ValuesView" | "MappingView" | "OrderedDict" | "Counter" | "deque");
                                let native_reversible = matches!(type_name.as_str(), "list" | "tuple" | "str" | "bytes" | "range" | "dict" | "dict_keys" | "dict_values" | "dict_items" | "OrderedDict" | "Counter" | "deque" | "bytearray" | "memoryview");
                                let native_iterable = matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray" | "range" | "memoryview" | "dict_keys" | "dict_values" | "dict_items" | "KeysView" | "ItemsView" | "ValuesView" | "MappingView" | "OrderedDict" | "Counter" | "deque" | "list_iterator" | "tuple_iterator" | "dict_keyiterator" | "dict_valueiterator" | "dict_itemiterator" | "set_iterator" | "str_iterator" | "bytes_iterator" | "bytearray_iterator" | "range_iterator" | "zip_iterator" | "generator");
                                let native_collection = matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray" | "range" | "memoryview" | "dict_keys" | "dict_values" | "dict_items" | "OrderedDict" | "Counter");
                                let native_container = matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray" | "dict_keys" | "dict_values" | "dict_items");
                                if cls_name == "Sized" && native_sized {
                                    return Ok(py_bool(true));
                                }
                                if cls_name == "Reversible" && native_reversible {
                                    return Ok(py_bool(true));
                                }
                                if cls_name == "Iterable" && native_iterable {
                                    return Ok(py_bool(true));
                                }
                                if cls_name == "Collection" && native_collection {
                                    return Ok(py_bool(true));
                                }
                                if cls_name == "Container" && native_container {
                                    return Ok(py_bool(true));
                                }
                                let native_iterator = matches!(type_name.as_str(), "list_iterator" | "tuple_iterator" | "dict_keyiterator" | "dict_valueiterator" | "dict_itemiterator" | "set_iterator" | "str_iterator" | "bytes_iterator" | "bytearray_iterator" | "range_iterator" | "zip_iterator" | "generator" | "list_reverseiterator");
                                if cls_name == "Iterator" && native_iterator {
                                    return Ok(py_bool(true));
                                }
                                if cls_name == "Hashable" && matches!(type_name.as_str(), "int" | "str" | "tuple" | "frozenset" | "bytes" | "float" | "bool") {
                                    return Ok(py_bool(true));
                                }
                                if cls_name == "Hashable" && matches!(type_name.as_str(), "list" | "dict" | "set" | "bytearray") {
                                    return Ok(py_bool(false));
                                }
                                if required.is_empty() {
                                    if cls_name == "Callable" {
                                        return Ok(crate::object::builtin_callable(&[args[1].clone()])?);
                                    }
                                    let type_name = args[1].borrow().type_name();
                                    let is_native = match cls_name.as_str() {
                                        "Sequence" => matches!(type_name.as_str(), "list" | "tuple" | "str" | "bytes" | "range" | "memoryview"),
                                        "MutableSequence" => matches!(type_name.as_str(), "list" | "bytearray" | "deque"),
                                        "Set" => matches!(type_name.as_str(), "set" | "frozenset"),
                                        "MutableSet" => type_name == "set",
                                        "Mapping" => matches!(type_name.as_str(), "dict" | "OrderedDict" | "Counter"),
                                        "MutableMapping" => matches!(type_name.as_str(), "dict" | "OrderedDict" | "Counter"),
                                        "Iterable" => matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray" | "range" | "memoryview"),
                                        _ => false,
                                    };
                                    if is_native {
                                        return Ok(py_bool(true));
                                    }
                                    return Ok(py_bool(false));
                                }
                                for m in required {
                                    let has = match args[1].borrow().get_attribute(m) {
                                        Ok(v) => !matches!(&*v.borrow(), PyObject::None),
                                        Err(_) => false,
                                    };
                                    if !has {
                                        let typ = crate::object::builtin_type_of(&[args[1].clone()])?;
                                        let via_mro = crate::object::lookup_dunder_via_mro(&typ, m).is_some_and(|f| !matches!(&*f.borrow(), PyObject::None));
                                        if !via_mro {
                                            let type_name = args[1].borrow().type_name();
                                            let native_ok = match (cls_name.as_str(), *m) {
                                                ("Iterable", "__iter__") => matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray" | "range" | "memoryview" | "list_iterator" | "tuple_iterator" | "dict_keyiterator" | "dict_valueiterator" | "dict_itemiterator" | "set_iterator" | "str_iterator" | "bytes_iterator" | "bytearray_iterator" | "range_iterator" | "zip_iterator" | "generator"),
                                                ("Iterator", "__next__") => matches!(type_name.as_str(), "list_iterator" | "tuple_iterator" | "set_iterator" | "str_iterator" | "bytes_iterator" | "bytearray_iterator" | "dict_keyiterator" | "dict_valueiterator" | "dict_itemiterator" | "range_iterator" | "zip_iterator" | "generator"),
                                                ("Sized", "__len__") => matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray" | "range" | "memoryview"),
                                                ("Container", "__contains__") => matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray"),
                                                ("Collection", "__contains__") => matches!(type_name.as_str(), "list" | "tuple" | "dict" | "set" | "frozenset" | "str" | "bytes" | "bytearray"),
                                                _ => false,
                                            };
                                            if !native_ok {
                                                return Ok(py_bool(false));
                                            }
                                        }
                                    }
                                }
                                let type_name = args[1].borrow().type_name();
                                if cls_name == "MutableSet" && type_name == "frozenset" {
                                    return Ok(py_bool(false));
                                }
                                if cls_name == "MutableSequence" && matches!(type_name.as_str(), "tuple" | "str" | "bytes") {
                                    return Ok(py_bool(false));
                                }
                                Ok(py_bool(true))
                            },
                        }),
                    ),
                ]))),
                bases: vec![],
                mro: vec![],
            })
        };
    }

    d.insert_str("Hashable", abc_class!("Hashable"));
    d.insert_str("Iterable", abc_class!("Iterable"));
    d.insert_str("Iterator", abc_class!("Iterator"));
    d.insert_str("Generator", abc_class!("Generator"));
    d.insert_str("Reversible", abc_class!("Reversible"));
    d.insert_str("Sized", abc_class!("Sized"));
    d.insert_str("Container", abc_class!("Container"));
    d.insert_str("Callable", abc_class!("Callable"));
    d.insert_str("Collection", abc_class!("Collection"));
    d.insert_str("Sequence", abc_class!("Sequence"));
    d.insert_str("MutableSequence", abc_class!("MutableSequence"));
    d.insert_str("Set", abc_class!("Set"));
    d.insert_str("MutableSet", abc_class!("MutableSet"));
    d.insert_str("Mapping", abc_class!("Mapping"));
    d.insert_str("MutableMapping", abc_class!("MutableMapping"));
    d.insert_str("MappingView", abc_class!("MappingView"));
    d.insert_str("KeysView", abc_class!("KeysView"));
    d.insert_str("ItemsView", abc_class!("ItemsView"));
    d.insert_str("ValuesView", abc_class!("ValuesView"));
    d.insert_str("Awaitable", abc_class!("Awaitable"));
    d.insert_str("Coroutine", abc_class!("Coroutine"));
    d.insert_str("AsyncIterable", abc_class!("AsyncIterable"));
    d.insert_str("AsyncIterator", abc_class!("AsyncIterator"));
    d.insert_str("AsyncGenerator", abc_class!("AsyncGenerator"));
    d.insert_str("Buffer", abc_class!("Buffer"));
    d.insert_str("ByteString", abc_class!("ByteString"));
    // Set __abstractmethods__ for each ABC
    {
        let abstract_map: Vec<(&str, Vec<&str>)> = vec![
            ("Hashable", vec!["__hash__"]),
            ("Awaitable", vec!["__await__"]),
            ("Coroutine", vec!["__await__", "send", "throw", "close"]),
            ("AsyncIterable", vec!["__aiter__"]),
            ("AsyncIterator", vec!["__aiter__", "__anext__"]),
            ("AsyncGenerator", vec!["__aiter__", "__anext__", "asend", "athrow", "aclose"]),
            ("Iterable", vec!["__iter__"]),
            ("Iterator", vec!["__iter__", "__next__"]),
            ("Generator", vec!["__iter__", "__next__", "send", "throw", "close"]),
            ("Reversible", vec!["__iter__", "__reversed__"]),
            ("Sized", vec!["__len__"]),
            ("Container", vec!["__contains__"]),
            ("Collection", vec!["__iter__", "__len__", "__contains__"]),
            ("Callable", vec!["__call__"]),
            ("Set", vec!["__contains__", "__iter__", "__len__"]),
            ("MutableSet", vec!["__contains__", "__iter__", "__len__", "add", "discard"]),
            ("Mapping", vec!["__getitem__", "__iter__", "__len__"]),
            ("MutableMapping", vec!["__getitem__", "__setitem__", "__delitem__", "__iter__", "__len__"]),
            ("MappingView", vec!["__len__"]),
            ("KeysView", vec!["__len__", "__iter__", "__contains__"]),
            ("ItemsView", vec!["__len__", "__iter__", "__contains__"]),
            ("ValuesView", vec!["__len__", "__iter__"]),
            ("Sequence", vec!["__getitem__", "__len__"]),
            ("MutableSequence", vec!["__getitem__", "__setitem__", "__delitem__", "__len__", "insert"]),
            ("Buffer", vec!["__buffer__"]),
            ("ByteString", vec!["__getitem__", "__len__"]),
        ];
        for (name, methods) in abstract_map {
            if let Some(abc) = d.get(name) {
                if let PyObject::Type { dict, .. } = &mut *abc.borrow_mut() {
                    let mut s = crate::object::PySet::new();
                    for m in methods {
                        let _ = s.add(py_str(m));
                    }
                    dict.insert_str("__abstractmethods__", PyObjectRef::new(PyObject::FrozenSet(s)));
                    dict.insert_str("_abc_registry", PyObjectRef::new(PyObject::FrozenSet(crate::object::PySet::new())));
                }
            }
        }
    }
    // CPython's Mapping/MutableMapping set `__reversed__ = None` — the
    // documented way to explicitly DISABLE reversal on a len/getitem class
    // (`reversed(MyMapping())` raises TypeError).
    if let Some(m) = d.get("Mapping") {
        if let PyObject::Type { dict, .. } = &mut *m.borrow_mut() {
            dict.insert_str("__reversed__", py_none());
            // Add items/keys/values methods — ThemeSection uses items()
            // which inherits from Mapping, but the native Mapping ABC
            // doesn't define these Python-level methods yet.
            dict.insert_str(
                "items",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "items".to_string(),
                    func: |args| {
                        if args.is_empty() {
                            return Err(PyError::type_error("items() missing self"));
                        }
                        let obj = &args[0];
                        // Use call_function to invoke __iter__ and __getitem__
                        // via the normal calling path to avoid RefCell borrow conflicts
                        let iter_fn = {
                            let obj_borrowed = obj.borrow();
                            obj_borrowed.get_attribute("__iter__")?
                        };
                        let iter_obj =
                            crate::object::call_bound_method(iter_fn, obj.clone(), vec![])?;
                        let mut keys_list = Vec::new();
                        loop {
                            let key = match crate::object::builtin_next(&[iter_obj.clone()]) {
                                Ok(v) => v,
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            };
                            keys_list.push(key);
                        }
                        let getitem_fn = {
                            let obj_borrowed = obj.borrow();
                            obj_borrowed.get_attribute("__getitem__")?
                        };
                        let mut result = Vec::new();
                        for k in keys_list {
                            let v = crate::object::call_bound_method(
                                getitem_fn.clone(),
                                obj.clone(),
                                vec![k.clone()],
                            )?;
                            result.push(py_tuple(vec![k, v]));
                        }
                        Ok(py_list(result))
                    },
                }),
            );
        }
    }
    if let Some(m) = d.get("MutableMapping") {
        if let PyObject::Type { dict, .. } = &mut *m.borrow_mut() {
            dict.insert_str("__reversed__", py_none());
            dict.insert_str(
                "update",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "update".to_string(),
                    func: |args| {
                        if args.is_empty() {
                            return Err(PyError::type_error("update() missing self"));
                        }
                        let self_obj = &args[0];
                        let other = if args.len() > 1 {
                            args[1].clone()
                        } else {
                            py_none()
                        };

                        if !matches!(&*other.borrow(), PyObject::None) {
                            // Determine if `other` is a mapping-like (dict or
                            // dict subclass) or an iterable of (key, value) pairs.
                            let is_dict_based = {
                                let other_borrow = other.borrow();
                                match &*other_borrow {
                                    PyObject::Dict(_) => true,
                                    PyObject::Instance { typ, .. } => {
                                        let typ_borrow = typ.borrow();
                                        if let PyObject::Type { mro, .. } = &*typ_borrow {
                                            mro.iter().any(|base| {
                                                if let PyObject::Type { name, .. } = &*base.borrow()
                                                {
                                                    name == "dict"
                                                } else {
                                                    false
                                                }
                                            })
                                        } else {
                                            false
                                        }
                                    }
                                    _ => false,
                                }
                            };

                            if is_dict_based {
                                // Dict or dict subclass: for k in other: self[k] = other[k]
                                let iter_obj = crate::object::builtin_iter(&[other.clone()])?;
                                loop {
                                    let key = match crate::object::builtin_next(&[iter_obj.clone()])
                                    {
                                        Ok(v) => v,
                                        Err(e) if crate::object::is_stop_iteration_error(&e) => {
                                            break
                                        }
                                        Err(e) => return Err(e),
                                    };
                                    let value = crate::object::py_getitem(&other, &key)?;
                                    crate::object::py_setitem(self_obj, &key, value)?;
                                }
                            } else {
                                // Iterable of (key, value) pairs
                                let iter_obj = crate::object::builtin_iter(&[other.clone()])?;
                                loop {
                                    let pair =
                                        match crate::object::builtin_next(&[iter_obj.clone()]) {
                                            Ok(v) => v,
                                            Err(e)
                                                if crate::object::is_stop_iteration_error(&e) =>
                                            {
                                                break
                                            }
                                            Err(e) => return Err(e),
                                        };
                                    let key = crate::object::py_getitem(&pair, &py_int(0))?;
                                    let value = crate::object::py_getitem(&pair, &py_int(1))?;
                                    crate::object::py_setitem(self_obj, &key, value)?;
                                }
                            }
                        }
                        // Process keyword arguments
                        let remaining = &args[2..];
                        for chunk in remaining.chunks(2) {
                            if chunk.len() == 2 {
                                crate::object::py_setitem(self_obj, &chunk[0], chunk[1].clone())?;
                            }
                        }
                        Ok(py_none())
                    },
                }),
            );
        }
    }
    d.insert_str("MappingView", abc_class!("MappingView"));
    d.insert_str("ItemsView", abc_class!("ItemsView"));
    d.insert_str("KeysView", abc_class!("KeysView"));
    d.insert_str("ValuesView", abc_class!("ValuesView"));
    d.insert_str("Container", abc_class!("Container"));
    d.insert_str("Awaitable", abc_class!("Awaitable"));
    d.insert_str("Coroutine", abc_class!("Coroutine"));
    d.insert_str("AsyncIterable", abc_class!("AsyncIterable"));
    d.insert_str("AsyncIterator", abc_class!("AsyncIterator"));
    d.insert_str("AsyncGenerator", abc_class!("AsyncGenerator"));
    d.insert_str("Generator", abc_class!("Generator"));
    d.insert_str("Reversible", abc_class!("Reversible"));
    d.insert_str("Collection", abc_class!("Collection"));
    d.insert_str("ByteString", abc_class!("ByteString"));
    d.insert_str("Buffer", abc_class!("Buffer"));
    // Aliases CPython exposes on collections.abc (point to builtin types).
    d.insert_str("dict_items", abc_class!("dict_items"));
    d.insert_str("dict_keys", abc_class!("dict_keys"));
    d.insert_str("dict_values", abc_class!("dict_values"));
    d.insert_str("dict_itemiterator", abc_class!("dict_itemiterator"));
    d.insert_str("dict_keyiterator", abc_class!("dict_keyiterator"));
    d.insert_str("dict_valueiterator", abc_class!("dict_valueiterator"));
    d.insert_str("generator", abc_class!("generator"));
    d.insert_str("coroutine", abc_class!("coroutine"));
    d.insert_str("async_generator", abc_class!("async_generator"));
    d.insert_str("list_iterator", abc_class!("list_iterator"));
    d.insert_str("list_reverseiterator", abc_class!("list_reverseiterator"));
    d.insert_str("tuple_iterator", abc_class!("tuple_iterator"));
    d.insert_str("set_iterator", abc_class!("set_iterator"));
    d.insert_str("str_iterator", abc_class!("str_iterator"));
    d.insert_str("range_iterator", abc_class!("range_iterator"));
    d.insert_str("longrange_iterator", abc_class!("longrange_iterator"));
    d.insert_str("zip_iterator", abc_class!("zip_iterator"));
    d.insert_str("bytes_iterator", abc_class!("bytes_iterator"));
    d.insert_str("bytearray_iterator", abc_class!("bytearray_iterator"));
    d.insert_str("mappingproxy", abc_class!("mappingproxy"));
    d.insert_str("framelocalsproxy", abc_class!("framelocalsproxy"));

    d
}
