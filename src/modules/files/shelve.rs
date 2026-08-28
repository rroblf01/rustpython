use crate::object::*;
use std::collections::HashMap;

// Moved here from object.rs (was under a "=== SHELVE MODULE ===" banner in
// the monolithic object.rs — see the file-splitting refactor's memory
// entry for context).
// Shelf class backed by a dict. open(filename) -> Shelf instance.

/// Extract the _data dict from a Shelf Instance (args[0]).
fn shelf_get_data(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("method requires self"));
    }
    match &*args[0].borrow() {
        PyObject::Instance { dict, .. } => match dict.get("_data") {
            Some(data) => Ok(data.clone()),
            None => Err(PyError::runtime_error(
                "Shelf instance corrupted: missing _data",
            )),
        },
        _ => Err(PyError::type_error("expected Shelf instance")),
    }
}

fn shelf_close(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _ = args;
    Ok(py_none())
}

fn shelf_sync(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _ = args;
    Ok(py_none())
}

fn shelf_get(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // args[0] = self, args[1] = key, args[2] = default (optional)
    if args.len() < 2 {
        return Err(PyError::type_error(
            "get() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        match d.get(&py_key)? {
            Some(val) => Ok(val),
            None => {
                if args.len() > 2 {
                    Ok(args[2].clone())
                } else {
                    Ok(py_none())
                }
            }
        }
    } else {
        Ok(py_none())
    }
}

fn shelf_keys(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let ks = d.keys();
        Ok(PyObjectRef::new(PyObject::List(ks)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

fn shelf_values(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let vs = d.values();
        Ok(PyObjectRef::new(PyObject::List(vs)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

fn shelf_items(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let pairs: Vec<PyObjectRef> = d
            .items()
            .into_iter()
            .map(|(k, v)| PyObjectRef::new(PyObject::Tuple(vec![k, v])))
            .collect();
        Ok(PyObjectRef::new(PyObject::List(pairs)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

// __len__(self) -> int (for len())
fn shelf_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        Ok(py_int(d.len() as i64))
    } else {
        Ok(py_int(0))
    }
}

// __contains__(self, key) -> bool (for 'key in shelf')
fn shelf_contains(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__contains__() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        Ok(py_bool(d.contains(&py_key)?))
    } else {
        Ok(py_bool(false))
    }
}

// __repr__(self) -> str
fn shelf_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        Ok(py_str(&format!("Shelf({} items)", d.len())))
    } else {
        Ok(py_str("Shelf(0 items)"))
    }
}

// __getitem__(self, key) -> value (for shelf[key])
fn shelf_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__getitem__() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        match d.get(&py_key)? {
            Some(val) => Ok(val),
            None => Err(PyError::key_error(format!("'{}'", key))),
        }
    } else {
        Err(PyError::key_error(format!("'{}'", key)))
    }
}

// __setitem__(self, key, value) (for shelf[key] = value)
fn shelf_setitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error(
            "__setitem__() takes at least 3 arguments (self, key, value)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    {
        let mut data_mut = data.borrow_mut();
        if let PyObject::Dict(ref mut d) = &mut *data_mut {
            d.set(py_str(&key), args[2].clone())?;
        }
    }
    Ok(py_none())
}

// __delitem__(self, key) (for del shelf[key])
fn shelf_delitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__delitem__() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    {
        let mut data_mut = data.borrow_mut();
        if let PyObject::Dict(ref mut d) = &mut *data_mut {
            let py_key = py_str(&key);
            d.remove(&py_key)?;
        }
    }
    Ok(py_none())
}

pub fn shelf_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "open() takes at least 1 argument (filename)",
        ));
    }
    let filename = args[0].str();

    // Internal data dict
    let data_dict = py_dict();

    // Instance dict with field and methods
    let mut inst_dict = AttrMap::new();
    inst_dict.insert("_data".to_string(), data_dict);
    inst_dict.insert("filename".to_string(), py_str(&filename));

    inst_dict.insert(
        "close".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: shelf_close,
        }),
    );
    inst_dict.insert(
        "sync".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "sync".to_string(),
            func: shelf_sync,
        }),
    );
    inst_dict.insert(
        "get".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "get".to_string(),
            func: shelf_get,
        }),
    );
    inst_dict.insert(
        "keys".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "keys".to_string(),
            func: shelf_keys,
        }),
    );
    inst_dict.insert(
        "values".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "values".to_string(),
            func: shelf_values,
        }),
    );
    inst_dict.insert(
        "items".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "items".to_string(),
            func: shelf_items,
        }),
    );

    // Type dict with dunder methods (used by py_getitem/py_setitem dispatch)
    let mut type_dict = HashMap::new();
    type_dict.insert(
        "__getitem__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getitem__".to_string(),
            func: shelf_getitem,
        }),
    );
    type_dict.insert(
        "__setitem__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__setitem__".to_string(),
            func: shelf_setitem,
        }),
    );
    type_dict.insert(
        "__delitem__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__delitem__".to_string(),
            func: shelf_delitem,
        }),
    );
    type_dict.insert(
        "__len__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__len__".to_string(),
            func: shelf_len,
        }),
    );
    type_dict.insert(
        "__contains__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__contains__".to_string(),
            func: shelf_contains,
        }),
    );
    type_dict.insert(
        "__repr__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: shelf_repr,
        }),
    );

    // Build Shelf type
    let shelf_type = PyObjectRef::new(PyObject::Type {
        name: "Shelf".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        // MRO includes self so __getitem__ lookup works
        mro: vec![],
    });

    let instance = PyObjectRef::new(PyObject::Instance {
        typ: shelf_type,
        dict: inst_dict,
    });

    Ok(instance)
}

pub fn create_shelve_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "open",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "open".to_string(),
            func: shelf_open,
        }),
    );
    d.insert_str("Shelf", py_str("Shelf"));
    d
}
