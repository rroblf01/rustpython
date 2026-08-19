use crate::object::*;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Convert seconds since epoch to (year, month, day, hour, minute, second, weekday, yearday)
fn epoch_to_ymd(secs: i64) -> (i64, i64, i64, i64, i64, i64, i64, i64) {
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hour = time_secs / 3600;
    let minute = (time_secs % 3600) / 60;
    let second = time_secs % 60;

    // Days to year/month/day (algorithm from Howard Hinnant)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    // Weekday (0=Mon, 6=Sun) and yearday (0-365)
    let wday = (days + 3) % 7;
    let yday = if m > 1 {
        let month_days = [
            31,
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        month_days[..(m as usize - 1)].iter().sum::<i64>() + d - 1
    } else {
        d - 1
    };

    (y, m, d, hour, minute, second, wday, yday)
}

const STRUCT_TIME_FIELDS: [&str; 9] = [
    "tm_year", "tm_mon", "tm_mday", "tm_hour", "tm_min", "tm_sec", "tm_wday", "tm_yday", "tm_isdst",
];

thread_local! {
    static STRUCT_TIME_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

fn build_struct_time_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    // `time.struct_time` is a real CPython "structseq" — simultaneously
    // index-accessible like a plain 9-tuple (`t[0]`) AND attribute-
    // accessible by name (`t.tm_year`) — real code uses BOTH forms
    // interchangeably. The previous stub was a bare passthrough
    // (`struct_time(x) -> x`), so anything built from it was just a plain
    // tuple with NO attribute access at all (`t.tm_year` raised
    // `AttributeError: 'tuple' object has no attribute 'tm_year'`).
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
                let i = if idx < 0 { idx + 9 } else { idx };
                let name = STRUCT_TIME_FIELDS
                    .get(i as usize)
                    .ok_or_else(|| PyError::index_error("struct_time index out of range"))?;
                Ok(dict.get(name).cloned().unwrap_or_else(py_none))
            } else {
                Err(PyError::runtime_error("__getitem__ on non-struct_time"))
            }
        }),
    );
    type_dict.insert_str("__len__", bf!("__len__", |_args| Ok(py_int(9))));
    type_dict.insert_str(
        "__iter__",
        bf!("__iter__", |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let items: Vec<PyObjectRef> = STRUCT_TIME_FIELDS
                    .iter()
                    .map(|f| dict.get(f).cloned().unwrap_or_else(py_none))
                    .collect();
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: items,
                    index: 0,
                }))
            } else {
                Err(PyError::runtime_error("__iter__ on non-struct_time"))
            }
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let body = STRUCT_TIME_FIELDS
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
                Ok(py_str(&format!("time.struct_time({})", body)))
            } else {
                Ok(py_str("time.struct_time(...)"))
            }
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "time.struct_time".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn get_struct_time_type() -> PyObjectRef {
    let existing = STRUCT_TIME_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_struct_time_type();
    STRUCT_TIME_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

#[allow(clippy::too_many_arguments)]
fn make_struct_time(
    y: i64,
    mon: i64,
    mday: i64,
    h: i64,
    min: i64,
    s: i64,
    wday: i64,
    yday: i64,
    isdst: i64,
) -> PyObjectRef {
    let mut dict = AttrMap::new();
    dict.insert_str("tm_year", py_int(y));
    dict.insert_str("tm_mon", py_int(mon));
    dict.insert_str("tm_mday", py_int(mday));
    dict.insert_str("tm_hour", py_int(h));
    dict.insert_str("tm_min", py_int(min));
    dict.insert_str("tm_sec", py_int(s));
    dict.insert_str("tm_wday", py_int(wday));
    dict.insert_str("tm_yday", py_int(yday + 1));
    dict.insert_str("tm_isdst", py_int(isdst));
    PyObjectRef::new(PyObject::Instance {
        typ: get_struct_time_type(),
        dict,
    })
}

/// Days since 0000-03-01 for a given (year, 1-indexed month, day) — inverse
/// of `epoch_to_ymd`'s own Howard Hinnant `days_from_civil` algorithm, used
/// to derive `tm_wday`/`tm_yday` for a caller-supplied date (`strptime`,
/// `mktime`-adjacent code) where a real epoch-seconds value isn't
/// necessarily available yet.
fn civil_to_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn weekday_yday_for(y: i64, m: i64, d: i64) -> (i64, i64) {
    let days = civil_to_days(y, m, d);
    let wday = ((days + 3) % 7 + 7) % 7;
    let jan1 = civil_to_days(y, 1, 1);
    let yday = days - jan1;
    (wday, yday)
}

fn format_strftime(
    fmt: &str,
    y: i64,
    m: i64,
    d: i64,
    h: i64,
    min: i64,
    s: i64,
    wday: i64,
    yday: i64,
) -> String {
    let mut result = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('Y') => result.push_str(&format!("{:04}", y)),
                Some('y') => result.push_str(&format!("{:02}", y % 100)),
                Some('m') => result.push_str(&format!("{:02}", m)),
                Some('d') => result.push_str(&format!("{:02}", d)),
                Some('H') => result.push_str(&format!("{:02}", h)),
                Some('M') => result.push_str(&format!("{:02}", min)),
                Some('S') => result.push_str(&format!("{:02}", s)),
                // `%I` (12-hour, 01-12) and `%p` (AM/PM) — `test_strftime.py`'s
                // FATAL strftest1 list checks both (an unsupported standard
                // directive that comes back unchanged starting with '%' is a
                // hard failure, not a soft one).
                Some('I') => {
                    let ih = h % 12;
                    result.push_str(&format!("{:02}", if ih == 0 { 12 } else { ih }));
                }
                Some('p') => result.push_str(if h < 12 { "AM" } else { "PM" }),
                Some('j') => result.push_str(&format!("{:03}", yday + 1)),
                Some('w') => result.push_str(&format!("{}", (wday + 1) % 7)),
                Some('u') => result.push_str(&format!("{}", if wday == 0 { 7 } else { wday })),
                // `%U`/`%W` — week numbers (Sunday-first / Monday-first),
                // matching the exact formula `test_strftime.py` itself uses
                // (`(tm_yday + jan1_tm_wday)//7` with the Monday-based
                // Python `tm_wday` convention, and its `%W` variant).
                Some('U') => {
                    let jan1_wday = weekday_yday_for(y, 1, 1).0;
                    result.push_str(&format!("{:02}", (yday + 1 + jan1_wday) / 7));
                }
                Some('W') => {
                    let jan1_wday = weekday_yday_for(y, 1, 1).0;
                    result.push_str(&format!(
                        "{:02}",
                        (yday + 1 + (jan1_wday - 1).rem_euclid(7)) / 7
                    ));
                }
                Some('Z') => { /* this interpreter models no timezone (localtime == gmtime == UTC) — emit nothing rather than the raw directive */
                }
                Some('e') => result.push_str(&format!("{:2}", d)),
                Some('k') => result.push_str(&format!("{:2}", h)),
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('R') => result.push_str(&format!("{:02}:{:02}", h, min)),
                Some('T') => result.push_str(&format!("{:02}:{:02}:{:02}", h, min, s)),
                Some('D') => result.push_str(&format!("{:02}/{:02}/{:02}", m, d, y % 100)),
                Some('r') => {
                    let ih = h % 12;
                    let ih = if ih == 0 { 12 } else { ih };
                    result.push_str(&format!(
                        "{:02}:{:02}:{:02} {}",
                        ih,
                        min,
                        s,
                        if h < 12 { "AM" } else { "PM" }
                    ));
                }
                Some('A') => {
                    let weekdays = [
                        "Monday",
                        "Tuesday",
                        "Wednesday",
                        "Thursday",
                        "Friday",
                        "Saturday",
                        "Sunday",
                    ];
                    result.push_str(weekdays[wday as usize]);
                }
                Some('a') => {
                    let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                    result.push_str(weekdays[wday as usize]);
                }
                Some('B') => {
                    let months = [
                        "January",
                        "February",
                        "March",
                        "April",
                        "May",
                        "June",
                        "July",
                        "August",
                        "September",
                        "October",
                        "November",
                        "December",
                    ];
                    result.push_str(months[(m - 1) as usize]);
                }
                Some('b') | Some('h') => {
                    let months = [
                        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                        "Nov", "Dec",
                    ];
                    result.push_str(months[(m - 1) as usize]);
                }
                Some('c') => result.push_str(&format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    y, m, d, h, min, s
                )),
                Some('x') => result.push_str(&format!("{:04}-{:02}-{:02}", y, m, d)),
                Some('X') => result.push_str(&format!("{:02}:{:02}:{:02}", h, min, s)),
                Some('%') => result.push('%'),
                Some(other) => {
                    result.push('%');
                    result.push(other);
                }
                None => result.push('%'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

const FULL_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const ABBR_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const FULL_WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const ABBR_WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A hand-written (not regex-based, unlike real CPython's own `_strptime.py`)
/// but genuinely FUNCTIONAL `strptime` parser covering the directives real
/// code overwhelmingly actually uses. Replaces a previous "stub" that
/// silently ignored BOTH its `data_string` and `format` arguments entirely,
/// always returning the CURRENT system time regardless of input — a severe,
/// silently-wrong-answer bug (not a crash) for one of the most common
/// date-parsing entry points in the whole standard library.
fn parse_strptime(data: &str, fmt: &str) -> Result<(i64, i64, i64, i64, i64, i64), String> {
    let mut year: i64 = 1900;
    let mut month: i64 = 1;
    let mut day: i64 = 1;
    let mut hour: i64 = 0;
    let mut minute: i64 = 0;
    let mut second: i64 = 0;
    let mut pm = false;
    let mut hour12_seen = false;

    let dbytes: Vec<char> = data.chars().collect();
    let mut di = 0usize;
    let mut fchars = fmt.chars().peekable();

    fn skip_ws(d: &[char], i: &mut usize) {
        while *i < d.len() && d[*i].is_whitespace() {
            *i += 1;
        }
    }
    fn read_digits(d: &[char], i: &mut usize, max: usize) -> Option<i64> {
        let start = *i;
        let mut n: usize = 0;
        while *i < d.len() && d[*i].is_ascii_digit() && n < max {
            *i += 1;
            n += 1;
        }
        if *i == start {
            return None;
        }
        d[start..*i].iter().collect::<String>().parse::<i64>().ok()
    }
    fn match_name<'a>(d: &[char], i: &mut usize, names: &'a [&'a str]) -> Option<usize> {
        let rest: String = d[*i..].iter().collect();
        let rest_lower = rest.to_lowercase();
        for (idx, name) in names.iter().enumerate() {
            if rest_lower.starts_with(&name.to_lowercase()) {
                *i += name.chars().count();
                return Some(idx);
            }
        }
        None
    }

    while let Some(fc) = fchars.next() {
        if fc == '%' {
            match fchars.next() {
                Some('Y') => {
                    year = read_digits(&dbytes, &mut di, 4).ok_or("bad year")?;
                }
                Some('y') => {
                    let yy = read_digits(&dbytes, &mut di, 2).ok_or("bad year")?;
                    year = if yy <= 68 { 2000 + yy } else { 1900 + yy };
                }
                Some('m') => {
                    month = read_digits(&dbytes, &mut di, 2).ok_or("bad month")?;
                }
                Some('d') => {
                    day = read_digits(&dbytes, &mut di, 2).ok_or("bad day")?;
                }
                Some('H') => {
                    hour = read_digits(&dbytes, &mut di, 2).ok_or("bad hour")?;
                }
                Some('I') => {
                    hour = read_digits(&dbytes, &mut di, 2).ok_or("bad hour")?;
                    hour12_seen = true;
                }
                Some('M') => {
                    minute = read_digits(&dbytes, &mut di, 2).ok_or("bad minute")?;
                }
                Some('S') => {
                    second = read_digits(&dbytes, &mut di, 2).ok_or("bad second")?;
                }
                Some('f') => {
                    read_digits(&dbytes, &mut di, 6);
                }
                Some('j') => {
                    read_digits(&dbytes, &mut di, 3).ok_or("bad yday")?;
                }
                Some('p') => {
                    let rest: String = dbytes[di..].iter().collect();
                    let upper = rest.to_uppercase();
                    if upper.starts_with("PM") {
                        pm = true;
                        di += 2;
                    } else if upper.starts_with("AM") {
                        di += 2;
                    }
                }
                Some('B') => {
                    month = 1 + match_name(&dbytes, &mut di, &FULL_MONTHS)
                        .ok_or("bad month name")? as i64;
                }
                Some('b') | Some('h') => {
                    month = 1 + match_name(&dbytes, &mut di, &ABBR_MONTHS)
                        .ok_or("bad month name")? as i64;
                }
                Some('A') => {
                    match_name(&dbytes, &mut di, &FULL_WEEKDAYS).ok_or("bad weekday name")?;
                }
                Some('a') => {
                    match_name(&dbytes, &mut di, &ABBR_WEEKDAYS).ok_or("bad weekday name")?;
                }
                Some('z') => {
                    if di < dbytes.len() && (dbytes[di] == '+' || dbytes[di] == '-') {
                        di += 1;
                        read_digits(&dbytes, &mut di, 2);
                        if di < dbytes.len() && dbytes[di] == ':' {
                            di += 1;
                        }
                        read_digits(&dbytes, &mut di, 2);
                    }
                }
                Some('Z') => {
                    while di < dbytes.len()
                        && (dbytes[di].is_alphabetic()
                            || dbytes[di] == '+'
                            || dbytes[di] == '-'
                            || dbytes[di].is_ascii_digit()
                            || dbytes[di] == ':')
                    {
                        di += 1;
                    }
                }
                Some('%') => {
                    if di < dbytes.len() && dbytes[di] == '%' {
                        di += 1;
                    } else {
                        return Err("expected '%'".to_string());
                    }
                }
                Some(other) => return Err(format!("unsupported strptime directive %{}", other)),
                None => return Err("trailing '%' in format".to_string()),
            }
        } else if fc.is_whitespace() {
            skip_ws(&dbytes, &mut di);
        } else {
            if di >= dbytes.len() || dbytes[di] != fc {
                return Err(format!(
                    "time data does not match format (expected '{}')",
                    fc
                ));
            }
            di += 1;
        }
    }
    if hour12_seen && pm && hour < 12 {
        hour += 12;
    }
    if hour12_seen && !pm && hour == 12 {
        hour = 0;
    }
    Ok((year, month, day, hour, minute, second))
}

pub fn create_time_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    macro_rules! time_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // time() -> float seconds since epoch
    time_func!("time", |_args| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0));
        let secs = now.as_secs() as f64 + now.subsec_nanos() as f64 * 1e-9;
        Ok(py_float(secs))
    });

    // sleep(seconds) — busy-wait (simplified)
    time_func!("sleep", |args| {
        // Convert through the full float protocol (__float__/__index__/
        // errors), so sleep(FloatLike("")) raises TypeError like CPython
        // instead of silently sleeping 0 seconds.
        let secs = if args.is_empty() {
            0.0
        } else {
            crate::object::builtin_float(&[args[0].clone()])?
                .as_f64()
                .unwrap_or(0.0)
        };
        let nanos = (secs * 1e9) as u64;
        let start = SystemTime::now();
        loop {
            if let Ok(elapsed) = SystemTime::now().duration_since(start) {
                if elapsed.as_nanos() >= nanos as u128 {
                    break;
                }
            }
        }
        Ok(py_none())
    });

    // monotonic() — monotonic clock in seconds
    time_func!("monotonic", |_args| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0));
        Ok(py_float(now.as_secs_f64()))
    });

    // get_clock_info(name) -> namespace with clock info attributes
    time_func!("get_clock_info", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "get_clock_info() missing required argument",
            ));
        }
        let name = args[0].str();
        let implementation = match name.as_str() {
            "monotonic" => py_str("clock_gettime(CLOCK_MONOTONIC)"),
            "perf_counter" => py_str("clock_gettime(CLOCK_MONOTONIC)"),
            "time" => py_str("gettimeofday()"),
            "process_time" => py_str("clock_gettime(CLOCK_PROCESS_CPUTIME_ID)"),
            "thread_time" => py_str("clock_gettime(CLOCK_THREAD_CPUTIME_ID)"),
            _ => py_str("clock_gettime"),
        };
        let mut dict = AttrMap::new();
        dict.insert_str("implementation", implementation);
        dict.insert_str(
            "monotonic",
            py_bool(name == "monotonic" || name == "perf_counter"),
        );
        dict.insert_str("adjustable", py_bool(false));
        dict.insert_str("resolution", py_float(1e-9));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "namespace".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict,
        }))
    });

    // gmtime(secs=None) -> struct_time
    time_func!("gmtime", |args| {
        let secs = if !args.is_empty() {
            args[0]
                .as_i64()
                .or_else(|| args[0].as_f64().map(|f| f as i64))
                .unwrap_or(0)
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        };
        let (y, m, d, h, min, s, wday, yday) = epoch_to_ymd(secs);
        Ok(make_struct_time(y, m, d, h, min, s, wday, yday, 0))
    });

    // localtime(secs=None) -> struct_time — accepts int OR float seconds
    // (real CPython does; `time.mktime` returns a float, so round-tripping
    // `localtime(mktime(t))` must not silently coerce a float to 0).
    time_func!("localtime", |args| {
        let secs = if !args.is_empty() {
            args[0]
                .as_i64()
                .or_else(|| args[0].as_f64().map(|f| f as i64))
                .unwrap_or(0)
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        };
        let (y, m, d, h, min, s, wday, yday) = epoch_to_ymd(secs);
        Ok(make_struct_time(y, m, d, h, min, s, wday, yday, 0))
    });

    // mktime(t) -> float — inverse of localtime(): interpret the caller's
    // date fields as this interpreter's own timezone (UTC — the only one it
    // models; localtime/gmtime are identical here) and return epoch seconds.
    // Was missing entirely (`AttributeError`), breaking `test_strftime.py`'s
    // `_update_variables` (`time.mktime(jan1)` to derive tm_wday/tm_yday for
    // `%U`/`%W` week-number computations) and any caller that round-trips a
    // struct_time through epoch seconds. Real CPython additionally NORMALIZES
    // out-of-range fields (tm_mday=32 rolls into next month); not modeled
    // here — the common case passes valid dates.
    time_func!("mktime", |args| {
        let t = args
            .first()
            .ok_or_else(|| PyError::type_error("mktime() missing required argument"))?;
        let get = |field: &str, idx: usize| -> i64 {
            match &*t.borrow() {
                PyObject::Instance { dict, .. } => {
                    dict.get(field).and_then(|v| v.as_i64()).unwrap_or(0)
                }
                PyObject::Tuple(items) => items.get(idx).and_then(|v| v.as_i64()).unwrap_or(0),
                _ => 0,
            }
        };
        let (y, mo, d) = (get("tm_year", 0), get("tm_mon", 1), get("tm_mday", 2));
        let (h, mi, s) = (get("tm_hour", 3), get("tm_min", 4), get("tm_sec", 5));
        let days = civil_to_days(y, mo, d);
        let secs = days * 86400 + h * 3600 + mi * 60 + s;
        Ok(py_float(secs as f64))
    });

    // strftime(format, struct_time) -> string
    time_func!("strftime", |args| {
        let fmt = if args.len() > 0 {
            args[0].str()
        } else {
            "%c".to_string()
        };
        let (y, m, d, h, min, s, wday, yday) = if args.len() > 1 {
            let t = &args[1];
            let get = |field: &str, idx: usize| -> Option<i64> {
                match &*t.borrow() {
                    PyObject::Instance { dict, .. } => dict.get(field).and_then(|v| v.as_i64()),
                    PyObject::Tuple(items) => items.get(idx).and_then(|v| v.as_i64()),
                    _ => None,
                }
            };
            (
                get("tm_year", 0).unwrap_or(2025),
                get("tm_mon", 1).unwrap_or(1),
                get("tm_mday", 2).unwrap_or(1),
                get("tm_hour", 3).unwrap_or(0),
                get("tm_min", 4).unwrap_or(0),
                get("tm_sec", 5).unwrap_or(0),
                get("tm_wday", 6).unwrap_or(0),
                // `%j`/internal yday math here is 0-indexed (matching
                // `epoch_to_ymd`'s own convention) but `tm_yday` on a real
                // struct_time is 1-indexed — convert back.
                get("tm_yday", 7).map(|v| v - 1).unwrap_or(0),
            )
        } else {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            epoch_to_ymd(now)
        };
        Ok(py_str(&format_strftime(
            &fmt, y, m, d, h, min, s, wday, yday,
        )))
    });

    // asctime(t=None) -> str — real CPython's classic fixed-width
    // "Sun Jun 20 23:21:05 1993" layout (`%a %b %d %H:%M:%S %Y`, with the
    // day space-padded, not zero-padded). Was missing entirely. The test
    // suite's `fixasctime` helper exists precisely because real asctime
    // space-pads single-digit days.
    time_func!("asctime", |args| {
        let (y, m, d, h, min, s, wday, _) = if let Some(t) = args.first() {
            let get = |field: &str, idx: usize| -> i64 {
                match &*t.borrow() {
                    PyObject::Instance { dict, .. } => {
                        dict.get(field).and_then(|v| v.as_i64()).unwrap_or(0)
                    }
                    PyObject::Tuple(items) => items.get(idx).and_then(|v| v.as_i64()).unwrap_or(0),
                    _ => 0,
                }
            };
            (
                get("tm_year", 0),
                get("tm_mon", 1),
                get("tm_mday", 2),
                get("tm_hour", 3),
                get("tm_min", 4),
                get("tm_sec", 5),
                get("tm_wday", 6),
                0,
            )
        } else {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            epoch_to_ymd(now)
        };
        let wdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let months = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let wd = wdays[((wday % 7 + 7) % 7) as usize];
        let mo = months[((((m - 1) % 12) + 12) % 12) as usize];
        Ok(py_str(&format!(
            "{} {} {:2} {:02}:{:02}:{:02} {:04}",
            wd, mo, d, h, min, s, y
        )))
    });

    // ctime(secs=None) -> str — real CPython's `asctime(localtime(secs))`.
    time_func!("ctime", |args| {
        let secs = if let Some(a) = args.first() {
            a.as_i64()
                .or_else(|| a.as_f64().map(|f| f as i64))
                .unwrap_or(0)
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        };
        let (y, m, d, h, min, s, wday, _) = epoch_to_ymd(secs);
        let wdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let months = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let wd = wdays[((wday % 7 + 7) % 7) as usize];
        let mo = months[((((m - 1) % 12) + 12) % 12) as usize];
        Ok(py_str(&format!(
            "{} {} {:2} {:02}:{:02}:{:02} {:04}",
            wd, mo, d, h, min, s, y
        )))
    });

    // strptime(string, format) -> struct_time
    time_func!("strptime", |args| {
        let string = if args.len() > 0 {
            args[0].str()
        } else {
            String::new()
        };
        let fmt = if args.len() > 1 {
            args[1].str()
        } else {
            "%a %b %d %H:%M:%S %Y".to_string()
        };
        let (y, m, d, h, min, s) = parse_strptime(&string, &fmt).map_err(PyError::value_error)?;
        let (wday, yday) = weekday_yday_for(y, m, d);
        Ok(make_struct_time(y, m, d, h, min, s, wday, yday, -1))
    });

    // perf_counter() -> float (high-resolution monotonic)
    time_func!("perf_counter", |_args| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0));
        Ok(py_float(now.as_secs_f64()))
    });

    // Constants
    d.insert_str("altzone", py_int(0));
    d.insert_str("daylight", py_int(0));
    d.insert_str("timezone", py_int(0));
    d.insert_str("tzname", py_tuple(vec![py_str("UTC"), py_str("UTC")]));

    // `time.struct_time(sequence)` — real CPython accepts any 9+-element
    // sequence and builds a real structseq from it (used directly by real
    // code — e.g. `_strptime.py`'s own `time.struct_time(tt[:9])`). The
    // previous stub just returned its argument completely unchanged (a
    // bare tuple, with none of the named-attribute access a real
    // struct_time provides).
    d.insert_str(
        "struct_time",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "struct_time".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "struct_time() takes at least 1 argument",
                    ));
                }
                let get = |i: usize| -> i64 {
                    match &*args[0].borrow() {
                        PyObject::Tuple(items) | PyObject::List(items) => {
                            items.get(i).and_then(|v| v.as_i64()).unwrap_or(0)
                        }
                        _ => 0,
                    }
                };
                Ok(make_struct_time(
                    get(0),
                    get(1),
                    get(2),
                    get(3),
                    get(4),
                    get(5),
                    get(6),
                    get(7).saturating_sub(1),
                    get(8),
                ))
            },
        }),
    );
    // Real CPython's `_strptime.py` reads `time._STRUCT_TM_ITEMS` (value 11
    // — 9 named fields + `tm_zone`/`tm_gmtoff`) to know how much of its own
    // internal working tuple to slice off before calling
    // `time.struct_time(...)`. This interpreter's struct_time only
    // implements the 9 core fields (no `tm_zone`/`tm_gmtoff`), but the
    // constant must still exist and be large enough that the slice
    // `tt[:11]` doesn't silently truncate real fields.
    d.insert_str("_STRUCT_TM_ITEMS", py_int(11));

    d
}

