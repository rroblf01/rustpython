use crate::object::*;
use std::collections::HashMap;
use super::helpers::{ymd_to_ordinal, EPOCH_ORDINAL};
use super::tzif::{load_tz, tz_offset_for_instant};
use super::utils::{inst_get, inst_get_i64, instance_type_name};
use super::timedelta::make_timedelta;

thread_local! {
    static ZONEINFO_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

fn build_zoneinfo_type() -> PyObjectRef {
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
            if args.len() < 2 {
                return Err(PyError::type_error("ZoneInfo() missing key argument"));
            }
            let key = args[1].str();
            if load_tz(&key).is_none() {
                return Err(PyError::key_error(format!(
                    "No time zone found with key {}",
                    key
                )));
            }
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("key", py_str(&key));
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "utcoffset",
        bf!("utcoffset", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("utcoffset() missing datetime argument"));
            }
            let key = inst_get(&args[0], "key")
                .map(|v| v.str())
                .unwrap_or_default();
            let tz = load_tz(&key).ok_or_else(|| PyError::runtime_error("zone data not found"))?;
            let dt = &args[1];
            let ord = ymd_to_ordinal(
                inst_get_i64(dt, "year"),
                inst_get_i64(dt, "month"),
                inst_get_i64(dt, "day"),
            );
            let day_secs = inst_get_i64(dt, "hour") * 3600
                + inst_get_i64(dt, "minute") * 60
                + inst_get_i64(dt, "second");
            let unix_instant = (ord - EPOCH_ORDINAL) * 86400 + day_secs;
            let (off, _, _) = tz_offset_for_instant(&tz, unix_instant);
            Ok(make_timedelta(0, off as i64, 0))
        }),
    );
    type_dict.insert_str(
        "dst",
        bf!("dst", |args| {
            if args.len() < 2 {
                return Ok(py_none());
            }
            let key = inst_get(&args[0], "key")
                .map(|v| v.str())
                .unwrap_or_default();
            let tz = load_tz(&key).ok_or_else(|| PyError::runtime_error("zone data not found"))?;
            let dt = &args[1];
            let ord = ymd_to_ordinal(
                inst_get_i64(dt, "year"),
                inst_get_i64(dt, "month"),
                inst_get_i64(dt, "day"),
            );
            let day_secs = inst_get_i64(dt, "hour") * 3600
                + inst_get_i64(dt, "minute") * 60
                + inst_get_i64(dt, "second");
            let unix_instant = (ord - EPOCH_ORDINAL) * 86400 + day_secs;
            let (_, isdst, _) = tz_offset_for_instant(&tz, unix_instant);
            Ok(make_timedelta(0, if isdst { 3600 } else { 0 }, 0))
        }),
    );
    type_dict.insert_str(
        "tzname",
        bf!("tzname", |args| {
            if args.len() < 2 {
                return Ok(py_none());
            }
            let key = inst_get(&args[0], "key")
                .map(|v| v.str())
                .unwrap_or_default();
            let tz = load_tz(&key).ok_or_else(|| PyError::runtime_error("zone data not found"))?;
            let dt = &args[1];
            let ord = ymd_to_ordinal(
                inst_get_i64(dt, "year"),
                inst_get_i64(dt, "month"),
                inst_get_i64(dt, "day"),
            );
            let day_secs = inst_get_i64(dt, "hour") * 3600
                + inst_get_i64(dt, "minute") * 60
                + inst_get_i64(dt, "second");
            let unix_instant = (ord - EPOCH_ORDINAL) * 86400 + day_secs;
            let (_, _, name) = tz_offset_for_instant(&tz, unix_instant);
            Ok(py_str(&name))
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            let key = inst_get(&args[0], "key")
                .map(|v| v.str())
                .unwrap_or_default();
            Ok(py_str(&format!("zoneinfo.ZoneInfo(key='{}')", key)))
        }),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| Ok(
            inst_get(&args[0], "key").unwrap_or_else(|| py_str(""))
        )),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if instance_type_name(&args[1]) != "ZoneInfo" {
                return Ok(py_bool(false));
            }
            let a = inst_get(&args[0], "key")
                .map(|v| v.str())
                .unwrap_or_default();
            let b = inst_get(&args[1], "key")
                .map(|v| v.str())
                .unwrap_or_default();
            Ok(py_bool(a == b))
        }),
    );
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            let key = inst_get(&args[0], "key")
                .map(|v| v.str())
                .unwrap_or_default();
            builtin_hash(&[py_str(&key)])
        }),
    );

    PyObjectRef::new(PyObject::Type {
        name: "ZoneInfo".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub(crate) fn get_zoneinfo_type() -> PyObjectRef {
    let existing = ZONEINFO_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_zoneinfo_type();
    ZONEINFO_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}
