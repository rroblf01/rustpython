use crate::object::*;
use std::collections::HashMap;
use super::{inst_get, inst_get_i64, get_utcoffset_seconds, tzname_for, format_offset_iso, EPOCH_ORDINAL, CtorArgs, instance_type_name};
use super::timedelta::make_timedelta;

thread_local! {
    static TIME_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

// ---- time (time-of-day, no date component) ----

fn time_tuple_us(obj: &PyObjectRef) -> i64 {
    let h = inst_get_i64(obj, "hour");
    let mi = inst_get_i64(obj, "minute");
    let s = inst_get_i64(obj, "second");
    let us = inst_get_i64(obj, "microsecond");
    ((h * 3600 + mi * 60 + s) * 1_000_000) + us
}

fn time_isoformat(obj: &PyObjectRef) -> String {
    let h = inst_get_i64(obj, "hour");
    let mi = inst_get_i64(obj, "minute");
    let s = inst_get_i64(obj, "second");
    let us = inst_get_i64(obj, "microsecond");
    let mut out = format!("{:02}:{:02}:{:02}", h, mi, s);
    if us != 0 {
        out.push_str(&format!(".{:06}", us));
    }
    let tzinfo = inst_get(obj, "tzinfo").unwrap_or_else(py_none);
    if let Some(off) = get_utcoffset_seconds(&tzinfo, EPOCH_ORDINAL, 0) {
        out.push_str(&format_offset_iso(off));
    }
    out
}

fn build_time_type() -> PyObjectRef {
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
            let hour = ctor.get_i64(0, "hour", 0);
            let minute = ctor.get_i64(1, "minute", 0);
            let second = ctor.get_i64(2, "second", 0);
            let microsecond = ctor.get_i64(3, "microsecond", 0);
            let tzinfo = ctor.get(4, "tzinfo").unwrap_or_else(py_none);
            let fold = ctor.get_i64(5, "fold", 0);
            if !(0..24).contains(&hour) {
                return Err(PyError::value_error("hour must be in 0..23"));
            }
            if !(0..60).contains(&minute) {
                return Err(PyError::value_error("minute must be in 0..59"));
            }
            if !(0..60).contains(&second) {
                return Err(PyError::value_error("second must be in 0..59"));
            }
            if !(0..1_000_000).contains(&microsecond) {
                return Err(PyError::value_error("microsecond must be in 0..999999"));
            }
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("hour", py_int(hour));
                dict.insert_str("minute", py_int(minute));
                dict.insert_str("second", py_int(second));
                dict.insert_str("microsecond", py_int(microsecond));
                dict.insert_str("tzinfo", tzinfo);
                dict.insert_str("fold", py_int(fold));
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "isoformat",
        bf!("isoformat", |args| Ok(py_str(&time_isoformat(&args[0])))),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| Ok(py_str(&time_isoformat(&args[0])))),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            Ok(py_str(&format!(
                "datetime.time({}, {}, {})",
                inst_get_i64(&args[0], "hour"),
                inst_get_i64(&args[0], "minute"),
                inst_get_i64(&args[0], "second")
            )))
        }),
    );
    type_dict.insert_str(
        "replace",
        bf!("replace", |args| {
            let ctor = CtorArgs::parse(&args[1..]);
            let hour = ctor
                .get(0, "hour")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "hour"));
            let minute = ctor
                .get(1, "minute")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "minute"));
            let second = ctor
                .get(2, "second")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "second"));
            let microsecond = ctor
                .get(3, "microsecond")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "microsecond"));
            let tzinfo = ctor
                .get(4, "tzinfo")
                .unwrap_or_else(|| inst_get(&args[0], "tzinfo").unwrap_or_else(py_none));
            let fold = ctor.get_i64(5, "fold", 0);
            Ok(make_time(hour, minute, second, microsecond, tzinfo, fold))
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if instance_type_name(&args[1]) != "time" {
                return Ok(py_bool(false));
            }
            Ok(py_bool(time_tuple_us(&args[0]) == time_tuple_us(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__lt__",
        bf!("__lt__", |args| {
            if instance_type_name(&args[1]) != "time" {
                return Err(PyError::type_error(
                    "'<' not supported between instances of 'time' and other type",
                ));
            }
            Ok(py_bool(time_tuple_us(&args[0]) < time_tuple_us(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__le__",
        bf!("__le__", |args| {
            if instance_type_name(&args[1]) != "time" {
                return Err(PyError::type_error(
                    "'<=' not supported between instances of 'time' and other type",
                ));
            }
            Ok(py_bool(time_tuple_us(&args[0]) <= time_tuple_us(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__gt__",
        bf!("__gt__", |args| {
            if instance_type_name(&args[1]) != "time" {
                return Err(PyError::type_error(
                    "'>' not supported between instances of 'time' and other type",
                ));
            }
            Ok(py_bool(time_tuple_us(&args[0]) > time_tuple_us(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__ge__",
        bf!("__ge__", |args| {
            if instance_type_name(&args[1]) != "time" {
                return Err(PyError::type_error(
                    "'>=' not supported between instances of 'time' and other type",
                ));
            }
            Ok(py_bool(time_tuple_us(&args[0]) >= time_tuple_us(&args[1])))
        }),
    );
    // time.__hash__: CPython hashes the packed 6-byte representation
    // [hour, minute, second, us>>16, us>>8, us] with the seeded hash.
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            let h = inst_get_i64(&args[0], "hour");
            let mi = inst_get_i64(&args[0], "minute");
            let s = inst_get_i64(&args[0], "second");
            let us = inst_get_i64(&args[0], "microsecond");
            let bytes = [
                h as u8,
                mi as u8,
                s as u8,
                (us >> 16) as u8,
                (us >> 8) as u8,
                us as u8,
            ];
            Ok(py_int(crate::object::py_hash_bytes(&bytes) as i64))
        }),
    );
    type_dict.insert_str(
        "utcoffset",
        bf!("utcoffset", |args| {
            let tzinfo = inst_get(&args[0], "tzinfo").unwrap_or_else(py_none);
            match get_utcoffset_seconds(&tzinfo, EPOCH_ORDINAL, 0) {
                Some(s) => Ok(make_timedelta(0, s, 0)),
                None => Ok(py_none()),
            }
        }),
    );
    type_dict.insert_str(
        "tzname",
        bf!("tzname", |args| {
            let tzinfo = inst_get(&args[0], "tzinfo").unwrap_or_else(py_none);
            match tzname_for(&tzinfo, EPOCH_ORDINAL, 0) {
                Some(s) => Ok(py_str(&s)),
                None => Ok(py_none()),
            }
        }),
    );
    // time.fromisoformat — parse ISO time string HH:MM[:SS[.us]][+HH:MM]
    type_dict.insert_str(
        "fromisoformat",
        bf!("fromisoformat", |args| {
            if args.is_empty() {
                return Err(PyError::type_error(
                    "fromisoformat() requires an argument",
                ));
            }
            let s = args[0].str();
            let parts: Vec<&str> = s.splitn(2, 'T').collect();
            let time_str = if parts.len() == 2 { parts[1] } else { parts[0] };
            let (time_part, _tz_str) = if let Some(idx) = time_str.rfind('+') {
                (&time_str[..idx], &time_str[idx..])
            } else if let Some(idx) = time_str.rfind('-') {
                (&time_str[..idx], &time_str[idx..])
            } else {
                (time_str, "")
            };
            let time_parts: Vec<&str> = time_part.splitn(4, ':').collect();
            if time_parts.len() < 2 {
                return Err(PyError::value_error("Invalid isoformat string"));
            }
            let hour: i64 = time_parts[0].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
            let minute: i64 = time_parts[1].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
            let second: i64 = if time_parts.len() > 2 {
                let sec_str = time_parts[2].split('.').next().unwrap_or("0");
                sec_str.parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?
            } else { 0 };
            let microsecond: i64 = if time_parts.len() > 2 && time_parts[2].contains('.') {
                let us_str = time_parts[2].split('.').nth(1).unwrap_or("0");
                let padded = format!("{:0<6}", us_str);
                padded[..6].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?
            } else { 0 };
            Ok(make_time(hour, minute, second, microsecond, py_none(), 0))
        }),
    );

    PyObjectRef::new(PyObject::Type {
        name: "time".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub(crate) fn make_time(
    hour: i64,
    minute: i64,
    second: i64,
    microsecond: i64,
    tzinfo: PyObjectRef,
    fold: i64,
) -> PyObjectRef {
    let typ = get_time_type();
    let mut dict = AttrMap::new();
    dict.insert_str("hour", py_int(hour));
    dict.insert_str("minute", py_int(minute));
    dict.insert_str("second", py_int(second));
    dict.insert_str("microsecond", py_int(microsecond));
    dict.insert_str("tzinfo", tzinfo);
    dict.insert_str("fold", py_int(fold));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub(crate) fn get_time_type() -> PyObjectRef {
    let existing = TIME_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_time_type();
    TIME_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