// ===================== Real datetime module =====================
//
// date/time/datetime/timedelta/timezone are implemented as native classes
// (PyObject::Type with a method dict of PyObject::BuiltinFunction entries,
// auto-bound to instances via LOAD_ATTR — the same mechanism ordinary
// dunder dispatch uses). Each class's Type object is built once and cached
// in a thread_local so all instances share the same type identity.
//
// Native methods here are plain `fn` pointers with no VM access, so they
// can't invoke an arbitrary user-defined tzinfo subclass's overridden
// utcoffset()/dst()/tzname(). Only the two tzinfo implementations provided
// here (`timezone` and `zoneinfo.ZoneInfo`) are understood natively; any
// other tzinfo object is treated as naive (unknown offset). This covers the
// overwhelming majority of real-world usage (fixed UTC offsets and IANA
// zone lookups) without needing a much larger descriptor-dispatch rework.

const EPOCH_ORDINAL: i64 = 719163; // ymd_to_ordinal(1970, 1, 1)

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const DAYS_IN_MONTH: [i64; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn days_in_month(year: i64, month: i64) -> i64 {
    if month == 2 && is_leap_year(year) {
        29
    } else {
        DAYS_IN_MONTH[month as usize]
    }
}

fn days_before_month_table() -> [i64; 13] {
    let mut t = [0i64; 13];
    let mut acc = 0;
    for m in 1..13 {
        t[m] = acc;
        acc += DAYS_IN_MONTH[m];
    }
    t
}

fn days_before_year(year: i64) -> i64 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

fn days_before_month(year: i64, month: i64) -> i64 {
    days_before_month_table()[month as usize]
        + if month > 2 && is_leap_year(year) {
            1
        } else {
            0
        }
}

fn ymd_to_ordinal(year: i64, month: i64, day: i64) -> i64 {
    days_before_year(year) + days_before_month(year, month) + day
}

/// Port of CPython's `datetime._ord2ymd` (proleptic Gregorian calendar).
fn ordinal_to_ymd(n_in: i64) -> (i64, i64, i64) {
    let t = days_before_month_table();
    let mut n = n_in - 1;
    let n400 = n.div_euclid(146097);
    n = n.rem_euclid(146097);
    let mut year = n400 * 400 + 1;
    let n100 = n.div_euclid(36524);
    n = n.rem_euclid(36524);
    let n4 = n.div_euclid(1461);
    n = n.rem_euclid(1461);
    let n1 = n.div_euclid(365);
    n = n.rem_euclid(365);
    year += n100 * 100 + n4 * 4 + n1;
    if n1 == 4 || n100 == 4 {
        return (year - 1, 12, 31);
    }
    let mut month = (n + 50) >> 5;
    let mut preceding = t[month as usize]
        + if month > 2 && is_leap_year(year) {
            1
        } else {
            0
        };
    if preceding > n {
        month -= 1;
        preceding -= days_in_month(year, month);
    }
    n -= preceding;
    (year, month, n + 1)
}

/// 0 = Monday .. 6 = Sunday, matching CPython's `date.weekday()`.
fn weekday_from_ordinal(ord: i64) -> i64 {
    (ord + 6).rem_euclid(7)
}

fn day_of_year(year: i64, ordinal: i64) -> i64 {
    ordinal - days_before_year(year) - 1
}

fn normalize_timedelta(days: i64, seconds: i64, microseconds: i64) -> (i64, i64, i64) {
    let extra_s = microseconds.div_euclid(1_000_000);
    let microseconds = microseconds.rem_euclid(1_000_000);
    let seconds = seconds + extra_s;
    let extra_d = seconds.div_euclid(86400);
    let seconds = seconds.rem_euclid(86400);
    let days = days + extra_d;
    (days, seconds, microseconds)
}

// ---- TZif (IANA time zone binary format, RFC 8536) parsing ----

struct ParsedTz {
    transitions: Vec<i64>,
    trans_types: Vec<u8>,
    ttinfos: Vec<(i32, bool, String)>, // (utc offset seconds, is_dst, designation)
}

fn read_i32_be(data: &[u8], pos: usize) -> i32 {
    i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
}

fn read_i64_be(data: &[u8], pos: usize) -> i64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[pos..pos + 8]);
    i64::from_be_bytes(b)
}

