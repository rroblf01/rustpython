use crate::object::*;
use std::collections::HashMap;

mod helpers;
pub(crate) use helpers::{
    days_before_month, days_before_month_table, days_before_year, days_in_month, day_of_year,
    is_leap_year, normalize_timedelta, ordinal_to_ymd, weekday_from_ordinal, ymd_to_ordinal,
    EPOCH_ORDINAL,
};

mod tzif;
pub(crate) use tzif::{load_tz, tz_offset_for_instant};

mod utils;
pub(crate) use utils::{
    format_offset_iso, format_utc_offset_name, get_tzinfo_type, get_utcoffset_seconds, inst_get,
    inst_get_i64, instance_type_name, tzname_for, CtorArgs,
};

mod clock;
pub use clock::create_time_dict;
pub(crate) use clock::{civil_to_days, epoch_to_ymd, format_strftime, weekday_yday_for};

mod timedelta;
pub(crate) use timedelta::{
    get_timedelta_type, make_timedelta, make_timedelta_from_us, make_timedelta_with_type,
    timedelta_total_us,
};
mod date;
pub(crate) use date::{date_ordinal, get_date_type, make_date, make_date_from_ordinal};
mod time_type;
pub(crate) use time_type::{get_time_type, make_time};

mod datetime;
pub(crate) use datetime::{get_datetime_type, make_datetime, make_datetime_from_total_us};

mod timezone;
pub(crate) use timezone::{
    get_timezone_type, get_utc_singleton, make_timezone, make_timezone_with_type,
};

mod zoneinfo;
pub(crate) use zoneinfo::get_zoneinfo_type;

pub fn create_datetime_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("date", get_date_type());
    d.insert_str("time", get_time_type());
    d.insert_str("datetime", get_datetime_type());
    d.insert_str("timedelta", get_timedelta_type());
    let timezone_type = get_timezone_type();
    let utc_singleton = if let PyObject::Type { dict, .. } = &*timezone_type.borrow() {
        dict.get_str("utc")
            .cloned()
            .unwrap_or_else(|| make_timezone(0, None))
    } else {
        make_timezone(0, None)
    };
    d.insert_str("timezone", timezone_type);
    // `datetime.UTC` (3.11+) is the exact same object as `timezone.utc`.
    d.insert_str("UTC", utc_singleton);
    d.insert_str("tzinfo", get_tzinfo_type());
    d.insert_str("MINYEAR", py_int(1));
    d.insert_str("MAXYEAR", py_int(9999));
    d
}

pub fn create_zoneinfo_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("ZoneInfo", get_zoneinfo_type());
    d.insert_str(
        "available_timezones",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "available_timezones".to_string(),
            func: |_args| {
                let mut set = crate::object::PySet::new();
                fn walk(base: &std::path::Path, prefix: &str, set: &mut crate::object::PySet) {
                    let entries = match std::fs::read_dir(base) {
                        Ok(e) => e,
                        Err(_) => return,
                    };
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') {
                            continue;
                        }
                        let skip = matches!(
                            name.as_str(),
                            "posix"
                                | "right"
                                | "posixrules"
                                | "Factory"
                                | "iso3166.tab"
                                | "zone.tab"
                                | "zone1970.tab"
                                | "tzdata.zi"
                                | "leapseconds"
                                | "leap-seconds.list"
                        );
                        if skip {
                            continue;
                        }
                        let path = entry.path();
                        let rel = if prefix.is_empty() {
                            name.clone()
                        } else {
                            format!("{}/{}", prefix, name)
                        };
                        if path.is_dir() {
                            walk(&path, &rel, set);
                        } else {
                            let _ = set.add(py_str(&rel));
                        }
                    }
                }
                walk(std::path::Path::new("/usr/share/zoneinfo"), "", &mut set);
                Ok(PyObjectRef::new(PyObject::Set(set)))
            },
        }),
    );
    d
}
