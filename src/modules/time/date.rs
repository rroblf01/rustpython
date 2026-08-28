use crate::object::*;
use std::collections::HashMap;
use super::{ymd_to_ordinal, ordinal_to_ymd, weekday_from_ordinal, day_of_year, days_in_month, inst_get_i64, instance_type_name, CtorArgs, format_strftime, epoch_to_ymd, make_datetime, make_timezone, get_utc_singleton};
use super::timedelta::make_timedelta;

thread_local! {
    static DATE_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

// ---- date ----

pub(crate) fn date_ordinal(obj: &PyObjectRef) -> i64 {
    ymd_to_ordinal(
        inst_get_i64(obj, "year"),
        inst_get_i64(obj, "month"),
        inst_get_i64(obj, "day"),
    )
}

pub(crate) fn make_date_from_ordinal(ord: i64) -> PyObjectRef {
    let (y, m, d) = ordinal_to_ymd(ord);
    make_date(y, m, d)
}


/// Build an `IsoCalendarDate`-like instance (CPython returns a
/// tuple subclass with named fields year/week/weekday). Our version is an
/// Instance exposing attribute access, iteration, indexing and equality
/// against plain tuples so `isocalendar()` results behave like the real
/// thing for both attribute-style and tuple-style consumers.

/// Correct an approximate ISO week number for year boundaries: week 0
/// belongs to the last week of the previous year; anything beyond that
/// year's week count rolls into week 1 of the next.
fn iso_week_corrected(year: i64, approx_week: i64) -> (i64, i64) {
    fn p(y: i64) -> i64 {
        (y + y / 4 - y / 100 + y / 400) % 7
    }
    fn weeks_in_year(y: i64) -> i64 {
        if p(y) == 4 || p(y - 1) == 3 { 53 } else { 52 }
    }
    if approx_week <= 0 {
        return (year - 1, weeks_in_year(year - 1));
    }
    if approx_week > weeks_in_year(year) {
        return (year + 1, 1);
    }
    (year, approx_week)
}

fn make_isocalendar(year: i64, week: i64, weekday: i64) -> PyObjectRef {
    thread_local! {
        static ISO_TYPE: std::cell::RefCell<Option<PyObjectRef>> = const { std::cell::RefCell::new(None) };
    }
    let typ = ISO_TYPE.with(|c| {
        if let Some(t) = &*c.borrow() { return t.clone(); }
        let mut td: HashMap<String, PyObjectRef> = HashMap::new();
        let bf = |name: &str, f: crate::object::BuiltinFunc| {
            PyObjectRef::new(PyObject::BuiltinFunction { name: name.to_string(), func: f })
        };
        td.insert("__iter__".into(), bf("__iter__", |args| {
            let o = &args[0];
            let g = |n: &str| inst_get_i64(o, n);
            let tup = py_tuple(vec![
                py_int(g("year")),
                py_int(g("week")),
                py_int(g("weekday")),
            ]);
            crate::object::builtin_iter(&[tup])
        }));
        td.insert("__len__".into(), bf("__len__", |_a| Ok(py_int(3))));
        td.insert("__getitem__".into(), bf("__getitem__", |args| {
            let i = args[1].as_i64().unwrap_or(-999);
            let o = &args[0];
            let vals = [inst_get_i64(o, "year"), inst_get_i64(o, "week"), inst_get_i64(o, "weekday")];
            let idx = if i < 0 { 3 + i } else { i };
            if !(0..3).contains(&idx) { return Err(PyError::index_error("IsoCalendarDate index out of range")); }
            Ok(py_int(vals[idx as usize]))
        }));
        td.insert("__eq__".into(), bf("__eq__", |args| {
            let o = &args[0]; let other = &args[1];
            let vals = [inst_get_i64(o, "year"), inst_get_i64(o, "week"), inst_get_i64(o, "weekday")];
            let eq = match &*other.borrow() {
                PyObject::Tuple(t) => t.len() == 3 && t.iter().zip(vals.iter()).all(|(v, iv)| v.as_i64() == Some(*iv)),
                _ => {
                    if other.borrow().type_name() == "IsoCalendarDate" {
                        [inst_get_i64(other, "year"), inst_get_i64(other, "week"), inst_get_i64(other, "weekday")] == vals
                    } else { false }
                }
            };
            Ok(py_bool(eq))
        }));
        td.insert("__repr__".into(), bf("__repr__", |args| {
            let o = args[0].clone();
            Ok(py_str(&format!("IsoCalendarDate(year={}, week={}, weekday={})",
                inst_get_i64(&o,"year"), inst_get_i64(&o,"week"), inst_get_i64(&o,"weekday"))))
        }));
        let t = PyObjectRef::new(PyObject::Type {
            name: "IsoCalendarDate".into(),
            dict: Box::new(crate::object::str_map_to_typedict(td)),
            bases: vec![],
            mro: vec![],
        });
        *c.borrow_mut() = Some(t.clone());
        t
    });
    let mut dict = AttrMap::new();
    dict.insert_str("year", py_int(year));
    dict.insert_str("week", py_int(week));
    dict.insert_str("weekday", py_int(weekday));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn build_date_type() -> PyObjectRef {
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
            if !(1..=9999).contains(&year) {
                return Err(PyError::value_error("year out of range"));
            }
            if !(1..=12).contains(&month) {
                return Err(PyError::value_error("month must be in 1..12"));
            }
            if !(1..=days_in_month(year, month)).contains(&day) {
                return Err(PyError::value_error("day is out of range for month"));
            }
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("year", py_int(year));
                dict.insert_str("month", py_int(month));
                dict.insert_str("day", py_int(day));
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "isoformat",
        bf!("isoformat", |args| {
            Ok(py_str(&format!(
                "{:04}-{:02}-{:02}",
                inst_get_i64(&args[0], "year"),
                inst_get_i64(&args[0], "month"),
                inst_get_i64(&args[0], "day")
            )))
        }),
    );
    // isocalendar() -> (ISO year, ISO week, ISO weekday)
    type_dict.insert_str(
        "isocalendar",
        bf!("isocalendar", |args| {
            let year = inst_get_i64(&args[0], "year");
            let month = inst_get_i64(&args[0], "month");
            let day = inst_get_i64(&args[0], "day");
            // ISO weekday: 1=Monday ... 7=Sunday (1-indexed)
            let wday = weekday_from_ordinal(date_ordinal(&args[0])) + 1;
            // ISO week number — use the date's ordinal directly
            // (same system as weekday_from_ordinal)
            let leap = if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 { 1 } else { 0 };
            let days_in_month = [0i64, 31, 28 + leap, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let mut yday = day;
            for m in 1..month {
                yday += days_in_month[m as usize];
            }
            // Find ordinal for Jan 1 of this year
            let jan1_ord = ymd_to_ordinal(year, 1, 1);
            // Find ordinal for Jan 4 of this year (ISO week 1 contains Jan 4)
            let jan4_ord = ymd_to_ordinal(year, 1, 4);
            // Day of week for Jan 4 (0=Monday..6=Sunday) using same weekday function
            let jan4_wday = weekday_from_ordinal(jan4_ord); // 0-indexed
            // Monday of ISO week 1 is Jan 4 minus its weekday offset
            let week1_monday = jan4_ord - jan4_wday;
            // Current date ordinal
            let cur_ord = date_ordinal(&args[0]);
            // Week number (approximate; boundary-corrected below)
            let approx_week = if cur_ord < week1_monday {
                0
            } else {
                (cur_ord - week1_monday) / 7 + 1
            };
            let (iso_year, week_of_year) = iso_week_corrected(year, approx_week);
            Ok(make_isocalendar(iso_year, week_of_year, wday))
        }),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| {
            Ok(py_str(&format!(
                "{:04}-{:02}-{:02}",
                inst_get_i64(&args[0], "year"),
                inst_get_i64(&args[0], "month"),
                inst_get_i64(&args[0], "day")
            )))
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            Ok(py_str(&format!(
                "datetime.date({}, {}, {})",
                inst_get_i64(&args[0], "year"),
                inst_get_i64(&args[0], "month"),
                inst_get_i64(&args[0], "day")
            )))
        }),
    );
    type_dict.insert_str(
        "weekday",
        bf!("weekday", |args| Ok(py_int(weekday_from_ordinal(
            date_ordinal(&args[0])
        )))),
    );
    type_dict.insert_str(
        "isoweekday",
        bf!("isoweekday", |args| Ok(py_int(
            weekday_from_ordinal(date_ordinal(&args[0])) + 1
        ))),
    );
    type_dict.insert_str(
        "toordinal",
        bf!("toordinal", |args| Ok(py_int(date_ordinal(&args[0])))),
    );
    type_dict.insert_str(
        "timetuple",
        bf!("timetuple", |args| {
            let ord = date_ordinal(&args[0]);
            let year = inst_get_i64(&args[0], "year");
            let wday = weekday_from_ordinal(ord);
            let yday = day_of_year(year, ord);
            Ok(py_tuple(vec![
                py_int(year),
                py_int(inst_get_i64(&args[0], "month")),
                py_int(inst_get_i64(&args[0], "day")),
                py_int(0),
                py_int(0),
                py_int(0),
                py_int(wday),
                py_int(yday + 1),
                py_int(-1),
            ]))
        }),
    );
    type_dict.insert_str(
        "strftime",
        bf!("strftime", |args| {
            let fmt = if args.len() > 1 {
                args[1].str()
            } else {
                "%Y-%m-%d".to_string()
            };
            let ord = date_ordinal(&args[0]);
            let year = inst_get_i64(&args[0], "year");
            Ok(py_str(&format_strftime(
                &fmt,
                year,
                inst_get_i64(&args[0], "month"),
                inst_get_i64(&args[0], "day"),
                0,
                0,
                0,
                weekday_from_ordinal(ord),
                day_of_year(year, ord),
            )))
        }),
    );
    // ctime() for date objects — returns "Mon Jan 01 00:00:00 2024"
    type_dict.insert_str(
        "ctime",
        bf!("ctime", |args| {
            let year = inst_get_i64(&args[0], "year");
            let month = inst_get_i64(&args[0], "month");
            let day = inst_get_i64(&args[0], "day");
            let ord = date_ordinal(&args[0]);
            let wd = weekday_from_ordinal(ord);
            let day_name = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][wd as usize % 7];
            let month_name = [
                "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                "Dec",
            ][month as usize];
            Ok(py_str(&format!(
                "{} {} {:02} 00:00:00 {}",
                day_name, month_name, day, year
            )))
        }),
    );
    // __format__ for date objects — supports strftime-style format specs
    type_dict.insert_str(
        "__format__",
        bf!("__format__", |args| {
            let fmt = if args.len() > 1 {
                args[1].str()
            } else {
                "%Y-%m-%d".to_string()
            };
            let ord = date_ordinal(&args[0]);
            let year = inst_get_i64(&args[0], "year");
            Ok(py_str(&format_strftime(
                &fmt,
                year,
                inst_get_i64(&args[0], "month"),
                inst_get_i64(&args[0], "day"),
                0,
                0,
                0,
                weekday_from_ordinal(ord),
                day_of_year(year, ord),
            )))
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
            Ok(make_date(year, month, day))
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            if instance_type_name(&args[1]) != "date" {
                return Ok(py_bool(false));
            }
            Ok(py_bool(date_ordinal(&args[0]) == date_ordinal(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__lt__",
        bf!("__lt__", |args| {
            if instance_type_name(&args[1]) != "date" {
                return Err(PyError::type_error(
                    "'<' not supported between instances of 'date' and other type",
                ));
            }
            Ok(py_bool(date_ordinal(&args[0]) < date_ordinal(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__le__",
        bf!("__le__", |args| {
            if instance_type_name(&args[1]) != "date" {
                return Err(PyError::type_error(
                    "'<=' not supported between instances of 'date' and other type",
                ));
            }
            Ok(py_bool(date_ordinal(&args[0]) <= date_ordinal(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__gt__",
        bf!("__gt__", |args| {
            if instance_type_name(&args[1]) != "date" {
                return Err(PyError::type_error(
                    "'>' not supported between instances of 'date' and other type",
                ));
            }
            Ok(py_bool(date_ordinal(&args[0]) > date_ordinal(&args[1])))
        }),
    );
    type_dict.insert_str(
        "__ge__",
        bf!("__ge__", |args| {
            if instance_type_name(&args[1]) != "date" {
                return Err(PyError::type_error(
                    "'>=' not supported between instances of 'date' and other type",
                ));
            }
            Ok(py_bool(date_ordinal(&args[0]) >= date_ordinal(&args[1])))
        }),
    );
    // date.__hash__: CPython hashes the packed 4-byte representation
    // [year>>8, year, month, day] with the seeded str/bytes hash.
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            let y = inst_get_i64(&args[0], "year");
            let m = inst_get_i64(&args[0], "month");
            let d = inst_get_i64(&args[0], "day");
            let bytes = [(y >> 8) as u8, (y & 0xff) as u8, m as u8, d as u8];
            Ok(py_int(crate::object::py_hash_bytes(&bytes) as i64))
        }),
    );
    type_dict.insert_str(
        "__add__",
        bf!("__add__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for +: 'date' and other type",
                ));
            }
            Ok(make_date_from_ordinal(
                date_ordinal(&args[0]) + inst_get_i64(&args[1], "days"),
            ))
        }),
    );
    type_dict.insert_str(
        "__radd__",
        bf!("__radd__", |args| {
            if instance_type_name(&args[1]) != "timedelta" {
                return Err(PyError::type_error(
                    "unsupported operand type(s) for +: 'date' and other type",
                ));
            }
            Ok(make_date_from_ordinal(
                date_ordinal(&args[0]) + inst_get_i64(&args[1], "days"),
            ))
        }),
    );
    type_dict.insert_str(
        "__sub__",
        bf!("__sub__", |args| {
            match instance_type_name(&args[1]).as_str() {
                "timedelta" => Ok(make_date_from_ordinal(
                    date_ordinal(&args[0]) - inst_get_i64(&args[1], "days"),
                )),
                "date" => Ok(make_timedelta(
                    date_ordinal(&args[0]) - date_ordinal(&args[1]),
                    0,
                    0,
                )),
                _ => Err(PyError::type_error(
                    "unsupported operand type(s) for -: 'date' and other type",
                )),
            }
        }),
    );
    type_dict.insert_str(
        "today",
        bf!("today", |_args| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let (y, m, d, _, _, _, _, _) = epoch_to_ymd(now.as_secs() as i64);
            Ok(make_date(y, m, d))
        }),
    );
    type_dict.insert_str(
        "fromordinal",
        bf!("fromordinal", |args| {
            let n = if !args.is_empty() {
                args[0].as_i64().unwrap_or(1)
            } else {
                1
            };
            Ok(make_date_from_ordinal(n))
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
            let (y, m, d, _, _, _, _, _) = epoch_to_ymd(ts as i64);
            Ok(make_date(y, m, d))
        }),
    );
    type_dict.insert_str(
        "fromisoformat",
        bf!("fromisoformat", |args| {
            let bad = || PyError::value_error("Invalid isoformat string");
            let s = if args.is_empty() {
                String::new()
            } else if let PyObject::Bytes(b) = &*args[0].borrow() {
                // CPython accepts ASCII/UTF-8 bytes too.
                match std::str::from_utf8(&b.iter().map(|x| *x as u8).collect::<Vec<u8>>()) {
                    Ok(txt) => txt.to_string(),
                    Err(_) => return Err(bad()),
                }
            } else {
                args[0].str()
            };
            // CPython 3.11+-style ISO 8601 parser: most formats the stdlib
            // accepts — basic/extended dates, ISO weeks, ordinal dates,
            // arbitrary single-char date/time separator, optional fractional
            // seconds (truncated to microseconds), and 'Z'/±HH:MM[:SS] zones.
            let mut t = s.as_str();
            if t.is_empty() {
                {
                    if std::env::var("RPY_ISO_LOG").is_ok() {
                        eprintln!("ISO-REJ generic {:?}", s);
                    }
                    return Err(bad());
                }
            }
            // Trailing timezone: Z/z, or +/-HH[:]MM[:SS][.frac]
            let mut tz_off: Option<i64> = None;
            let bytes = t.as_bytes();
            if bytes[t.len() - 1] == b'Z' || bytes[t.len() - 1] == b'z' {
                tz_off = Some(0);
                t = &t[..t.len() - 1];
            } else {
                // Search for the last '+' or '-' that starts a valid zone.
                let idx = t.rfind(|c| c == '+' || c == '-');
                if let Some(pos) = idx {
                    if pos >= 9 {
                        let zone = &t[pos..];
                        let zc: String =
                            zone.chars().filter(|c| *c != ':').collect();
                        let zbody = &zc[1..];
                        let neg = zc.starts_with('-');
                        let digits: Vec<&str> = if zc.contains('.') {
                            let dot = zbody.find('.').unwrap();
                            let intpart = &zbody[..dot];
                            let _frac = &zbody[dot + 1..];
                            // fractional offsets are accepted but truncated
                            vec![&intpart[0..intpart.len().min(6)], ""]
                        } else {
                            match zbody.len() {
                                2 => vec![&zbody[0..2], "0000"],
                                4 => vec![&zbody[0..2], &zbody[2..4]],
                                6 => vec![&zbody[0..2], &zbody[2..4], &zbody[4..6]],
                                _ => vec!["", ""],
                            }
                        };
                        if !digits[0].is_empty()
                            && digits[0].chars().all(|c| c.is_ascii_digit())
                            && digits.get(1).map(|d| {
                                d.chars().all(|c| c.is_ascii_digit()) || d.is_empty()
                            }) == Some(true)
                        {
                            let hh: i64 = digits[0].parse().unwrap_or(99);
                            let mm: i64 = if digits[1].len() >= 2 {
                                digits[1][..2].parse().unwrap_or(99)
                            } else {
                                0
                            };
                            if hh <= 24 && mm < 60 {
                                let total = hh * 3600 + mm * 60;
                                tz_off = Some(if neg { -total } else { total });
                                t = &t[..pos];
                            }
                        }
                    }
                }
            }
            // Split date / time at first separator (any single non-digit,
            // non-'-' char per 3.11 relaxation; T/t/space are the real ones).
            // 3.11 relaxation: ANY single character may separate date and
            // time. The separator is the first non-digit/non-'-'/'+' char
            // after a plausible date prefix; what follows must parse as a
            // time (validated below anyway).
            let (date_s, time_s) = 'found: {
                // Extended date YYYY-MM-DD ends at index 10.
                for (i, ch) in t.char_indices() {
                    if i >= 8 && !ch.is_ascii_digit() {
                        break 'found (&t[..i], &t[i + ch.len_utf8()..]);
                    }
                }
                (t, "")
            };

            fn parse_date(ds: &str) -> Option<(i64, i64, i64)> {
                let ds = ds.trim();
                if std::env::var("RPY_DBG_ISO").is_ok() { eprintln!("parse_date({})", ds); }
                // Extended YYYY-MM-DD
                if ds.len() == 10 && ds.as_bytes()[4] == b'-' && ds.as_bytes()[7] == b'-' {
                    let (y, m, d) = (
                        ds.get(0..4)?.parse::<i64>().ok()?,
                        ds.get(5..7)?.parse::<i64>().ok()?,
                        ds.get(8..10)?.parse::<i64>().ok()?,
                    );
                    return Some((y, m, d));
                }
                // Basic YYYYMMDD
                if ds.len() == 8 && ds.chars().all(|c| c.is_ascii_digit()) {
                    return Some((
                        ds.get(0..4)?.parse().ok()?,
                        ds.get(4..6)?.parse().ok()?,
                        ds.get(6..8)?.parse().ok()?,
                    ));
                }
                // Ordinal YYYY-DDD / YYYYDDD
                let compact: String = ds.chars().filter(|c| *c != '-').collect();
                if compact.len() == 7 && compact.chars().all(|c| c.is_ascii_digit())
                    && !compact.contains(['W', 'w'])
                {
                    let y: i64 = compact.get(0..4)?.parse().ok()?;
                    let doy: i64 = compact.get(4..7)?.parse().ok()?;
                    if !(1..=366).contains(&doy) {
                        return None;
                    }
                    let jan1 = ymd_to_ordinal(y, 1, 1);
                    return {
                        let ord = jan1 + doy - 1;
                        // invert via the existing helpers
                        Some(ordinal_to_ymd(ord))
                    };
                }
                // ISO week YYYY-Www-D / YYYYWwwD
                let w_pos = compact.find(|c: char| c == 'W' || c == 'w');
                if let Some(wp) = w_pos {
                    if wp == 4 && (compact.len() == 7 || compact.len() == 8) {
                        let y: i64 = compact.get(0..4)?.parse().ok()?;
                        let wk: i64 = compact.get(5..7)?.parse().ok()?;
                        let wd: i64 = if compact.len() == 8 {
                            compact.get(7..8)?.parse().ok()?
                        } else {
                            1
                        };
                        if !(1..=53).contains(&wk) || !(1..=7).contains(&wd) {
                            return None;
                        }
                        let jan4_ord = ymd_to_ordinal(y, 1, 4);
                        let jan4_wd = ((jan4_ord % 7) + 6) % 7;
                        let week1_monday = jan4_ord - jan4_wd;
                        let ord = week1_monday + (wk - 1) * 7 + (wd - 1);
                        return Some(ordinal_to_ymd(ord));
                    }
                }
                None
            }

            let (y, mo, d) = match parse_date(date_s) {
                Some(v) => v,
                None => {
                    if std::env::var("RPY_ISO_LOG").is_ok() {
                        eprintln!("ISO-REJ date of {:?}", s);
                    }
                    {
                    if std::env::var("RPY_ISO_LOG").is_ok() {
                        eprintln!("ISO-REJ generic {:?}", s);
                    }
                    return Err(bad());
                }
                }
            };
            if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
                {
                    if std::env::var("RPY_ISO_LOG").is_ok() {
                        eprintln!("ISO-REJ generic {:?}", s);
                    }
                    return Err(bad());
                }
            }

            let (mut hh, mut mi, mut ss, mut us) = (0i64, 0i64, 0i64, 0i64);
            if !time_s.is_empty() {
                // Zone already stripped above; reject embedded zones here.
                let tp = time_s;
                let (tp, frac) = match tp.find(|c| c == '.' || c == ',') {
                    Some(dot) => (&tp[..dot], &tp[dot + 1..]),
                    None => (tp, ""),
                };
                let digits: String = tp.chars().filter(|&c| c != ':').collect();
                if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                    {
                    if std::env::var("RPY_ISO_LOG").is_ok() {
                        eprintln!("ISO-REJ generic {:?}", s);
                    }
                    return Err(bad());
                }
                }
                if std::env::var("RPY_ISO_LOG").is_ok() && digits.is_empty() {
                    eprintln!("ISO-REJ empty time of {:?}", s);
                }
                match digits.len() {
                    2 => hh = digits.parse().unwrap_or(99),
                    4 => {
                        hh = digits.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(99);
                        mi = digits.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(99);
                    }
                    6 => {
                        hh = digits.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(99);
                        mi = digits.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(99);
                        ss = digits.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(99);
                    }
                    _ => return Err(bad()),
                }
                if hh > 23 || mi > 59 || ss > 59 {
                    {
                    if std::env::var("RPY_ISO_LOG").is_ok() {
                        eprintln!("ISO-REJ generic {:?}", s);
                    }
                    return Err(bad());
                }
                }
                if !frac.is_empty() {
                    if !frac.chars().all(|c| c.is_ascii_digit()) {
                        {
                    if std::env::var("RPY_ISO_LOG").is_ok() {
                        eprintln!("ISO-REJ generic {:?}", s);
                    }
                    return Err(bad());
                }
                    }
                    let f6: String = frac.chars().take(6).collect();
                    let padded = format!("{:<06}", f6);
                    us = padded.parse().unwrap_or(0);
                }
            }

            if let Some(off) = tz_off {
                let tz = if off == 0 {
                    get_utc_singleton()
                } else {
                    make_timezone(off, None)
                };
                return Ok(make_datetime(y, mo, d, hh, mi, ss, us, tz, 0));
            }
            Ok(make_datetime(y, mo, d, hh, mi, ss, us, py_none(), 0))
        }),
    );
    type_dict.insert_str(
        "fromisocalendar",
        bf!("fromisocalendar", |args| {
            if args.len() < 3 {
                return Err(PyError::type_error(
                    "fromisocalendar() missing required arguments: 'year', 'week', 'weekday'",
                ));
            }
            let year = args[0]
                .as_i64()
                .ok_or_else(|| PyError::type_error("an integer is required for 'year'"))?;
            let week = args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("an integer is required for 'week'"))?;
            let weekday = args[2]
                .as_i64()
                .ok_or_else(|| PyError::type_error("an integer is required for 'weekday'"))?;
            if !(1..=9999).contains(&year) {
                return Err(PyError::value_error("year is out of range"));
            }
            if !(1..=53).contains(&week) {
                return Err(PyError::value_error("week is out of range"));
            }
            if !(1..=7).contains(&weekday) {
                return Err(PyError::value_error("weekday is out of range"));
            }
            // The Monday of week 1 is the first Monday on or before Jan 4.
            // Jan 4 is always in week 1.
            let jan4_ord = ymd_to_ordinal(year, 1, 4);
            // Convert ordinal day-of-week: 0=Monday..6=Sunday
            let jan4_weekday = ((jan4_ord % 7) + 6) % 7;
            let week1_monday = jan4_ord - jan4_weekday;
            let target_ord = week1_monday + (week - 1) * 7 + (weekday - 1);

            // Compute the ISO year of the target date.
            // The ISO year is the year containing the Thursday of the ISO week,
            // which equals the calendar year unless the date falls in the
            // trailing days of the previous/leading days of the next year's
            // week 1.
            let (result_y, _result_m, _result_d) = ordinal_to_ymd(target_ord);
            let result_jan4 = ymd_to_ordinal(result_y, 1, 4);
            let result_jan4_wd = ((result_jan4 % 7) + 6) % 7;
            let result_w1_monday = result_jan4 - result_jan4_wd;
            let iso_year = if target_ord < result_w1_monday {
                result_y - 1
            } else {
                let next_jan4 = ymd_to_ordinal(result_y + 1, 1, 4);
                let next_jan4_wd = ((next_jan4 % 7) + 6) % 7;
                let next_w1_monday = next_jan4 - next_jan4_wd;
                if target_ord >= next_w1_monday {
                    result_y + 1
                } else {
                    result_y
                }
            };

            if iso_year != year {
                return Err(PyError::value_error(&format!("Invalid week: {}", week)));
            }
            if !(1..=9999).contains(&result_y) {
                return Err(PyError::value_error(&format!(
                    "year must be in 1..9999, not {}",
                    result_y
                )));
            }
            Ok(make_date_from_ordinal(target_ord))
        }),
    );

    PyObjectRef::new(PyObject::Type {
        name: "date".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub(crate) fn make_date(year: i64, month: i64, day: i64) -> PyObjectRef {
    let typ = get_date_type();
    let mut dict = AttrMap::new();
    dict.insert_str("year", py_int(year));
    dict.insert_str("month", py_int(month));
    dict.insert_str("day", py_int(day));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub(crate) fn get_date_type() -> PyObjectRef {
    let existing = DATE_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_date_type();
    let min_inst = PyObjectRef::new(PyObject::Instance {
        typ: typ.clone(),
        dict: AttrMap::from([
            ("year".to_string(), py_int(1)),
            ("month".to_string(), py_int(1)),
            ("day".to_string(), py_int(1)),
        ]),
    });
    let max_inst = PyObjectRef::new(PyObject::Instance {
        typ: typ.clone(),
        dict: AttrMap::from([
            ("year".to_string(), py_int(9999)),
            ("month".to_string(), py_int(12)),
            ("day".to_string(), py_int(31)),
        ]),
    });
    let res_inst = make_timedelta(1, 0, 0);
    if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
        dict.insert_str("min", min_inst);
        dict.insert_str("max", max_inst);
        dict.insert_str("resolution", res_inst);
    }
    DATE_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

