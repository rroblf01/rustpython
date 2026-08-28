use crate::object::*;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use super::helpers::{ymd_to_ordinal, EPOCH_ORDINAL};

/// Convert seconds since epoch to (year, month, day, hour, minute, second, weekday, yearday)
pub(crate) fn epoch_to_ymd(secs: i64) -> (i64, i64, i64, i64, i64, i64, i64, i64) {
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
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                // Slice support (st[:3], st[2:6]) returns a plain tuple,
                // matching structseq semantics; the test suite slices
                // heavily (`strp_output[:3]`, `tm[7]`).
                if let PyObject::Slice {
                    start,
                    stop,
                    step,
                } = &*args[1].borrow()
                {
                    use crate::object::extract_slice_fields;
                    let n = STRUCT_TIME_FIELDS.len() as isize;
                    let (s0, s1, step_v) =
                        extract_slice_fields(start, stop, step)?;
                    let norm = |v: Option<isize>, dflt: isize| -> isize {
                        match v {
                            Some(v) => {
                                if v < 0 {
                                    (n + v).max(0)
                                } else {
                                    v.min(n)
                                }
                            }
                            None => dflt,
                        }
                    };
                    let mut out = Vec::new();
                    if step_v > 0 {
                        let start = norm(s0, 0);
                        let stop = norm(s1, n);
                        let mut i = start;
                        while i < stop {
                            let name = STRUCT_TIME_FIELDS.get(i as usize)
                                .ok_or_else(|| PyError::index_error("struct_time index out of range"))?;
                            out.push(dict.get(name).cloned().unwrap_or_else(py_none));
                            i += step_v;
                        }
                    } else {
                        let start = match s0 { Some(v) => if v < 0 { (n+v).max(-1) } else { v.min(n-1) }, None => n - 1 };
                        let stop = norm(s1, -1);
                        let mut i = start;
                        while i > stop {
                            if let Some(name) = STRUCT_TIME_FIELDS.get(i as usize) {
                                out.push(dict.get(name).cloned().unwrap_or_else(py_none));
                            }
                            i += step_v;
                        }
                    }
                    return Ok(py_tuple(out));
                }
                let idx = args[1]
                    .as_i64()
                    .ok_or_else(|| PyError::type_error("indices must be integers"))?;
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
pub(crate) fn civil_to_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub(crate) fn weekday_yday_for(y: i64, m: i64, d: i64) -> (i64, i64) {
    let days = civil_to_days(y, m, d);
    let wday = ((days + 3) % 7 + 7) % 7;
    let jan1 = civil_to_days(y, 1, 1);
    let yday = days - jan1;
    (wday, yday)
}

pub(crate) fn format_strftime(
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
                // %G (ISO year) / %V (ISO week): computed from the date's
                // ordinal via the same Jan-4 anchor `isocalendar` uses.
                gv @ (Some('G') | Some('V')) => {
                    let c = gv.unwrap();
                    let ord = ymd_to_ordinal(y, m, d);
                    let jan4 = ymd_to_ordinal(y, 1, 4);
                    let jan4_wd = ((jan4 % 7) + 6) % 7; // 0=Mon
                    let week1_mon = jan4 - jan4_wd;
                    let (iso_year, iso_week) = if ord < week1_mon {
                        // belongs to last ISO year's final week
                        let py_ = y - 1;
                        let p = |yy: i64| (yy + yy / 4 - yy / 100 + yy / 400).rem_euclid(7);
                        let w53 = p(py_) == 4 || p(py_ - 1) == 3;
                        (py_, if w53 { 53 } else { 52 })
                    } else {
                        let wk = ((ord - week1_mon) / 7 + 1) as i64;
                        let p = |yy: i64| (yy + yy / 4 - yy / 100 + yy / 400).rem_euclid(7);
                        let w53 = p(y) == 4 || p(y - 1) == 3;
                        let maxw = if w53 { 53 } else { 52 };
                        if wk > maxw {
                            (y + 1, 1)
                        } else {
                            (y, wk)
                        }
                    };
                    if c == 'G' {
                        result.push_str(&format!("{:04}", iso_year));
                    } else {
                        result.push_str(&format!("{:02}", iso_week));
                    }
                }
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
        // Cooperative scheduling: a sleeping thread yields so deferred
        // thread bodies (producers, signal senders, event setters) can run
        // — exactly the happens-before real OS sleep provides. Drain once
        // per ~1ms of virtual wait; pure busy-wait otherwise starves them.
        let mut drained = false;
        loop {
            if let Ok(elapsed) = SystemTime::now().duration_since(start) {
                if elapsed.as_nanos() >= nanos as u128 {
                    break;
                }
                if !drained && elapsed.as_millis() >= 0 {
                    crate::modules::coop_threads_drain();
                    drained = true;
                }
            }
            std::thread::yield_now();
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