fn parse_tzif_block(data: &[u8], pos: usize, time_size: usize) -> Option<(ParsedTz, usize)> {
    if pos + 44 > data.len() {
        return None;
    }
    let isutcnt = read_i32_be(data, pos + 20) as usize;
    let isstdcnt = read_i32_be(data, pos + 24) as usize;
    let leapcnt = read_i32_be(data, pos + 28) as usize;
    let timecnt = read_i32_be(data, pos + 32) as usize;
    let typecnt = read_i32_be(data, pos + 36) as usize;
    let charcnt = read_i32_be(data, pos + 40) as usize;
    let mut p = pos + 44;

    let mut transitions = Vec::with_capacity(timecnt);
    for _ in 0..timecnt {
        if p + time_size > data.len() {
            return None;
        }
        let t = if time_size == 8 {
            read_i64_be(data, p)
        } else {
            read_i32_be(data, p) as i64
        };
        transitions.push(t);
        p += time_size;
    }
    let mut trans_types = Vec::with_capacity(timecnt);
    for _ in 0..timecnt {
        trans_types.push(*data.get(p)?);
        p += 1;
    }
    let mut ttinfo_raw = Vec::with_capacity(typecnt);
    for _ in 0..typecnt {
        if p + 6 > data.len() {
            return None;
        }
        let utoff = read_i32_be(data, p);
        let isdst = data[p + 4] != 0;
        let desigidx = data[p + 5] as usize;
        ttinfo_raw.push((utoff, isdst, desigidx));
        p += 6;
    }
    if p + charcnt > data.len() {
        return None;
    }
    let charpool = &data[p..p + charcnt];
    p += charcnt;
    p += leapcnt * (time_size + 4);
    p += isstdcnt;
    p += isutcnt;

    let ttinfos: Vec<(i32, bool, String)> = ttinfo_raw
        .into_iter()
        .map(|(utoff, isdst, idx)| {
            let desig = if idx < charpool.len() {
                let end = charpool[idx..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|o| idx + o)
                    .unwrap_or(charpool.len());
                String::from_utf8_lossy(&charpool[idx..end]).to_string()
            } else {
                String::new()
            };
            (utoff, isdst, desig)
        })
        .collect();

    Some((
        ParsedTz {
            transitions,
            trans_types,
            ttinfos,
        },
        p,
    ))
}

