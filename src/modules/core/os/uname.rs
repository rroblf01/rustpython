// `os.uname()`/`os.uname_result` — was missing entirely (`AttributeError:
// 'module' object has no attribute 'uname_result'`), breaking
// `test_sysconfig.py`'s `_set_uname` helper (`os.uname_result(uname)`,
// used to fake `os.uname()` for platform-detection tests) and real
// `sysconfig.get_platform()`'s own `os.uname()` unpacking on POSIX.
// Distinct from (and NOT the same type as) `platform.uname_result`
// (`src/modules/misc/platform.rs`) — real CPython's `os.uname_result` has
// 5 fields (`sysname`, `nodename`, `release`, `version`, `machine`; no
// `processor`), a different structseq entirely. Built the same
// "synthetic cached `Type` + `Instance`" way as `time.struct_time`/
// `platform.uname_result` so it is simultaneously attribute-accessible
// AND a 5-element iterable/indexable sequence (real code does
// `osname, host, release, version, machine = os.uname()`).
use crate::object::*;
use std::collections::HashMap;

const UNAME_FIELDS: [&str; 5] = ["sysname", "nodename", "release", "version", "machine"];

thread_local! {
    static OS_UNAME_RESULT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

fn build_os_uname_result_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    type_dict.insert_str(
        "__getitem__",
        bf!("__getitem__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error(
                    "__getitem__() takes exactly one argument",
                ));
            }
            let idx = args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("indices must be integers"))?;
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let i = if idx < 0 { idx + 5 } else { idx };
                let name = UNAME_FIELDS
                    .get(i as usize)
                    .ok_or_else(|| PyError::index_error("uname_result index out of range"))?;
                Ok(dict.get(name).cloned().unwrap_or_else(py_none))
            } else {
                Err(PyError::runtime_error("__getitem__ on non-uname_result"))
            }
        }),
    );
    type_dict.insert_str("__len__", bf!("__len__", |_| Ok(py_int(5))));
    type_dict.insert_str(
        "__iter__",
        bf!("__iter__", |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let items: Vec<PyObjectRef> = UNAME_FIELDS
                    .iter()
                    .map(|f| dict.get(f).cloned().unwrap_or_else(py_none))
                    .collect();
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: items,
                    index: 0,
                }))
            } else {
                Err(PyError::runtime_error("__iter__ on non-uname_result"))
            }
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if args.len() < 2 {
                return Ok(py_bool(false));
            }
            let lhs = match &*args[0].borrow() {
                PyObject::Instance { dict, .. } => Some(
                    UNAME_FIELDS
                        .iter()
                        .map(|f| dict.get(f).cloned().unwrap_or_else(py_none))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            };
            let rhs: Option<Vec<PyObjectRef>> = match &*args[1].borrow() {
                PyObject::Instance { dict, .. } => Some(
                    UNAME_FIELDS
                        .iter()
                        .map(|f| dict.get(f).cloned().unwrap_or_else(py_none))
                        .collect::<Vec<_>>(),
                ),
                PyObject::Tuple(items) => Some(items.clone()),
                _ => None,
            };
            match (lhs, rhs) {
                (Some(l), Some(r)) => {
                    if l.len() != r.len() {
                        return Ok(py_bool(false));
                    }
                    for (a, b) in l.iter().zip(r.iter()) {
                        let eq = crate::object::py_compare(a, b, 2)?;
                        if !eq.truthy() {
                            return Ok(py_bool(false));
                        }
                    }
                    Ok(py_bool(true))
                }
                _ => Ok(py_bool(false)),
            }
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let body = UNAME_FIELDS
                    .iter()
                    .map(|f| {
                        format!(
                            "{}={}",
                            f,
                            dict.get(f)
                                .map(|v| v.repr())
                                .unwrap_or_else(|| "None".to_string())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Ok(py_str(&format!("posix.uname_result({})", body)))
            } else {
                Ok(py_str("posix.uname_result(...)"))
            }
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "os.uname_result".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn get_os_uname_result_type() -> PyObjectRef {
    let existing = OS_UNAME_RESULT_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_os_uname_result_type();
    OS_UNAME_RESULT_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn make_uname_result(values: [PyObjectRef; 5]) -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    for (name, value) in UNAME_FIELDS.iter().zip(values.into_iter()) {
        dict.insert_str(name, value);
    }
    PyObjectRef::new(PyObject::Instance {
        typ: get_os_uname_result_type(),
        dict,
    })
}

/// `os.uname()` — returns THIS process's real system info.
pub(crate) fn os_uname(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let sysname = py_str(std::env::consts::OS);
    let nodename = py_str(&std::env::var("HOSTNAME").unwrap_or_default());
    let machine = py_str(std::env::consts::ARCH);
    Ok(make_uname_result([
        sysname,
        nodename,
        py_str(""),
        py_str(""),
        machine,
    ]))
}

/// `os.uname_result(iterable)` — the structseq TYPE's constructor, used by
/// real code (and this test suite) to build/fake a result from an arbitrary
/// 5-element sequence rather than the real live system's own values.
pub(crate) fn os_uname_result_ctor(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `args[0]` is `self`/the type is never passed here — this is wired as
    // a plain `BuiltinFunction`, called as `os.uname_result(seq)`.
    if args.is_empty() {
        return Err(PyError::type_error(
            "uname_result() missing required argument 'sequence'",
        ));
    }
    let seq = args[0].clone();
    let items: Vec<PyObjectRef> = match &*seq.borrow() {
        PyObject::Tuple(items) => items.clone(),
        PyObject::List(items) => items.clone(),
        _ => {
            return Err(PyError::type_error(
                "uname_result() argument must be a sequence",
            ));
        }
    };
    if items.len() != 5 {
        return Err(PyError::type_error(format!(
            "uname_result() takes a 5-sequence ({}-sequence given)",
            items.len()
        )));
    }
    let arr: [PyObjectRef; 5] = items.try_into().unwrap();
    Ok(make_uname_result(arr))
}
