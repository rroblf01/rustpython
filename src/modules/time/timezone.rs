use crate::object::*;
use std::collections::HashMap;
use super::utils::{inst_get, inst_get_i64, instance_type_name, CtorArgs, format_utc_offset_name};
use super::timedelta::make_timedelta;

thread_local! {
    static TIMEZONE_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

fn build_timezone_type() -> PyObjectRef {
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
        "__init__",
        bf!("__init__", |args| {
            let ctor = CtorArgs::parse(&args[1..]);
            let offset_seconds = match ctor.get(0, "offset") {
                Some(td) if instance_type_name(&td) == "timedelta" => {
                    inst_get_i64(&td, "days") * 86400 + inst_get_i64(&td, "seconds")
                }
                _ => 0,
            };
            let name = ctor.get(1, "name").map(|v| v.str());
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("_offset_seconds", py_int(offset_seconds));
                dict.insert_str("_name", name.map(|n| py_str(&n)).unwrap_or_else(py_none));
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "utcoffset",
        bf!("utcoffset", |args| Ok(make_timedelta(
            0,
            inst_get_i64(&args[0], "_offset_seconds"),
            0
        ))),
    );
    type_dict.insert_str("dst", bf!("dst", |_args| Ok(py_none())));
    type_dict.insert_str(
        "tzname",
        bf!("tzname", |args| {
            match inst_get(&args[0], "_name") {
                Some(v) if !matches!(v, PyObjectRef::None) => Ok(v),
                _ => Ok(py_str(&format_utc_offset_name(inst_get_i64(
                    &args[0],
                    "_offset_seconds",
                )))),
            }
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if instance_type_name(&args[1]) != "timezone" {
                return Ok(py_bool(false));
            }
            Ok(py_bool(
                inst_get_i64(&args[0], "_offset_seconds")
                    == inst_get_i64(&args[1], "_offset_seconds"),
            ))
        }),
    );
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| Ok(py_int(inst_get_i64(
            &args[0],
            "_offset_seconds"
        )))),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            let off = inst_get_i64(&args[0], "_offset_seconds");
            if off == 0 {
                Ok(py_str("datetime.timezone.utc"))
            } else {
                Ok(py_str(&format!(
                    "datetime.timezone(datetime.timedelta(seconds={}))",
                    off
                )))
            }
        }),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| {
            match inst_get(&args[0], "_name") {
                Some(v) if !matches!(v, PyObjectRef::None) => Ok(v),
                _ => Ok(py_str(&format_utc_offset_name(inst_get_i64(
                    &args[0],
                    "_offset_seconds",
                )))),
            }
        }),
    );

    let typ = PyObjectRef::new(PyObject::Type {
        name: "timezone".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });

    // timezone.min and timezone.max — created AFTER the type exists
    // to avoid circular reference (make_timezone calls get_timezone_type)
    let tz_min = make_timezone_with_type(typ.clone(), -86400, None);
    let tz_max = make_timezone_with_type(typ.clone(), 86400, None);
    if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
        dict.insert_str("min", tz_min);
        dict.insert_str("max", tz_max);
    }

    typ
}

pub(crate) fn make_timezone_with_type(
    typ: PyObjectRef,
    offset_seconds: i64,
    name: Option<String>,
) -> PyObjectRef {
    let mut dict = AttrMap::new();
    dict.insert_str("_offset_seconds", py_int(offset_seconds));
    dict.insert_str("_name", name.map(|n| py_str(&n)).unwrap_or_else(py_none));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub(crate) fn make_timezone(offset_seconds: i64, name: Option<String>) -> PyObjectRef {
    make_timezone_with_type(get_timezone_type(), offset_seconds, name)
}

pub(crate) fn get_timezone_type() -> PyObjectRef {
    let existing = TIMEZONE_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_timezone_type();
    let utc_inst = make_timezone_with_type(typ.clone(), 0, None);
    if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
        dict.insert_str("utc", utc_inst);
    }
    TIMEZONE_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

pub(crate) fn get_utc_singleton() -> PyObjectRef {
    let tz_type = get_timezone_type();
    let borrowed = tz_type.borrow();
    if let PyObject::Type { dict, .. } = &*borrowed {
        dict.get_str("utc")
            .cloned()
            .unwrap_or_else(|| make_timezone(0, None))
    } else {
        make_timezone(0, None)
    }
}