fn parse_tzif(bytes: &[u8]) -> Option<ParsedTz> {
    if bytes.len() < 44 || &bytes[0..4] != b"TZif" {
        return None;
    }
    let version = bytes[4];
    let (v1_result, next_pos) = parse_tzif_block(bytes, 0, 4)?;
    if version == 0 {
        return Some(v1_result);
    }
    if next_pos + 4 <= bytes.len() && &bytes[next_pos..next_pos + 4] == b"TZif" {
        if let Some((v2_result, _)) = parse_tzif_block(bytes, next_pos, 8) {
            return Some(v2_result);
        }
    }
    Some(v1_result)
}

fn tz_offset_for_instant(tz: &ParsedTz, instant: i64) -> (i32, bool, String) {
    if tz.ttinfos.is_empty() {
        return (0, false, "UTC".to_string());
    }
    if tz.transitions.is_empty() {
        return tz.ttinfos[0].clone();
    }
    let idx = match tz.transitions.binary_search(&instant) {
        Ok(i) => Some(i),
        Err(0) => None,
        Err(i) => Some(i - 1),
    };
    match idx {
        Some(i) => tz.ttinfos[tz.trans_types[i] as usize].clone(),
        None => tz
            .ttinfos
            .iter()
            .find(|t| !t.1)
            .cloned()
            .unwrap_or_else(|| tz.ttinfos[0].clone()),
    }
}

