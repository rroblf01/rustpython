use crate::object::*;
use std::collections::HashMap;
use super::{normalize_timedelta, inst_get_i64, instance_type_name, CtorArgs};

thread_local! {
    static TIMEDELTA_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

// ---- timedelta ----

pub(crate) fn timedelta_total_us(obj: &PyObjectRef) -> i128 {
    let d = inst_get_i64(obj, "days") as i128;
    let s = inst_get_i64(obj, "seconds") as i128;
    let us = inst_get_i64(obj, "microseconds") as i128;
    d * 86_400_000_000 + s * 1_000_000 + us
}

pub(crate) fn make_timedelta_from_us(us: i128) -> PyObjectRef {
    let days = us.div_euclid(86_400_000_000);
    let rem = us.rem_euclid(86_400_000_000);
    let seconds = rem / 1_000_000;
    let micro = rem % 1_000_000;
    make_timedelta(days as i64, seconds as i64, micro as i64)
}

fn timedelta_str(obj: &PyObjectRef) -> String {
    let d = inst_get_i64(obj, "days");
    let s = inst_get_i64(obj, "seconds");
    let us = inst_get_i64(obj, "microseconds");
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    let mut out = String::new();
    if d != 0 {
        out.push_str(&format!(
            "{} day{}, ",
            d,
            if d.abs() == 1 { "" } else { "s" }
        ));
    }
    out.push_str(&format!("{}:{:02}:{:02}", h, m, sec));
    if us != 0 {
        out.push_str(&format!(".{:06}", us));
    }
    out
}

fn build_timedelta_type() -> PyObjectRef {
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
            let days = ctor.get_i64(0, "days", 0);
            let seconds = ctor.get_i64(1, "seconds", 0);
            let microseconds = ctor.get_i64(2, "microseconds", 0);
            let milliseconds = ctor.get_i64(3, "milliseconds", 0);
            let minutes = ctor.get_i64(4, "minutes", 0);
            let hours = ctor.get_i64(5, "hours", 0);
            let weeks = ctor.get_i64(6, "weeks", 0);
            let total_days = days + weeks * 7;
            let total_seconds = seconds + minutes * 60 + hours * 3600;
            let total_us = microseconds + milliseconds * 1000;
            let (d, s, us) = normalize_timedelta(total_days, total_seconds, total_us);
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("days", py_int(d));
                dict.insert_str("seconds", py_int(s));
                dict.insert_str("microseconds", py_int(us));
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "total_seconds",
        bf!("total_seconds", |args| {
            let d = inst_get_i64(&args[0], "days");
            let s = inst_get_i64(&args[0], "seconds");
            let us = inst_get_i64(&args[0], "microseconds");
            Ok(py_float(
                d as f64 * 86400.0 + s as f64 + us as f64 / 1_000_000.0,
            ))
        }),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| Ok(py_str(&timedelta_str(&args[0])))),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            let d = inst_get_i64(&args[0], "days");
            let s = inst_get_i64(&args[0], "seconds");
            let us = inst_get_i64(&args[0], "microseconds");
            let mut parts = vec![];
            if d != 0 {
                parts.push(format!("days={}", d));
            }
            if s != 0 {
                parts.push(format!("seconds={}", s));
            }
            if us != 0 {
                parts.push(format!("microseconds={}", us));
            }
            if parts.is_empty() {
                parts.push("0".to_string());
            }
            Ok(py_str(&format!("datetime.timedelta({})", parts.join(", "))))
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Ok(py_bool(false));
            }
            Ok(py_bool(
                timedelta_total_us(&args[0]) == timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__lt__",
        bf!("__lt__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "'<' not supported between instances of 'timedelta' and other type",
                ));
            }
            Ok(py_bool(
                timedelta_total_us(&args[0]) < timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__le__",
        bf!("__le__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "'<=' not supported between instances of 'timedelta' and other type",
                ));
            }
            Ok(py_bool(
                timedelta_total_us(&args[0]) <= timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__gt__",
        bf!("__gt__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "'>' not supported between instances of 'timedelta' and other type",
                ));
            }
            Ok(py_bool(
                timedelta_total_us(&args[0]) > timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__ge__",
        bf!("__ge__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "'>=' not supported between instances of 'timedelta' and other type",
                ));
            }
            Ok(py_bool(
                timedelta_total_us(&args[0]) >= timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| Ok(py_int(
            timedelta_total_us(&args[0]) as i64
        ))),
    );
    type_dict.insert_str(
        "__bool__",
        bf!("__bool__", |args| Ok(py_bool(
            timedelta_total_us(&args[0]) != 0
        ))),
    );
    type_dict.insert_str(
        "__add__",
        bf!("__add__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for +: 'timedelta' and other type",
                ));
            }
            Ok(make_timedelta_from_us(
                timedelta_total_us(&args[0]) + timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__radd__",
        bf!("__radd__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for +: 'timedelta' and other type",
                ));
            }
            Ok(make_timedelta_from_us(
                timedelta_total_us(&args[0]) + timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__sub__",
        bf!("__sub__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for -: 'timedelta' and other type",
                ));
            }
            Ok(make_timedelta_from_us(
                timedelta_total_us(&args[0]) - timedelta_total_us(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__rsub__",
        bf!("__rsub__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for -: 'timedelta' and other type",
                ));
            }
            Ok(make_timedelta_from_us(
                timedelta_total_us(&args[1]) - timedelta_total_us(&args[0]),
            ))
        }),
    );
    type_dict.insert_str(
        "__neg__",
        bf!("__neg__", |args| Ok(make_timedelta_from_us(
            -timedelta_total_us(&args[0])
        ))),
    );
    type_dict.insert_str(
        "__pos__",
        bf!("__pos__", |args| Ok(make_timedelta_from_us(
            timedelta_total_us(&args[0])
        ))),
    );
    type_dict.insert_str(
        "__abs__",
        bf!("__abs__", |args| Ok(make_timedelta_from_us(
            timedelta_total_us(&args[0]).abs()
        ))),
    );
    type_dict.insert_str(
        "__mul__",
        bf!("__mul__", |args| {
            let factor = args[1]
                .as_f64()
                .ok_or_else(|| PyError::type_error("unsupported operand type(s) for *"))?;
            Ok(make_timedelta_from_us(
                (timedelta_total_us(&args[0]) as f64 * factor).round() as i128,
            ))
        }),
    );
    type_dict.insert_str(
        "__rmul__",
        bf!("__rmul__", |args| {
            let factor = args[1]
                .as_f64()
                .ok_or_else(|| PyError::type_error("unsupported operand type(s) for *"))?;
            Ok(make_timedelta_from_us(
                (timedelta_total_us(&args[0]) as f64 * factor).round() as i128,
            ))
        }),
    );
    type_dict.insert_str(
        "__truediv__",
        bf!("__truediv__", |args| {
            if instance_type_name(&args[1]) == "timedelta" {
                let a = timedelta_total_us(&args[0]) as f64;
                let b = timedelta_total_us(&args[1]) as f64;
                return Ok(py_float(a / b));
            }
            let divisor = args[1]
                .as_f64()
                .ok_or_else(|| PyError::type_error("unsupported operand type(s) for /"))?;
            Ok(make_timedelta_from_us(
                (timedelta_total_us(&args[0]) as f64 / divisor).round() as i128,
            ))
        }),
    );
    type_dict.insert_str(
        "__floordiv__",
        bf!("__floordiv__", |args| {
            if instance_type_name(&args[1]) == "timedelta" {
                let a = timedelta_total_us(&args[0]);
                let b = timedelta_total_us(&args[1]);
                if b == 0 {
                    return Err(PyError::zero_division());
                }
                return Ok(py_int((a / b) as i64));
            }
            let divisor = args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("unsupported operand type(s) for //"))?;
            if divisor == 0 {
                return Err(PyError::zero_division());
            }
            Ok(make_timedelta_from_us(
                timedelta_total_us(&args[0]) / divisor as i128,
            ))
        }),
    );

    PyObjectRef::new(PyObject::Type {
        name: "timedelta".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub(crate) fn make_timedelta_with_type(
    typ: PyObjectRef,
    days: i64,
    seconds: i64,
    microseconds: i64,
) -> PyObjectRef {
    let mut dict = AttrMap::new();
    dict.insert_str("days", py_int(days));
    dict.insert_str("seconds", py_int(seconds));
    dict.insert_str("microseconds", py_int(microseconds));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub(crate) fn make_timedelta(days: i64, seconds: i64, microseconds: i64) -> PyObjectRef {
    let (days, seconds, microseconds) = normalize_timedelta(days, seconds, microseconds);
    make_timedelta_with_type(get_timedelta_type(), days, seconds, microseconds)
}

pub(crate) fn get_timedelta_type() -> PyObjectRef {
    let existing = TIMEDELTA_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_timedelta_type();
    let min_inst = make_timedelta_with_type(typ.clone(), -999_999_999, 0, 0);
    let max_inst = make_timedelta_with_type(typ.clone(), 999_999_999, 86399, 999_999);
    let res_inst = make_timedelta_with_type(typ.clone(), 0, 0, 1);
    if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
        dict.insert_str("min", min_inst);
        dict.insert_str("max", max_inst);
        dict.insert_str("resolution", res_inst);
    }
    TIMEDELTA_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

