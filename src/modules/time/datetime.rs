use crate::object::*;
use std::collections::HashMap;
use super::helpers::{ymd_to_ordinal, ordinal_to_ymd, weekday_from_ordinal, day_of_year, days_in_month, EPOCH_ORDINAL};
use super::tzif::{load_tz, tz_offset_for_instant};
use super::utils::{inst_get, inst_get_i64, instance_type_name, get_utcoffset_seconds, tzname_for, format_utc_offset_name, format_offset_iso, CtorArgs};
use super::timedelta::{make_timedelta, make_timedelta_from_us, timedelta_total_us};
use super::timezone::{make_timezone, get_utc_singleton};

thread_local! {
    static DATETIME_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

use super::date::{make_date, get_date_type, date_ordinal};
use super::clock::{epoch_to_ymd, format_strftime};
use super::time_type::{make_time, get_time_type};

mod isoformat;
use isoformat::{datetime_isoformat, datetime_isoformat_ts};
mod parse;
use parse::parse_datetime_isoformat;
// ---- datetime ----

fn datetime_ordinal(obj: &PyObjectRef) -> i64 {
    ymd_to_ordinal(
        inst_get_i64(obj, "year"),
        inst_get_i64(obj, "month"),
        inst_get_i64(obj, "day"),
    )
}

fn datetime_day_us(obj: &PyObjectRef) -> i64 {
    let h = inst_get_i64(obj, "hour");
    let mi = inst_get_i64(obj, "minute");
    let s = inst_get_i64(obj, "second");
    let us = inst_get_i64(obj, "microsecond");
    ((h * 3600 + mi * 60 + s) * 1_000_000) + us
}

fn datetime_tzinfo(obj: &PyObjectRef) -> PyObjectRef {
    inst_get(obj, "tzinfo").unwrap_or_else(py_none)
}

fn datetime_is_aware(obj: &PyObjectRef) -> bool {
    !matches!(datetime_tzinfo(obj), PyObjectRef::None)
}

fn datetime_total_us_utc(obj: &PyObjectRef) -> i128 {
    let ord = datetime_ordinal(obj);
    let day_us = datetime_day_us(obj);
    let tz = datetime_tzinfo(obj);
    let mut total = (ord as i128) * 86_400_000_000 + day_us as i128;
    if let Some(off) = get_utcoffset_seconds(&tz, ord, day_us / 1_000_000) {
        total -= (off as i128) * 1_000_000;
    }
    total
}

pub(crate) fn make_datetime_from_total_us(total: i128, tzinfo: PyObjectRef) -> PyObjectRef {
    let ord = total.div_euclid(86_400_000_000);
    let day_us = total.rem_euclid(86_400_000_000);
    let (y, mo, d) = ordinal_to_ymd(ord as i64);
    let h = day_us / 3_600_000_000;
    let mi = (day_us % 3_600_000_000) / 60_000_000;
    let s = (day_us % 60_000_000) / 1_000_000;
    let us = day_us % 1_000_000;
    make_datetime(
        y, mo, d, h as i64, mi as i64, s as i64, us as i64, tzinfo, 0,
    )
}

fn build_datetime_type() -> PyObjectRef {
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
            let year = ctor.get_i64(0, "year", 1);
            let month = ctor.get_i64(1, "month", 1);
            let day = ctor.get_i64(2, "day", 1);
            let hour = ctor.get_i64(3, "hour", 0);
            let minute = ctor.get_i64(4, "minute", 0);
            let second = ctor.get_i64(5, "second", 0);
            let microsecond = ctor.get_i64(6, "microsecond", 0);
            let tzinfo = ctor.get(7, "tzinfo").unwrap_or_else(py_none);
            let fold = ctor.get_i64(8, "fold", 0);
            if !(1..=9999).contains(&year) {
                return Err(PyError::value_error("year out of range"));
            }
            if !(1..=12).contains(&month) {
                return Err(PyError::value_error("month must be in 1..12"));
            }
            if !(1..=days_in_month(year, month)).contains(&day) {
                return Err(PyError::value_error("day is out of range for month"));
            }
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
                dict.insert_str("year", py_int(year));
                dict.insert_str("month", py_int(month));
                dict.insert_str("day", py_int(day));
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
        bf!("isoformat", |args| {
            // kwargs arrive packed as a trailing Dict argument; the old code
            // stringified that dict and took its first char as the separator,
            // producing "2025-01-02{03:04:05" for isoformat(timespec=...).
            let mut sep = 'T';
            let mut timespec: Option<usize> = None;
            if args.len() > 1 {
                match &*args[1].borrow() {
                    PyObject::Str(sv) => {
                        sep = sv.chars().next().unwrap_or('T');
                    }
                    PyObject::Dict(d) => {
                        for (k, v) in d.items() {
                            let key = k.str();
                            match key.as_str() {
                                "sep" => sep = v.str().chars().next().unwrap_or('T'),
                                "timespec" => {
                                    timespec = match v.str().as_str() {
                                        "hours" => Some(1),
                                        "minutes" => Some(2),
                                        "seconds" => Some(3),
                                        "milliseconds" => Some(4),
                                        "microseconds" => Some(5),
                                        "auto" => None,
                                        other => {
                                            return Err(PyError::value_error(format!(
                                                "Invalid timespec {}",
                                                other
                                            )))
                                        }
                                    };
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(py_str(&datetime_isoformat_ts(&args[0], sep, timespec)))
        }),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| Ok(py_str(&datetime_isoformat(
            &args[0], ' '
        )))),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            Ok(py_str(&format!(
                "datetime.datetime({}, {}, {}, {}, {}, {})",
                inst_get_i64(&args[0], "year"),
                inst_get_i64(&args[0], "month"),
                inst_get_i64(&args[0], "day"),
                inst_get_i64(&args[0], "hour"),
                inst_get_i64(&args[0], "minute"),
                inst_get_i64(&args[0], "second"),
            )))
        }),
    );
    type_dict.insert_str(
        "date",
        bf!("date", |args| Ok(make_date(
            inst_get_i64(&args[0], "year"),
            inst_get_i64(&args[0], "month"),
            inst_get_i64(&args[0], "day")
        ))),
    );
    type_dict.insert_str(
        "time",
        bf!("time", |args| Ok(make_time(
            inst_get_i64(&args[0], "hour"),
            inst_get_i64(&args[0], "minute"),
            inst_get_i64(&args[0], "second"),
            inst_get_i64(&args[0], "microsecond"),
            py_none(),
            0
        ))),
    );
    type_dict.insert_str(
        "timetz",
        bf!("timetz", |args| Ok(make_time(
            inst_get_i64(&args[0], "hour"),
            inst_get_i64(&args[0], "minute"),
            inst_get_i64(&args[0], "second"),
            inst_get_i64(&args[0], "microsecond"),
            datetime_tzinfo(&args[0]),
            0
        ))),
    );
    type_dict.insert_str(
        "weekday",
        bf!("weekday", |args| Ok(py_int(weekday_from_ordinal(
            datetime_ordinal(&args[0])
        )))),
    );
    type_dict.insert_str(
        "isoweekday",
        bf!("isoweekday", |args| Ok(py_int(
            weekday_from_ordinal(datetime_ordinal(&args[0])) + 1
        ))),
    );
    type_dict.insert_str(
        "toordinal",
        bf!("toordinal", |args| Ok(py_int(datetime_ordinal(&args[0])))),
    );
    type_dict.insert_str(
        "timestamp",
        bf!("timestamp", |args| {
            let ord = datetime_ordinal(&args[0]);
            let day_us = datetime_day_us(&args[0]);
            let tz = datetime_tzinfo(&args[0]);
            let off = get_utcoffset_seconds(&tz, ord, day_us / 1_000_000).unwrap_or(0);
            let unix_us = (ord - EPOCH_ORDINAL) as i128 * 86_400_000_000 + day_us as i128
                - (off as i128) * 1_000_000;
            Ok(py_float(unix_us as f64 / 1_000_000.0))
        }),
    );
    type_dict.insert_str(
        "utcoffset",
        bf!("utcoffset", |args| {
            let tz = datetime_tzinfo(&args[0]);
            match get_utcoffset_seconds(
                &tz,
                datetime_ordinal(&args[0]),
                datetime_day_us(&args[0]) / 1_000_000,
            ) {
                Some(s) => Ok(make_timedelta(0, s, 0)),
                None => Ok(py_none()),
            }
        }),
    );
    type_dict.insert_str(
        "dst",
        bf!("dst", |args| {
            let tz = datetime_tzinfo(&args[0]);
            if matches!(tz, PyObjectRef::None) {
                return Ok(py_none());
            }
            if instance_type_name(&tz) == "ZoneInfo" {
                let key = inst_get(&tz, "key").map(|v| v.str()).unwrap_or_default();
                if let Some(parsed) = load_tz(&key) {
                    let ord = datetime_ordinal(&args[0]);
                    let day_us = datetime_day_us(&args[0]);
                    let unix_instant = (ord - EPOCH_ORDINAL) * 86400 + day_us / 1_000_000;
                    let (_, isdst, _) = tz_offset_for_instant(&parsed, unix_instant);
                    return Ok(make_timedelta(0, if isdst { 3600 } else { 0 }, 0));
                }
            }
            Ok(make_timedelta(0, 0, 0))
        }),
    );
    type_dict.insert_str(
        "tzname",
        bf!("tzname", |args| {
            let tz = datetime_tzinfo(&args[0]);
            match tzname_for(
                &tz,
                datetime_ordinal(&args[0]),
                datetime_day_us(&args[0]) / 1_000_000,
            ) {
                Some(s) => Ok(py_str(&s)),
                None => Ok(py_none()),
            }
        }),
    );
    type_dict.insert_str(
        "replace",
        bf!("replace", |args| {
            let ctor = CtorArgs::parse(&args[1..]);
            let year = ctor
                .get(0, "year")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "year"));
            let month = ctor
                .get(1, "month")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "month"));
            let day = ctor
                .get(2, "day")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "day"));
            let hour = ctor
                .get(3, "hour")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "hour"));
            let minute = ctor
                .get(4, "minute")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "minute"));
            let second = ctor
                .get(5, "second")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "second"));
            let microsecond = ctor
                .get(6, "microsecond")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| inst_get_i64(&args[0], "microsecond"));
            let tzinfo = ctor
                .get(7, "tzinfo")
                .unwrap_or_else(|| datetime_tzinfo(&args[0]));
            let fold = ctor.get_i64(8, "fold", 0);
            Ok(make_datetime(
                year,
                month,
                day,
                hour,
                minute,
                second,
                microsecond,
                tzinfo,
                fold,
            ))
        }),
    );
    type_dict.insert_str(
        "astimezone",
        bf!("astimezone", |args| {
            let new_tz = if args.len() > 1 {
                args[1].clone()
            } else {
                py_none()
            };
            let total_utc = datetime_total_us_utc(&args[0]);
            let ord = total_utc.div_euclid(86_400_000_000);
            let day_us_utc = total_utc.rem_euclid(86_400_000_000);
            let off = get_utcoffset_seconds(&new_tz, ord as i64, (day_us_utc / 1_000_000) as i64)
                .unwrap_or(0);
            let local_total = total_utc + (off as i128) * 1_000_000;
            Ok(make_datetime_from_total_us(local_total, new_tz))
        }),
    );
    type_dict.insert_str(
        "strftime",
        bf!("strftime", |args| {
            let fmt = if args.len() > 1 {
                args[1].str()
            } else {
                "%Y-%m-%d %H:%M:%S".to_string()
            };
            let ord = datetime_ordinal(&args[0]);
            let year = inst_get_i64(&args[0], "year");
            Ok(py_str(&format_strftime(
                &fmt,
                year,
                inst_get_i64(&args[0], "month"),
                inst_get_i64(&args[0], "day"),
                inst_get_i64(&args[0], "hour"),
                inst_get_i64(&args[0], "minute"),
                inst_get_i64(&args[0], "second"),
                weekday_from_ordinal(ord),
                day_of_year(year, ord),
            )))
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if instance_type_name(&args[1]) != "datetime" {
                return Ok(py_bool(false));
            }
            if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) {
                return Ok(py_bool(false));
            }
            Ok(py_bool(
                datetime_total_us_utc(&args[0]) == datetime_total_us_utc(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__lt__",
        bf!("__lt__", |args| {
            if instance_type_name(&args[1]) != "datetime" {
                return Err(PyError::type_error(
                    "'<' not supported between instances of 'datetime.datetime' and other type",
                ));
            }
            if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) {
                return Err(PyError::type_error(
                    "can't compare offset-naive and offset-aware datetimes",
                ));
            }
            Ok(py_bool(
                datetime_total_us_utc(&args[0]) < datetime_total_us_utc(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__le__",
        bf!("__le__", |args| {
            if instance_type_name(&args[1]) != "datetime" {
                return Err(PyError::type_error(
                    "'<=' not supported between instances of 'datetime.datetime' and other type",
                ));
            }
            if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) {
                return Err(PyError::type_error(
                    "can't compare offset-naive and offset-aware datetimes",
                ));
            }
            Ok(py_bool(
                datetime_total_us_utc(&args[0]) <= datetime_total_us_utc(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__gt__",
        bf!("__gt__", |args| {
            if instance_type_name(&args[1]) != "datetime" {
                return Err(PyError::type_error(
                    "'>' not supported between instances of 'datetime.datetime' and other type",
                ));
            }
            if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) {
                return Err(PyError::type_error(
                    "can't compare offset-naive and offset-aware datetimes",
                ));
            }
            Ok(py_bool(
                datetime_total_us_utc(&args[0]) > datetime_total_us_utc(&args[1]),
            ))
        }),
    );
    type_dict.insert_str(
        "__ge__",
        bf!("__ge__", |args| {
            if instance_type_name(&args[1]) != "datetime" {
                return Err(PyError::type_error(
                    "'>=' not supported between instances of 'datetime.datetime' and other type",
                ));
            }
            if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) {
                return Err(PyError::type_error(
                    "can't compare offset-naive and offset-aware datetimes",
                ));
            }
            Ok(py_bool(
                datetime_total_us_utc(&args[0]) >= datetime_total_us_utc(&args[1]),
            ))
        }),
    );
    // datetime.__hash__: CPython hashes the packed 10-byte representation
    // [date 4 bytes, time 6 bytes] with the seeded hash.
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            let y = inst_get_i64(&args[0], "year");
            let mo = inst_get_i64(&args[0], "month");
            let d = inst_get_i64(&args[0], "day");
            let h = inst_get_i64(&args[0], "hour");
            let mi = inst_get_i64(&args[0], "minute");
            let s = inst_get_i64(&args[0], "second");
            let us = inst_get_i64(&args[0], "microsecond");
            let bytes = [
                (y >> 8) as u8,
                (y & 0xff) as u8,
                mo as u8,
                d as u8,
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
        "__add__",
        bf!("__add__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for +: 'datetime.datetime' and other type",
                ));
            }
            let td_us = timedelta_total_us(&args[1]);
            let ord = datetime_ordinal(&args[0]);
            let day_us = datetime_day_us(&args[0]) as i128;
            let total = (ord as i128) * 86_400_000_000 + day_us + td_us;
            Ok(make_datetime_from_total_us(
                total,
                datetime_tzinfo(&args[0]),
            ))
        }),
    );
    type_dict.insert_str(
        "__radd__",
        bf!("__radd__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for +: 'datetime.datetime' and other type",
                ));
            }
            let td_us = timedelta_total_us(&args[1]);
            let ord = datetime_ordinal(&args[0]);
            let day_us = datetime_day_us(&args[0]) as i128;
            let total = (ord as i128) * 86_400_000_000 + day_us + td_us;
            Ok(make_datetime_from_total_us(
                total,
                datetime_tzinfo(&args[0]),
            ))
        }),
    );
    type_dict.insert_str(
        "__sub__",
        bf!("__sub__", |args| {
            match instance_type_name(&args[1]).as_str() {
                "timedelta" => {
                    let td_us = timedelta_total_us(&args[1]);
                    let ord = datetime_ordinal(&args[0]);
                    let day_us = datetime_day_us(&args[0]) as i128;
                    let total = (ord as i128) * 86_400_000_000 + day_us - td_us;
                    Ok(make_datetime_from_total_us(
                        total,
                        datetime_tzinfo(&args[0]),
                    ))
                }
                "datetime" => {
                    if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) {
                        return Err(PyError::type_error(
                            "can't subtract offset-naive and offset-aware datetimes",
                        ));
                    }
                    Ok(make_timedelta_from_us(
                        datetime_total_us_utc(&args[0]) - datetime_total_us_utc(&args[1]),
                    ))
                }
                _ => Err(PyError::type_error(
                    "unsupported operand type(s) for -: 'datetime.datetime' and other type",
                )),
            }
        }),
    );
    type_dict.insert_str(
        "now",
        bf!("now", |args| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(now.as_secs() as i64);
            let us = (now.subsec_nanos() / 1000) as i64;
            let tz = if !args.is_empty() && !matches!(args[0], PyObjectRef::None) {
                args[0].clone()
            } else {
                py_none()
            };
            if matches!(tz, PyObjectRef::None) {
                Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
            } else {
                let naive_ord = ymd_to_ordinal(y, mo, d);
                let day_us = ((h * 3600 + mi * 60 + s) * 1_000_000) + us;
                let off = get_utcoffset_seconds(&tz, naive_ord, day_us / 1_000_000).unwrap_or(0);
                let total = (naive_ord as i128) * 86_400_000_000
                    + day_us as i128
                    + (off as i128) * 1_000_000;
                Ok(make_datetime_from_total_us(total, tz))
            }
        }),
    );
    type_dict.insert_str(
        "utcnow",
        bf!("utcnow", |_args| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(now.as_secs() as i64);
            let us = (now.subsec_nanos() / 1000) as i64;
            Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
        }),
    );
    type_dict.insert_str(
        "today",
        bf!("today", |_args| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(now.as_secs() as i64);
            Ok(make_datetime(y, mo, d, h, mi, s, 0, py_none(), 0))
        }),
    );
    type_dict.insert_str(
        "fromtimestamp",
        bf!("fromtimestamp", |args| {
            let ts = if !args.is_empty() {
                args[0].as_f64().unwrap_or(0.0)
            } else {
                0.0
            };
            let tz = if args.len() > 1 && !matches!(args[1], PyObjectRef::None) {
                args[1].clone()
            } else {
                py_none()
            };
            let secs = ts.floor() as i64;
            let us = ((ts - ts.floor()) * 1_000_000.0).round() as i64;
            if matches!(tz, PyObjectRef::None) {
                let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(secs);
                Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
            } else {
                let off = get_utcoffset_seconds(
                    &tz,
                    EPOCH_ORDINAL + secs.div_euclid(86400),
                    secs.rem_euclid(86400),
                )
                .unwrap_or(0);
                let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(secs + off);
                Ok(make_datetime(y, mo, d, h, mi, s, us, tz, 0))
            }
        }),
    );
    type_dict.insert_str(
        "utcfromtimestamp",
        bf!("utcfromtimestamp", |args| {
            let ts = if !args.is_empty() {
                args[0].as_f64().unwrap_or(0.0)
            } else {
                0.0
            };
            let secs = ts.floor() as i64;
            let us = ((ts - ts.floor()) * 1_000_000.0).round() as i64;
            let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(secs);
            Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
        }),
    );
    type_dict.insert_str(
        "combine",
        bf!("combine", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error(
                    "combine() requires date and time arguments",
                ));
            }
            let d = &args[0];
            let t = &args[1];
            let tzinfo = if args.len() > 2 {
                args[2].clone()
            } else {
                inst_get(t, "tzinfo").unwrap_or_else(py_none)
            };
            Ok(make_datetime(
                inst_get_i64(d, "year"),
                inst_get_i64(d, "month"),
                inst_get_i64(d, "day"),
                inst_get_i64(t, "hour"),
                inst_get_i64(t, "minute"),
                inst_get_i64(t, "second"),
                inst_get_i64(t, "microsecond"),
                tzinfo,
                0,
            ))
        }),
    );
    type_dict.insert_str(
        "fromisoformat",
        bf!("fromisoformat", |args| {
            let s = if !args.is_empty() {
                args[0].str()
            } else {
                String::new()
            };
            parse_datetime_isoformat(&s)
        }),
    );
    // datetime.datetime.isocalendar() — delegates to date.isocalendar
    type_dict.insert_str(
        "isocalendar",
        bf!("isocalendar", |args| {
            // Get the date portion and call date.isocalendar logic
            let year = inst_get_i64(&args[0], "year");
            let month = inst_get_i64(&args[0], "month");
            let day = inst_get_i64(&args[0], "day");
            // ISO weekday: 1=Monday ... 7=Sunday
            let leap = if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 { 1 } else { 0 };
            let days_in_month = [0i64,31,28+leap,31,30,31,30,31,31,30,31,30,31];
            let mut yday = day;
            for m in 1..month {
                yday += days_in_month[m as usize];
            }
            let wday = weekday_from_ordinal(date_ordinal(&args[0])) + 1;
            // ISO week number using Julian Day
            let a_j = (14 - 1) / 12;
            let y_j = year + 4800 - a_j;
            let m_j = 1 + 12 * a_j - 3;
            let jd_jan1 = 1 + (153 * m_j + 2) / 5 + 365 * y_j + y_j / 4 - y_j / 100 + y_j / 400 - 32045;
            let a_d = (14 - month) / 12;
            let y_d = year + 4800 - a_d;
            let m_d = month + 12 * a_d - 3;
            let jd = day + (153 * m_d + 2) / 5 + 365 * y_d + y_d / 4 - y_d / 100 + y_d / 400 - 32045;
            // Delegate to the date-type implementation: identical math,
            // and it carries the year-boundary correction.
            let d_obj = make_date(year, month, day);
            let iso_fn =
                crate::object::lookup_dunder_via_mro(&get_date_type(), "isocalendar")
                    .expect("date.isocalendar missing");
            call_bound_method(iso_fn, d_obj, vec![])
        }),
    );
    // datetime.datetime.fromisocalendar(year, week, weekday) — classmethod
    type_dict.insert_str(
        "fromisocalendar",
        bf!("fromisocalendar", |args| {
            if args.len() < 3 {
                return Err(PyError::type_error(
                    "fromisocalendar() missing required arguments: 'year', 'week', 'weekday'",
                ));
            }
            let year = args[0].as_i64().ok_or_else(|| PyError::type_error("year must be an integer"))? as i64;
            let week = args[1].as_i64().ok_or_else(|| PyError::type_error("week must be an integer"))? as i64;
            let weekday = args[2].as_i64().ok_or_else(|| PyError::type_error("weekday must be an integer"))? as i64;
            // Calculate the date from ISO week
            let jan4_wday = ((year * 365 + (year - 1) / 4 - (year - 1) / 100 + (year - 1) / 400 + 4) % 7 + 1) % 7 + 1;
            let mon_of_week1 = 4 - jan4_wday;
            let mon_of_week1_adj = if mon_of_week1 <= 0 { mon_of_week1 + 7 } else { mon_of_week1 };
            let day_of_year = mon_of_week1_adj + (week - 1) * 7 + (weekday - 1);
            let leap = if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 { 1 } else { 0 };
            let days_in_month = [0i64,31,28+leap,31,30,31,30,31,31,30,31,30,31];
            let mut remaining = day_of_year;
            let mut month_out: i64 = 1;
            for m in 1i64..13 {
                if remaining <= days_in_month[m as usize] {
                    month_out = m;
                    break;
                }
                remaining -= days_in_month[m as usize];
            }
            Ok(make_datetime(year, month_out, remaining, 0, 0, 0, 0, py_none(), 0))
        }),
    );

    // datetime.min and datetime.max
    type_dict.insert_str(
        "min",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "min".to_string(),
            func: |_args| Ok(make_datetime(1, 1, 1, 0, 0, 0, 0, py_none(), 0)),
        }),
    );
    type_dict.insert_str(
        "max",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "max".to_string(),
            func: |_args| Ok(make_datetime(9999, 12, 31, 23, 59, 59, 999999, py_none(), 0)),
        }),
    );

    PyObjectRef::new(PyObject::Type {
        name: "datetime".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub(crate) fn make_datetime(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    microsecond: i64,
    tzinfo: PyObjectRef,
    fold: i64,
) -> PyObjectRef {
    let typ = get_datetime_type();
    let mut dict = AttrMap::new();
    dict.insert_str("year", py_int(year));
    dict.insert_str("month", py_int(month));
    dict.insert_str("day", py_int(day));
    dict.insert_str("hour", py_int(hour));
    dict.insert_str("minute", py_int(minute));
    dict.insert_str("second", py_int(second));
    dict.insert_str("microsecond", py_int(microsecond));
    dict.insert_str("tzinfo", tzinfo);
    dict.insert_str("fold", py_int(fold));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub(crate) fn get_datetime_type() -> PyObjectRef {
    let existing = DATETIME_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_datetime_type();
    DATETIME_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}