thread_local! {
    static TZ_CACHE: std::cell::RefCell<HashMap<String, std::rc::Rc<ParsedTz>>> = std::cell::RefCell::new(HashMap::new());
}

/// Loads and caches a real IANA time zone from the system's tzdata
/// (`/usr/share/zoneinfo`). Rejects keys that could escape the zoneinfo
/// root (defense against path traversal via a crafted zone key).
fn load_tz(key: &str) -> Option<std::rc::Rc<ParsedTz>> {
    if key.is_empty() || key.contains("..") || key.starts_with('/') || key.contains('\0') {
        return None;
    }
    TZ_CACHE.with(|c| {
        if let Some(v) = c.borrow().get(key) {
            return Some(v.clone());
        }
        let path = format!("/usr/share/zoneinfo/{}", key);
        let bytes = std::fs::read(&path).ok()?;
        let parsed = parse_tzif(&bytes)?;
        let rc = std::rc::Rc::new(parsed);
        c.borrow_mut().insert(key.to_string(), rc.clone());
        Some(rc)
    })
}

// ---- Instance/attribute helpers ----

fn inst_get(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        dict.get(name).cloned()
    } else {
        None
    }
}

fn inst_get_i64(obj: &PyObjectRef, name: &str) -> i64 {
    inst_get(obj, name).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn instance_type_name(obj: &PyObjectRef) -> String {
    if let PyObject::Instance { typ, .. } = &*obj.borrow() {
        if let PyObject::Type { name, .. } = &*typ.borrow() {
            return name.clone();
        }
    }
    String::new()
}

/// UTC offset in seconds for `tzinfo`, evaluated at the naive wall-clock
/// instant given by `ordinal`/`day_seconds` (days since 0001-01-01 and
/// seconds since local midnight). Only understands this module's own
/// `timezone` and `zoneinfo.ZoneInfo` — see module-level doc comment.
fn get_utcoffset_seconds(tzinfo: &PyObjectRef, ordinal: i64, day_seconds: i64) -> Option<i64> {
    if matches!(tzinfo, PyObjectRef::None) {
        return None;
    }
    match instance_type_name(tzinfo).as_str() {
        "timezone" => Some(inst_get_i64(tzinfo, "_offset_seconds")),
        "ZoneInfo" => {
            let key = inst_get(tzinfo, "key").map(|v| v.str()).unwrap_or_default();
            let tz = load_tz(&key)?;
            let unix_instant = (ordinal - EPOCH_ORDINAL) * 86400 + day_seconds;
            Some(tz_offset_for_instant(&tz, unix_instant).0 as i64)
        }
        _ => None,
    }
}

fn tzname_for(tzinfo: &PyObjectRef, ordinal: i64, day_seconds: i64) -> Option<String> {
    if matches!(tzinfo, PyObjectRef::None) {
        return None;
    }
    match instance_type_name(tzinfo).as_str() {
        "timezone" => {
            let name = inst_get(tzinfo, "_name");
            match name {
                Some(n) if !matches!(n, PyObjectRef::None) => Some(n.str()),
                _ => {
                    let off = inst_get_i64(tzinfo, "_offset_seconds");
                    Some(format_utc_offset_name(off))
                }
            }
        }
        "ZoneInfo" => {
            let key = inst_get(tzinfo, "key").map(|v| v.str()).unwrap_or_default();
            let tz = load_tz(&key)?;
            let unix_instant = (ordinal - EPOCH_ORDINAL) * 86400 + day_seconds;
            Some(tz_offset_for_instant(&tz, unix_instant).2)
        }
        _ => None,
    }
}

fn format_utc_offset_name(offset_seconds: i64) -> String {
    if offset_seconds == 0 {
        return "UTC".to_string();
    }
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    if m == 0 {
        format!("UTC{}{:02}", sign, h)
    } else {
        format!("UTC{}{:02}:{:02}", sign, h, m)
    }
}

fn format_offset_iso(offset_seconds: i64) -> String {
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    let s = abs % 60;
    if s == 0 {
        format!("{}{:02}:{:02}", sign, h, m)
    } else {
        format!("{}{:02}:{:02}:{:02}", sign, h, m, s)
    }
}

// ---- Constructor-argument parsing (positional args + trailing kwargs dict) ----

struct CtorArgs {
    pos: Vec<PyObjectRef>,
    kw: HashMap<String, PyObjectRef>,
}

impl CtorArgs {
    /// `args` excludes the leading `self`/instance argument.
    fn parse(args: &[PyObjectRef]) -> Self {
        let mut pos = args.to_vec();
        let mut kw = HashMap::new();
        if let Some(last) = pos.last().cloned() {
            if matches!(&*last.borrow(), PyObject::Dict(_)) {
                pos.pop();
                if let PyObject::Dict(d) = &*last.borrow() {
                    for (k, v) in d.items() {
                        kw.insert(k.str(), v);
                    }
                }
            }
        }
        CtorArgs { pos, kw }
    }

    fn get(&self, idx: usize, name: &str) -> Option<PyObjectRef> {
        self.pos
            .get(idx)
            .cloned()
            .or_else(|| self.kw.get(name).cloned())
    }

    fn get_i64(&self, idx: usize, name: &str, default: i64) -> i64 {
        self.get(idx, name)
            .and_then(|v| v.as_i64())
            .unwrap_or(default)
    }
}

thread_local! {
    static TZINFO_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static DATE_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static TIME_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static DATETIME_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static TIMEDELTA_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static TIMEZONE_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    static ZONEINFO_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

// ---- tzinfo (abstract base — just enough for isinstance/import to work) ----

fn get_tzinfo_type() -> PyObjectRef {
    let existing = TZINFO_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
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
        "utcoffset",
        bf!("utcoffset", |_args| Err(PyError::runtime_error(
            "tzinfo subclasses must override utcoffset()"
        ))),
    );
    type_dict.insert_str(
        "dst",
        bf!("dst", |_args| Err(PyError::runtime_error(
            "tzinfo subclasses must override dst()"
        ))),
    );
    type_dict.insert_str(
        "tzname",
        bf!("tzname", |_args| Err(PyError::runtime_error(
            "tzinfo subclasses must override tzname()"
        ))),
    );
    let typ = PyObjectRef::new(PyObject::Type {
        name: "tzinfo".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    TZINFO_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

// ---- timedelta ----

fn timedelta_total_us(obj: &PyObjectRef) -> i128 {
    let d = inst_get_i64(obj, "days") as i128;
    let s = inst_get_i64(obj, "seconds") as i128;
    let us = inst_get_i64(obj, "microseconds") as i128;
    d * 86_400_000_000 + s * 1_000_000 + us
}

fn make_timedelta_from_us(us: i128) -> PyObjectRef {
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

fn make_timedelta_with_type(
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

fn make_timedelta(days: i64, seconds: i64, microseconds: i64) -> PyObjectRef {
    let (days, seconds, microseconds) = normalize_timedelta(days, seconds, microseconds);
    make_timedelta_with_type(get_timedelta_type(), days, seconds, microseconds)
}

fn get_timedelta_type() -> PyObjectRef {
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

// ---- date ----

fn date_ordinal(obj: &PyObjectRef) -> i64 {
    ymd_to_ordinal(
        inst_get_i64(obj, "year"),
        inst_get_i64(obj, "month"),
        inst_get_i64(obj, "day"),
    )
}

fn make_date_from_ordinal(ord: i64) -> PyObjectRef {
    let (y, m, d) = ordinal_to_ymd(ord);
    make_date(y, m, d)
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
            // Week number
            let week_of_year = if cur_ord < week1_monday {
                // Date is before ISO week 1 → belongs to previous year's last week
                52 // simplified; actual value is 52 or 53
            } else {
                (cur_ord - week1_monday) / 7 + 1
            };
            let iso_year = if cur_ord < week1_monday {
                year - 1
            } else if week_of_year > 52 {
                year + 1
            } else {
                year
            };
            Ok(py_tuple(vec![
                py_int(iso_year),
                py_int(week_of_year),
                py_int(wday),
            ]))
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
            let s = if !args.is_empty() {
                args[0].str()
            } else {
                String::new()
            };
            let s = s.trim();
            let date_str = match s.find(|c: char| c == 'T' || c == ' ') {
                Some(idx) => &s[..idx],
                None => s,
            };
            // Try YYYY-MM-DD format
            let parts: Vec<&str> = date_str.splitn(3, '-').collect();
            if parts.len() == 3
                && parts[0].chars().all(|c| c.is_ascii_digit())
                && parts[1].chars().all(|c| c.is_ascii_digit())
                && parts[2].chars().all(|c| c.is_ascii_digit())
            {
                let y: i64 = parts[0]
                    .parse()
                    .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
                let m: i64 = parts[1]
                    .parse()
                    .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
                let d: i64 = parts[2]
                    .parse()
                    .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
                return Ok(make_date(y, m, d));
            }
            // Try YYYYMMDD format (8 digits)
            if date_str.len() == 8 && date_str.chars().all(|c| c.is_ascii_digit()) {
                let y: i64 = date_str[..4]
                    .parse()
                    .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
                let m: i64 = date_str[4..6]
                    .parse()
                    .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
                let d: i64 = date_str[6..8]
                    .parse()
                    .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
                return Ok(make_date(y, m, d));
            }
            // Try ISO week format: YYYYWww, YYYY-Www, YYYYWwwd, YYYY-Www-d
            let compact: String = date_str.chars().filter(|c| *c != '-').collect();
            if compact.len() >= 6 && compact.len() <= 8 {
                let w_pos = compact.find(|c: char| c == 'W' || c == 'w');
                if let Some(wp) = w_pos {
                    if wp == 4 && compact.len() >= 6 {
                        if let (Ok(y), Ok(wk)) =
                            (compact[..4].parse::<i64>(), compact[5..7].parse::<i64>())
                        {
                            let wd: i64 = if compact.len() >= 8 {
                                compact[7..8].parse().unwrap_or(1)
                            } else {
                                1
                            };
                            if y >= 1 && y <= 9999 && wk >= 1 && wk <= 53 && wd >= 1 && wd <= 7 {
                                let jan4_ord = ymd_to_ordinal(y, 1, 4);
                                let jan4_weekday = ((jan4_ord % 7) + 6) % 7;
                                let week1_monday = jan4_ord - jan4_weekday;
                                let target_ord = week1_monday + (wk - 1) * 7 + (wd - 1);
                                return Ok(make_date_from_ordinal(target_ord));
                            }
                        }
                    }
                }
            }
            Err(PyError::value_error("Invalid isoformat string"))
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

fn make_date(year: i64, month: i64, day: i64) -> PyObjectRef {
    let typ = get_date_type();
    let mut dict = AttrMap::new();
    dict.insert_str("year", py_int(year));
    dict.insert_str("month", py_int(month));
    dict.insert_str("day", py_int(day));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn get_date_type() -> PyObjectRef {
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

fn make_time(
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

fn get_time_type() -> PyObjectRef {
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

fn make_datetime_from_total_us(total: i128, tzinfo: PyObjectRef) -> PyObjectRef {
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

fn datetime_isoformat(obj: &PyObjectRef, sep: char) -> String {
    let y = inst_get_i64(obj, "year");
    let mo = inst_get_i64(obj, "month");
    let d = inst_get_i64(obj, "day");
    let h = inst_get_i64(obj, "hour");
    let mi = inst_get_i64(obj, "minute");
    let s = inst_get_i64(obj, "second");
    let us = inst_get_i64(obj, "microsecond");
    let mut out = format!(
        "{:04}-{:02}-{:02}{}{:02}:{:02}:{:02}",
        y, mo, d, sep, h, mi, s
    );
    if us != 0 {
        out.push_str(&format!(".{:06}", us));
    }
    let tz = datetime_tzinfo(obj);
    if let Some(off) = get_utcoffset_seconds(&tz, datetime_ordinal(obj), h * 3600 + mi * 60 + s) {
        out.push_str(&format_offset_iso(off));
    }
    out
}

fn parse_datetime_isoformat(s: &str) -> PyResult<PyObjectRef> {
    let s = s.trim();
    if s.is_empty() {
        return Err(PyError::value_error("Invalid isoformat string"));
    }
    let (date_part, rest) = match s.find(|c: char| c == 'T' || c == ' ') {
        Some(idx) => (&s[..idx], Some(&s[idx + 1..])),
        None => (s, None),
    };

    // Parse date part: YYYY-MM-DD, YYYYMMDD, or ISO week
    let (year, month, day) = parse_date_part(date_part)?;

    let rest = match rest {
        Some(r) => r,
        None => return Ok(make_datetime(year, month, day, 0, 0, 0, 0, py_none(), 0)),
    };
    let tz_is_utc = rest.ends_with(|c: char| c == 'Z' || c == 'z');
    let rest = if tz_is_utc {
        &rest[..rest.len() - 1]
    } else {
        rest
    };
    let (time_part, tz_part, tz_is_utc) = if rest.is_empty() {
        ("", None, tz_is_utc)
    } else {
        let tz_start = rest.rfind(['+', '-']);
        match tz_start {
            Some(pos) if pos > 0 => {
                let time_str = &rest[..pos];
                if time_str.ends_with(':') {
                    (
                        time_str.trim_end_matches(':'),
                        Some(&rest[pos..]),
                        tz_is_utc,
                    )
                } else {
                    (time_str, Some(&rest[pos..]), tz_is_utc)
                }
            }
            _ => (rest, None, tz_is_utc),
        }
    };

    // Parse time part (supports both HH:MM:SS and compact HHMMSS)
    let (hour, minute, second, micro) = parse_time_part(time_part)?;

    let tzinfo = if tz_is_utc {
        get_utc_singleton()
    } else {
        match tz_part {
            Some(tz_str) if !tz_str.is_empty() => {
                let sign: i64 = if tz_str.starts_with('-') { -1 } else { 1 };
                let tz_body = &tz_str[1..];
                // Validate tz_body: must be non-empty and contain only digits and colons
                if tz_body.is_empty() || !tz_body.chars().all(|c| c.is_ascii_digit() || c == ':') {
                    return Err(PyError::value_error("Invalid isoformat string"));
                }
                let th: i64;
                let tm: i64;
                let ts: i64;
                if let Some(colon_pos) = tz_body.find(':') {
                    let hours_str = &tz_body[..colon_pos];
                    let mins_str = &tz_body[colon_pos + 1..];
                    if let Some(colon2) = mins_str.find(':') {
                        th = hours_str.parse().unwrap_or(0);
                        tm = mins_str[..colon2].parse().unwrap_or(0);
                        ts = mins_str[colon2 + 1..].parse().unwrap_or(0);
                    } else {
                        th = hours_str.parse().unwrap_or(0);
                        tm = mins_str.parse().unwrap_or(0);
                        ts = 0;
                    }
                } else {
                    // Compact tz offset: +HH, +HHMM, +HHMMSS
                    match tz_body.len() {
                        2 => {
                            th = tz_body.parse().unwrap_or(0);
                            tm = 0;
                            ts = 0;
                        }
                        4 => {
                            th = tz_body[..2].parse().unwrap_or(0);
                            tm = tz_body[2..].parse().unwrap_or(0);
                            ts = 0;
                        }
                        6 => {
                            th = tz_body[..2].parse().unwrap_or(0);
                            tm = tz_body[2..4].parse().unwrap_or(0);
                            ts = tz_body[4..].parse().unwrap_or(0);
                        }
                        _ => {
                            th = tz_body.parse().unwrap_or(0);
                            tm = 0;
                            ts = 0;
                        }
                    }
                }
                let off = sign * (th * 3600 + tm * 60 + ts);
                if off == 0 {
                    get_utc_singleton()
                } else {
                    make_timezone(off, None)
                }
            }
            _ => py_none(),
        }
    };
    Ok(make_datetime(
        year, month, day, hour, minute, second, micro, tzinfo, 0,
    ))
}

fn parse_date_part(date_part: &str) -> PyResult<(i64, i64, i64)> {
    // Try YYYY-MM-DD format
    let parts: Vec<&str> = date_part.splitn(3, '-').collect();
    if parts.len() == 3
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].chars().all(|c| c.is_ascii_digit())
        && parts[2].chars().all(|c| c.is_ascii_digit())
    {
        let y: i64 = parts[0]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let m: i64 = parts[1]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let d: i64 = parts[2]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        return Ok((y, m, d));
    }
    // Try YYYYMMDD format (8 digits)
    if date_part.len() == 8 && date_part.chars().all(|c| c.is_ascii_digit()) {
        let y: i64 = date_part[..4]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let m: i64 = date_part[4..6]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let d: i64 = date_part[6..8]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        return Ok((y, m, d));
    }
    // Try ISO week format: YYYYWww, YYYY-Www, YYYYWwwd, YYYY-Www-d
    let compact: String = date_part.chars().filter(|c| *c != '-').collect();
    if compact.len() >= 6 && compact.len() <= 8 {
        let w_pos = compact.find(|c: char| c == 'W' || c == 'w');
        if let Some(wp) = w_pos {
            if wp == 4 && compact.len() >= 6 {
                if let (Ok(y), Ok(wk)) = (compact[..4].parse::<i64>(), compact[5..7].parse::<i64>())
                {
                    let wd: i64 = if compact.len() >= 8 {
                        compact[7..8].parse().unwrap_or(1)
                    } else {
                        1
                    };
                    if y >= 1 && y <= 9999 && wk >= 1 && wk <= 53 && wd >= 1 && wd <= 7 {
                        let jan4_ord = ymd_to_ordinal(y, 1, 4);
                        let jan4_weekday = ((jan4_ord % 7) + 6) % 7;
                        let week1_monday = jan4_ord - jan4_weekday;
                        let target_ord = week1_monday + (wk - 1) * 7 + (wd - 1);
                        return Ok(ordinal_to_ymd(target_ord));
                    }
                }
            }
        }
    }
    Err(PyError::value_error("Invalid isoformat string"))
}

fn parse_time_part(time_part: &str) -> PyResult<(i64, i64, i64, i64)> {
    if time_part.is_empty() {
        return Ok((0, 0, 0, 0));
    }
    // Compact time format (no colons): HH, HHMM, HHMMSS, or with fractional
    if !time_part.contains(':') {
        let (int_part, frac_part) =
            if let Some(pos) = time_part.find(|c: char| c == '.' || c == ',') {
                if pos + 1 >= time_part.len() {
                    return Err(PyError::value_error("Invalid isoformat string"));
                }
                (&time_part[..pos], Some(&time_part[pos + 1..]))
            } else {
                (time_part, None)
            };
        // Validate int_part is all digits
        if !int_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(PyError::value_error("Invalid isoformat string"));
        }
        let (h, m, s) = match int_part.len() {
            1 | 2 => (int_part.parse().unwrap_or(0), 0, 0),
            4 => (
                int_part[..2].parse().unwrap_or(0),
                int_part[2..].parse().unwrap_or(0),
                0,
            ),
            6 => (
                int_part[..2].parse().unwrap_or(0),
                int_part[2..4].parse().unwrap_or(0),
                int_part[4..].parse().unwrap_or(0),
            ),
            _ => {
                return Err(PyError::value_error("Invalid isoformat string"));
            }
        };
        let us = match frac_part {
            Some(frac) => {
                let padded = format!("{:0<6}", frac);
                padded[..6.min(padded.len())].parse().unwrap_or(0)
            }
            None => 0,
        };
        return Ok((h, m, s, us));
    }
    // Colon-separated: HH:MM, HH:MM:SS, HH:MM:SS.fff, HH:MM:SS,fff
    // Reject trailing colons
    if time_part.ends_with(':') {
        return Err(PyError::value_error("Invalid isoformat string"));
    }
    let tparts: Vec<&str> = time_part.splitn(3, ':').collect();
    // Validate each part is non-empty and digits (or has fractional)
    for (i, part) in tparts.iter().enumerate() {
        if part.is_empty() {
            return Err(PyError::value_error("Invalid isoformat string"));
        }
        if i < 2 {
            // hour and minute must be all digits
            if !part.chars().all(|c| c.is_ascii_digit()) {
                return Err(PyError::value_error("Invalid isoformat string"));
            }
        } else {
            // second part: digits possibly followed by . or , and fraction
            let clean: String = part.chars().filter(|c| c.is_ascii_digit()).collect();
            if clean.is_empty() {
                return Err(PyError::value_error("Invalid isoformat string"));
            }
        }
    }
    let hour: i64 = tparts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
    let minute: i64 = tparts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let (second, micro): (i64, i64) = match tparts.get(2) {
        Some(sec_str) => {
            let frac_pos = sec_str.find('.').or_else(|| sec_str.find(','));
            match frac_pos {
                Some(dotpos) => {
                    if dotpos + 1 >= sec_str.len() {
                        return Err(PyError::value_error("Invalid isoformat string"));
                    }
                    let sec: i64 = sec_str[..dotpos].parse().unwrap_or(0);
                    let frac = &sec_str[dotpos + 1..];
                    let padded = format!("{:0<6}", frac);
                    let us: i64 = padded[..6.min(padded.len())].parse().unwrap_or(0);
                    (sec, us)
                }
                None => (sec_str.parse().unwrap_or(0), 0),
            }
        }
        None => (0, 0),
    };
    Ok((hour, minute, second, micro))
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
            let sep = if args.len() > 1 {
                args[1].str().chars().next().unwrap_or('T')
            } else {
                'T'
            };
            Ok(py_str(&datetime_isoformat(&args[0], sep)))
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
            let day_diff = jd - jd_jan1;
            let week_of_year = day_diff / 7 + 1;
            let iso_year = if week_of_year == 0 {
                year - 1
            } else if week_of_year > 52 {
                year + 1
            } else {
                year
            };
            Ok(py_tuple(vec![py_int(iso_year), py_int(week_of_year), py_int(wday)]))
        }),
    );
    // datetime.datetime.fromisocalendar(year, week, weekday) — classmethod
    type_dict.insert_str(
        "fromisocalendar",
        bf!("fromisocalendar", |args| {
            if args.len() < 4 {
                return Err(PyError::type_error(
                    "fromisocalendar() missing required arguments: 'year', 'week', 'weekday'",
                ));
            }
            let year = args[1].as_i64().ok_or_else(|| PyError::type_error("year must be an integer"))? as i64;
            let week = args[2].as_i64().ok_or_else(|| PyError::type_error("week must be an integer"))? as i64;
            let weekday = args[3].as_i64().ok_or_else(|| PyError::type_error("weekday must be an integer"))? as i64;
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

fn make_datetime(
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

fn get_datetime_type() -> PyObjectRef {
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

// ---- timezone ----

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

    PyObjectRef::new(PyObject::Type {
        name: "timezone".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn make_timezone_with_type(
    typ: PyObjectRef,
    offset_seconds: i64,
    name: Option<String>,
) -> PyObjectRef {
    let mut dict = AttrMap::new();
    dict.insert_str("_offset_seconds", py_int(offset_seconds));
    dict.insert_str("_name", name.map(|n| py_str(&n)).unwrap_or_else(py_none));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn make_timezone(offset_seconds: i64, name: Option<String>) -> PyObjectRef {
    make_timezone_with_type(get_timezone_type(), offset_seconds, name)
}

fn get_timezone_type() -> PyObjectRef {
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

fn get_utc_singleton() -> PyObjectRef {
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

// ---- zoneinfo.ZoneInfo ----

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

fn get_zoneinfo_type() -> PyObjectRef {
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
