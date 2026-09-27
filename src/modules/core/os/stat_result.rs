use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;

/// The ten INDEXABLE fields of a `stat_result`, in CPython's order. The
/// remaining attributes CPython exposes (`st_atime_ns`, `st_mtime_ns`,
/// `st_ctime_ns`, `st_birthtime`, …) are name-only and live in the
/// instance dict like any other attribute.
const SEQ_FIELDS: [&str; 10] = [
    "st_mode", "st_ino", "st_dev", "st_nlink", "st_uid", "st_gid", "st_size", "st_atime",
    "st_mtime", "st_ctime",
];

thread_local! {
    static STAT_RESULT_TYPE: Rc<PyObjectRef> = {
        let mut dict: HashMap<String, PyObjectRef> = HashMap::new();
        dict.insert_str("__len__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__len__".to_string(),
            func: |_args| Ok(py_int(SEQ_FIELDS.len() as i64)),
        }));
        dict.insert_str("__getitem__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getitem__".to_string(),
            func: |args| {
                let fields = seq_fields(args.first().ok_or_else(|| {
                    PyError::type_error("stat_result.__getitem__() missing self")
                })?)?;
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "stat_result.__getitem__() takes at least 2 arguments",
                    ));
                }
                let key = &args[1];
                let n = key.as_i64().ok_or_else(|| {
                    PyError::type_error("stat_result indices must be integers")
                })?;
                let idx = if n < 0 { n + SEQ_FIELDS.len() as i64 } else { n };
                if idx < 0 || idx as usize >= SEQ_FIELDS.len() {
                    return Err(PyError::index_error("stat_result index out of range"));
                }
                Ok(fields[idx as usize].clone())
            },
        }));
        dict.insert_str("__iter__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__iter__".to_string(),
            func: |args| {
                let fields = seq_fields(args.first().ok_or_else(|| {
                    PyError::type_error("stat_result.__iter__() missing self")
                })?)?;
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: fields,
                    index: 0,
                }))
            },
        }));
        // CPython's repr: every indexed field, in order, in one line.
        dict.insert_str("__repr__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |args| {
                let fields = seq_fields(args.first().ok_or_else(|| {
                    PyError::type_error("stat_result.__repr__() missing self")
                })?)?;
                let mut out = String::from("stat_result(");
                for (i, (name, value)) in SEQ_FIELDS.iter().zip(fields.iter()).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(name);
                    out.push('=');
                    out.push_str(&value.repr());
                }
                out.push(')');
                Ok(py_str(&out))
            },
        }));
        dict.insert_str("n_fields", py_int(10));
        dict.insert_str("n_sequence_fields", py_int(10));
        dict.insert_str("n_unnamed_fields", py_int(3));
        let ty = PyObjectRef::new(PyObject::Type {
            name: "stat_result".to_string(),
            dict: Box::new(str_map_to_typedict(dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *ty.borrow_mut() {
            *mro = vec![ty.clone()];
        }
        Rc::new(ty)
    };
}

/// The `os.stat_result` type object.
pub(crate) fn stat_result_type() -> PyObjectRef {
    STAT_RESULT_TYPE.with(|t| t.as_ref().clone())
}

/// Wrap a `stat_to_dict` mapping in a real `stat_result` instance. This used
/// to be a MODULE with the `st_*` keys as its namespace, which made
/// `type(os.stat(p))` a module and — worse — made a MISSING field read back
/// as `None` instead of raising (`shutil.copystat` then died on
/// `'NoneType' object has no attribute 'st_mode'`).
pub(crate) fn make_stat_result(fields: HashMap<String, PyObjectRef>) -> PyObjectRef {
    let mut dict = AttrMap::new();
    for (k, v) in fields {
        dict.insert(k, v);
    }
    PyObjectRef::new(PyObject::Instance {
        typ: stat_result_type(),
        dict,
    })
}

fn seq_fields(obj: &PyObjectRef) -> PyResult<Vec<PyObjectRef>> {
    let borrowed = obj.borrow();
    let instance = match &*borrowed {
        PyObject::Instance { dict, .. } => dict,
        _ => {
            return Err(PyError::type_error(
                "stat_result method called on a non-stat_result",
            ))
        }
    };
    let mut out = Vec::with_capacity(SEQ_FIELDS.len());
    for name in SEQ_FIELDS {
        match instance.get(name) {
            Some(v) => out.push(v.clone()),
            None => {
                return Err(PyError::attribute_error(format!(
                    "stat_result has no attribute '{}'",
                    name
                )))
            }
        }
    }
    Ok(out)
}
